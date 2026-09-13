//! Rhai scripting runtime for plugins.
//!
//! A `RhaiPlugin` compiles a user-supplied `.rhai` source file with a
//! heavily restricted `rhai::Engine` (tight operation / call / size
//! budgets, no print, no module loading) and exposes the **full**
//! capability-gated host surface: logging (`log_debug/info/warn/error`),
//! per-plugin storage (`storage_get/set/delete/list_keys`), UI
//! (`notify_*`, `register_panel`, `emit_panel_data`), execution
//! (`submit_order`, `cancel_order`, `get_positions`), market-data read
//! (`get_latest_candle`), and the callback-based registries
//! (`register_indicator`, `register_metric`, `register_keyword`,
//! `schedule`/`cancel_schedule`, `subscribe_event`). Every function is
//! gated by the plugin's declared capabilities via the host's
//! `*_guarded` accessors and returns a benign fallback ("" / false / ()
//! / NaN / no-op) when the capability is denied or the backend errors.
//!
//! Lifetime of a plugin script:
//! 1. `on_load` — compile source, register host functions, invoke
//!    `on_load()` if defined.
//! 2. `on_enable` — invoke `on_enable()` if defined.
//! 3. `on_disable` — invoke `on_disable()` if defined.
//! 4. `on_unload` — invoke `on_unload()` if defined (errors ignored),
//!    drop the host handle and the AST.
//!
//! Callback dispatch: the registry/scheduler/event closures need to call
//! the plugin's `FnPtr`s *later*, outside any rhai call context. They do
//! so through a shared, lazily-filled `OnceLock<Weak<Engine>>` cell — the
//! engine is wrapped in `Arc` only after all host functions are
//! registered on the uniquely-owned `&mut Engine`, then the cell is
//! populated with a `Weak` handle. Using `Weak` avoids a reference cycle
//! (the engine owns the closures, the closures would otherwise own the
//! engine) and makes every callback a no-op once the plugin is unloaded
//! and the engine `Arc` is dropped.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};
use parking_lot::Mutex;

use rhai::{Array, Dynamic, Engine, EvalAltResult, FnPtr, Map, Scope, AST};

use crate::models::Candle;
use crate::plugin::api::events::{EventBus, EventFilter, EventKind};
use crate::plugin::api::indicator_registry::SharedIndicatorRegistry;
use crate::plugin::api::{OrderRequest, OrderSide, OrderType, Position, UiPanel};
use crate::plugin::host::PluginHost;
use crate::plugin::types::{
    Capability, NotificationKind, PluginError, PluginId, PluginMeta, PluginResult, ScheduleHandle,
};
use crate::strategy::execution::paper::PaperTrade;
use crate::strategy::runtime::context::EvalContext;

use super::super::Plugin;

/// Newtype wrapper around `Candle` so we can register it with Rhai as a
/// custom type called "Candle" (Rhai types must be distinct from the
/// surrounding `Dynamic` value space).
#[derive(Clone)]
pub struct CandleWrapper(pub Candle);

impl std::fmt::Debug for CandleWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candle")
            .field("open", &self.0.open)
            .field("high", &self.0.high)
            .field("low", &self.0.low)
            .field("close", &self.0.close)
            .field("volume", &self.0.volume)
            .field("timestamp", &self.0.timestamp)
            .finish()
    }
}

/// Rhai-backed plugin. Compiles a `.rhai` source file at load time and
/// invokes the `on_load` / `on_enable` / `on_disable` / `on_unload`
/// script functions at the corresponding lifecycle events.
///
/// The engine and AST are wrapped in `Arc` so that closures registered
/// as host functions (specifically `register_indicator`) can hold
/// long-lived references to them and dispatch back into the script
/// when the indicator pipeline is evaluated. Rhai's `Engine` is not
/// `Clone`, so wrapping it is the only way to share it between the
/// plugin struct and the registered functions.
pub struct RhaiPlugin {
    meta: PluginMeta,
    capabilities: Vec<Capability>,
    source_path: PathBuf,
    host: Option<Arc<PluginHost>>,
    engine: Arc<Engine>,
    scope: Option<Scope<'static>>,
}

/// Build a freshly-hardened `rhai::Engine` with the containment budgets
/// and the `Candle` custom type registered. Used by both `RhaiPlugin::new`
/// (wrapped in `Arc`) and `on_load` (which registers host functions on the
/// uniquely-owned engine before wrapping it), so the two never drift.
fn build_hardened_engine() -> Engine {
    let mut engine = Engine::new();
    // Hard execution budgets — these are the primary containment
    // mechanism for untrusted plugin code.
    engine.set_max_operations(200_000);
    engine.set_max_call_levels(32);
    engine.set_max_string_size(65_536);
    engine.set_max_array_size(10_000);
    engine.set_max_map_size(1_000);
    // Silently swallow plugin print() calls; the engine still parses
    // them but they go nowhere.
    engine.on_print(|_| {});
    // Module loading is intentionally NOT installed — the spec says
    // to leave it disabled by default. Plugins only see what we
    // explicitly register below.

    // Register the Candle newtype so plugin scripts can address
    // candles by their public field names.
    engine.register_type_with_name::<CandleWrapper>("Candle");
    engine.register_get("open", |c: &mut CandleWrapper| c.0.open);
    engine.register_get("high", |c: &mut CandleWrapper| c.0.high);
    engine.register_get("low", |c: &mut CandleWrapper| c.0.low);
    engine.register_get("close", |c: &mut CandleWrapper| c.0.close);
    engine.register_get("volume", |c: &mut CandleWrapper| c.0.volume);
    engine.register_get("timestamp", |c: &mut CandleWrapper| c.0.timestamp);

    engine
}

impl RhaiPlugin {
    /// Build a new Rhai plugin with a hardened engine. The source file
    /// is *not* compiled yet — that happens in `on_load`, which builds a
    /// fresh engine, registers the host functions on it while it is
    /// uniquely owned, and replaces this placeholder engine.
    pub fn new(
        meta: PluginMeta,
        capabilities: Vec<Capability>,
        source_path: PathBuf,
    ) -> PluginResult<Self> {
        Ok(Self {
            meta,
            capabilities,
            source_path,
            host: None,
            engine: Arc::new(build_hardened_engine()),
            ast: None,
            scope: None,
        })
    }
}

/// Build a Rhai `Map` from a single `Candle` so plugin scripts can
/// iterate over candle arrays the same way they would over any other
/// array of record-like values.
fn candle_to_map(c: &Candle) -> Map {
    let mut m = Map::new();
    m.insert("open".into(), Dynamic::from(c.open));
    m.insert("high".into(), Dynamic::from(c.high));
    m.insert("low".into(), Dynamic::from(c.low));
    m.insert("close".into(), Dynamic::from(c.close));
    m.insert("volume".into(), Dynamic::from(c.volume));
    m.insert("timestamp".into(), Dynamic::from(c.timestamp));
    m
}

/// Convert a Rhai `Array` of numerics into `Vec<f64>`. Any non-numeric
/// entries become `f64::NAN` so the indicator pipeline keeps its
/// length and the failure is visible per-element rather than crashing
/// the whole evaluation.
fn dynamic_array_to_vec(arr: Array) -> Vec<f64> {
    arr.into_iter()
        .map(|v| {
            if v.is::<f64>() {
                v.cast::<f64>()
            } else if v.is::<i64>() {
                v.cast::<i64>() as f64
            } else {
                f64::NAN
            }
        })
        .collect()
}

/// Coerce a Rhai `Dynamic` returned from a plugin callback into `f64`.
/// `Dynamic::as_float` / `as_int` consume `self`, so we clone before each
/// attempt; anything that is neither a float nor an int becomes `NaN`.
fn dynamic_to_f64(d: &Dynamic) -> f64 {
    d.clone()
        .as_float()
        .ok()
        .or_else(|| d.clone().as_int().ok().map(|i| i as f64))
        .unwrap_or(f64::NAN)
}

/// Build a Rhai `Map` from the plugin-API `Candle` (which uses
/// `timestamp_ms` rather than the engine `Candle`'s `timestamp`). The
/// script sees the field as `timestamp` for parity with `candle_to_map`.
fn candle_api_to_map(c: &crate::plugin::api::Candle) -> Map {
    let mut m = Map::new();
    m.insert("open".into(), Dynamic::from(c.open));
    m.insert("high".into(), Dynamic::from(c.high));
    m.insert("low".into(), Dynamic::from(c.low));
    m.insert("close".into(), Dynamic::from(c.close));
    m.insert("volume".into(), Dynamic::from(c.volume));
    m.insert("timestamp".into(), Dynamic::from(c.timestamp_ms));
    m
}

/// Build a Rhai `Map` from a `PaperTrade` so analytics callbacks can
/// iterate over trade history. `side` is lowered to `"buy"` / `"sell"`
/// and `pnl` maps to a float or `()` when absent.
fn paper_trade_to_map(t: &PaperTrade) -> Map {
    let mut m = Map::new();
    m.insert("id".into(), Dynamic::from(t.id.clone()));
    m.insert("timestamp".into(), Dynamic::from(t.timestamp));
    m.insert("symbol".into(), Dynamic::from(t.symbol.clone()));
    let side = match t.side {
        crate::models::OrderSide::Buy => "buy",
        crate::models::OrderSide::Sell => "sell",
    };
    m.insert("side".into(), Dynamic::from(side.to_string()));
    m.insert("quantity".into(), Dynamic::from(t.quantity as i64));
    m.insert("price".into(), Dynamic::from(t.price));
    m.insert("ruleId".into(), Dynamic::from(t.rule_id.clone()));
    match t.pnl {
        Some(p) => m.insert("pnl".into(), Dynamic::from(p)),
        None => m.insert("pnl".into(), Dynamic::UNIT),
    };
    m
}

/// Build a Rhai `Map` from a broker `Position`.
fn position_to_map(p: &Position) -> Map {
    let mut m = Map::new();
    m.insert("symbol".into(), Dynamic::from(p.symbol.clone()));
    m.insert("quantity".into(), Dynamic::from(p.quantity));
    m.insert("averagePrice".into(), Dynamic::from(p.average_price));
    m
}

/// Build a Rhai `Map` from an `EventKind` so `subscribe_event` callbacks
/// receive a self-describing record keyed by `type`.
fn event_to_map(e: &EventKind) -> Map {
    let mut m = Map::new();
    match e {
        EventKind::CandleProcessed(c) => {
            m.insert("type".into(), Dynamic::from("candleProcessed".to_string()));
            m.insert("candle".into(), Dynamic::from(candle_to_map(c)));
        }
        EventKind::TradeExecuted(t) => {
            m.insert("type".into(), Dynamic::from("tradeExecuted".to_string()));
            m.insert("trade".into(), Dynamic::from(paper_trade_to_map(t)));
        }
        EventKind::RuleFired {
            rule_id,
            strategy_id,
        } => {
            m.insert("type".into(), Dynamic::from("ruleFired".to_string()));
            m.insert("ruleId".into(), Dynamic::from(rule_id.clone()));
            m.insert("strategyId".into(), Dynamic::from(strategy_id.clone()));
        }
        EventKind::StrategyStatusChanged {
            strategy_id,
            new_status,
        } => {
            m.insert(
                "type".into(),
                Dynamic::from("strategyStatusChanged".to_string()),
            );
            m.insert("strategyId".into(), Dynamic::from(strategy_id.clone()));
            m.insert("newStatus".into(), Dynamic::from(new_status.clone()));
        }
        EventKind::SystemShutdown => {
            m.insert("type".into(), Dynamic::from("systemShutdown".to_string()));
        }
    }
    m
}

/// Parse a script-supplied event filter string into an `EventFilter`.
/// Unknown values default to `All` so a typo subscribes broadly rather
/// than silently receiving nothing.
fn parse_event_filter(s: &str) -> EventFilter {
    match s.to_ascii_lowercase().as_str() {
        "candle" => EventFilter::CandleProcessed,
        "trade" => EventFilter::TradeExecuted,
        "rule" => EventFilter::RuleFired,
        "status" => EventFilter::StrategyStatusChanged,
        "shutdown" => EventFilter::SystemShutdown,
        _ => EventFilter::All,
    }
}

/// Invokes a Rhai callback using the execution context provided by the host.
/// Returns a `PluginResult` containing the result or a `PluginError::ApiError`.
fn invoke_rhai_callback<Args: rhai::FuncArgs>(
    host: &PluginHost,
    fn_ptr: &FnPtr,
    args: Args,
) -> PluginResult<Dynamic> {
    let engine = host
        .engine
        .get()
        .ok_or_else(|| PluginError::ApiError("plugin engine not loaded".into()))?;
    let ast = host
        .ast
        .get()
        .ok_or_else(|| PluginError::ApiError("plugin ast not loaded".into()))?;

    match fn_ptr.call(engine, ast, args) {
        Ok(d) => Ok(d),
        Err(e) => Err(PluginError::ApiError(e.to_string())),
    }
}

/// Register all host-facing functions onto the given engine. Each
/// function captures a clone of the host's `Arc`, so the engine owns
/// the references it needs and dropping the engine drops the closures.
///
/// Callback-based functions (`register_indicator` / `register_metric` /
/// `register_keyword` / `schedule` / `subscribe_event`) also capture the
/// shared `engine_cell` — a lazily-filled `OnceLock<Weak<Engine>>` — and
/// the plugin's `AST`. When the registered callback fires later, it
/// upgrades the `Weak` to reach the engine; if the plugin has been
/// unloaded (engine `Arc` dropped) the upgrade fails and the callback
/// returns its benign fallback.
fn register_host_functions(
    engine: &mut Engine,
    host: Arc<PluginHost>,
    plugin_id: PluginId,
) {
    // ---- Logging (unguarded by capability, namespaced to the plugin). ----
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("log_info", move |msg: &str| {
            host.log().info(&pid, msg);
        });
    }
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("log_warn", move |msg: &str| {
            host.log().warn(&pid, msg);
        });
    }
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("log_error", move |msg: &str| {
            host.log().error(&pid, msg);
        });
    }

    // ---- Per-plugin storage (Storage capability). ----
    //
    // The underlying `StorageApi::read` / `write` are synchronous
    // (the `async_trait` is on the trait only for forward compat), so
    // we call them directly. We still tolerate being inside a Tokio
    // runtime by going through `Handle::current().block_on` if the
    // implementation later becomes async — falling back to a direct
    // call if there is no current runtime.
    {
        let host = host.clone();
        engine.register_fn("storage_get", move |key: &str| -> Dynamic {
            let storage = match host.storage_guarded() {
                Ok(s) => s,
                Err(_) => return Dynamic::UNIT,
            };
            match storage.read(key) {
                Ok(Some(bytes)) => match String::from_utf8(bytes) {
                    Ok(s) => Dynamic::from(s),
                    Err(_) => Dynamic::UNIT,
                },
                _ => Dynamic::UNIT,
            }
        });
    }
    {
        let host = host.clone();
        engine.register_fn("storage_set", move |key: &str, val: &str| -> bool {
            let storage = match host.storage_guarded() {
                Ok(s) => s,
                Err(_) => return false,
            };
            storage.write(key, val.as_bytes()).is_ok()
        });
    }

    // ---- UI notifications (UiPanels capability). ----
    {
        let host = host.clone();
        engine.register_fn("notify_info", move |msg: &str| {
            if let Ok(ui) = host.ui_guarded() {
                let _ = ui.notify(NotificationKind::Info, msg);
            }
        });
    }
    {
        let host = host.clone();
        engine.register_fn("notify_warning", move |msg: &str| {
            if let Ok(ui) = host.ui_guarded() {
                let _ = ui.notify(NotificationKind::Warning, msg);
            }
        });
    }
    {
        let host = host.clone();
        engine.register_fn("notify_error", move |msg: &str| {
            if let Ok(ui) = host.ui_guarded() {
                let _ = ui.notify(NotificationKind::Error, msg);
            }
        });
    }

    // ---- Indicator registration (Indicators capability). ----
    //
    // The Rhai script calls `register_indicator(name, fn_ptr)`. The
    // `FnPtr` is a handle into the plugin's AST — to dispatch it, the
    // closure we hand to the registry needs to call
    // `func.call(&engine, &ast, args)`. That means the closure must
    // capture both the engine and the AST, which is why `RhaiPlugin`
    // stores them in `Arc`s.
    //
    // The trait-level `IndicatorRegistryApi` exposes a factory-based
    // `register` that loses plugin-id information. The spec's
    // `register(name, plugin_id, fn)` semantics live on the concrete
    // `SharedIndicatorRegistry::register_fn`. We downcast back to the
    // concrete type when the host is built with one; if the cast
    // fails, the registration is best-effort no-op.
    {
        let host_for_ind = host.clone();
        let cell = engine_cell.clone();
        let ast_clone = ast_arc.clone();
        let pid_clone = plugin_id.clone();
        engine.register_fn(
            "register_indicator",
            move |name: String, func: FnPtr| -> bool {
                let registry = match host_for_ind.indicators_guarded() {
                    Ok(r) => r,
                    Err(_) => return false,
                };
                // Try the spec-shaped path first (plugin-id-attributed).
                if let Some(shared) = registry.as_any().downcast_ref::<SharedIndicatorRegistry>() {
                    let cell_for_call = cell.clone();
                    let ast_for_call = ast_clone.clone();
                    let func_for_call = func.clone();
                    let indicator_fn: std::sync::Arc<
                        dyn Fn(&[Candle], usize) -> Vec<f64> + Send + Sync,
                    > = std::sync::Arc::new(move |candles: &[Candle], period: usize| -> Vec<f64> {
                        let n = candles.len();
                        let engine = match cell_for_call.get().and_then(|w| w.upgrade()) {
                            Some(e) => e,
                            None => return vec![f64::NAN; n],
                        };
                        let candles_array: Array = candles
                            .iter()
                            .map(candle_to_map)
                            .map(Dynamic::from)
                            .collect();
                        let call_result: Result<Array, Box<EvalAltResult>> = func_for_call.call(
                            &engine,
                            &ast_for_call,
                            (candles_array, period as i64),
                        );
                        match call_result {
                            Ok(arr) => dynamic_array_to_vec(arr),
                            Err(_) => vec![f64::NAN; n],
                        }
                    });
                    return shared
                        .register_fn(&name, pid_clone.clone(), indicator_fn)
                        .is_ok();
                }
                // No downcast: the trait-level register path requires a
                // plugin-id-attributed indicator function which we already
                // constructed above. The trait-level register method now
                // matches the spec shape and the factory-based fallback is
                // intentionally removed.
                false
            },
        );
    }

    // ---- Extra logging level (unguarded). ----
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("log_debug", move |msg: &str| {
            host.log().debug(&pid, msg);
        });
    }

    // ---- Storage: delete + list_keys (Storage capability). ----
    {
        let host = host.clone();
        engine.register_fn("storage_delete", move |key: &str| -> bool {
            let storage = match host.storage_guarded() {
                Ok(s) => s,
                Err(_) => return false,
            };
            storage.delete(key).is_ok()
        });
    }
    {
        let host = host.clone();
        engine.register_fn("storage_list_keys", move |prefix: &str| -> Array {
            let storage = match host.storage_guarded() {
                Ok(s) => s,
                Err(_) => return Array::new(),
            };
            match storage.list_keys(prefix) {
                Ok(keys) => keys.into_iter().map(Dynamic::from).collect(),
                Err(_) => Array::new(),
            }
        });
    }

    // ---- UI panels (UiPanels capability). ----
    {
        let host = host.clone();
        engine.register_fn(
            "register_panel",
            move |id: &str, title: &str, route: &str| -> bool {
                let ui = match host.ui_guarded() {
                    Ok(u) => u,
                    Err(_) => return false,
                };
                ui.register_panel(UiPanel {
                    id: id.to_string(),
                    title: title.to_string(),
                    route: route.to_string(),
                })
                .is_ok()
            },
        );
    }
    // Panel data emit downcasts to the Tauri-backed impl so the broadcast
    // channel picks the value up — same path the WASM runtime uses.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn(
            "emit_panel_data",
            move |panel_id: &str, json_string: &str| -> bool {
                let ui = match host.ui_guarded() {
                    Ok(u) => u,
                    Err(_) => return false,
                };
                let value: serde_json::Value = match serde_json::from_str(json_string) {
                    Ok(v) => v,
                    Err(e) => {
                        host.log()
                            .error(&pid, &format!("emit_panel_data: invalid json: {e}"));
                        return false;
                    }
                };
                if let Some(tauri_ui) = ui
                    .as_any()
                    .downcast_ref::<crate::plugin::api::ui::TauriUiApi>()
                {
                    tauri_ui.emit_panel_data(panel_id.to_string(), value).is_ok()
                } else {
                    host.log()
                        .error(&pid, "emit_panel_data: host ui is not a TauriUiApi");
                    false
                }
            },
        );
    }

    // ---- Execution (Execution capability). ----
    //
    // `submit_order` / `cancel_order` are async on the trait; we bridge
    // them synchronously via `block_in_place + Handle::block_on`, which
    // requires a multi-thread runtime (Tauri default; tests use
    // `#[tokio::test(flavor = "multi_thread")]`). `positions()` is sync
    // and self-manages any blocking internally, so it is called directly.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn(
            "submit_order",
            move |symbol: &str, side: &str, qty: i64, order_type: &str, price: f64| -> Result<String, Box<EvalAltResult>> {
                let execution = match host.execution_guarded() {
                    Ok(e) => e.clone(),
                    Err(e) => return Err(Box::new(EvalAltResult::ErrorRuntime(
                        format!("execution capability denied: {e}").into(),
                        rhai::Position::NONE,
                    ))),
                };
                if qty <= 0 {
                    return Ok(String::new());
                }
                let side = match side.to_ascii_lowercase().as_str() {
                    "buy" => OrderSide::Buy,
                    "sell" => OrderSide::Sell,
                    other => {
                        host.log()
                            .error(&pid, &format!("submit_order: invalid side '{other}'"));
                        return Ok(String::new());
                    }
                };
                let (order_type, price) = match order_type.to_ascii_lowercase().as_str() {
                    "market" => (OrderType::Market, None),
                    "limit" => (OrderType::Limit, Some(price)),
                    other => {
                        host.log()
                            .error(&pid, &format!("submit_order: invalid order type '{other}'"));
                        return Ok(String::new());
                    }
                };
                let request = OrderRequest {
                    symbol: symbol.to_string(),
                    side,
                    quantity: qty as u32,
                    order_type,
                    price,
                };
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(execution.submit_order(request))
                });
                match result {
                    Ok(id) => Ok(id),
                    Err(e) => {
                        host.log().error(&pid, &format!("submit_order: {e}"));
                        Err(Box::new(EvalAltResult::ErrorRuntime(
                            format!("submit_order failed: {e}").into(),
                            rhai::Position::NONE,
                        )))
                    }
                }
            },
        );
    }
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("cancel_order", move |order_id: &str| -> Result<bool, Box<EvalAltResult>> {
            let execution = match host.execution_guarded() {
                Ok(e) => e.clone(),
                Err(e) => return Err(Box::new(EvalAltResult::ErrorRuntime(
                    format!("execution capability denied: {e}").into(),
                    rhai::Position::NONE,
                ))),
            };
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(execution.cancel_order(order_id))
            });
            match result {
                Ok(()) => Ok(true),
                Err(e) => {
                    host.log().error(&pid, &format!("cancel_order: {e}"));
                    Err(Box::new(EvalAltResult::ErrorRuntime(
                        format!("cancel_order failed: {e}").into(),
                        rhai::Position::NONE,
                    )))
                }
            }
        });
    }
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("get_positions", move || -> Array {
            let execution = match host.execution_guarded() {
                Ok(e) => e,
                Err(_) => return Array::new(),
            };
            match execution.positions() {
                Ok(positions) => positions
                    .iter()
                    .map(position_to_map)
                    .map(Dynamic::from)
                    .collect(),
                Err(e) => {
                    host.log().error(&pid, &format!("get_positions: {e}"));
                    Array::new()
                }
            }
        });
    }

    // ---- Market-data read (MarketData capability). ----
    //
    // `latest_candle` is sync but its impl calls a bare `block_on`
    // internally, so we wrap the call in `block_in_place` to avoid
    // nesting a runtime inside the current worker.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("get_latest_candle", move |symbol: &str| -> Dynamic {
            let md = match host.market_data_guarded() {
                Ok(m) => m.clone(),
                Err(_) => return Dynamic::UNIT,
            };
            let result = tokio::task::block_in_place(|| md.latest_candle(symbol));
            match result {
                Ok(candle) => Dynamic::from(candle_api_to_map(&candle)),
                Err(e) => {
                    host.log().error(&pid, &format!("get_latest_candle: {e}"));
                    Dynamic::UNIT
                }
            }
        });
    }

    // ---- Analytics metric registration (Analytics capability). ----
    //
    // The script's `FnPtr` is invoked later as `fn(trades_array) -> number`
    // whenever the metric is evaluated.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("register_metric", move |name: String, func: FnPtr| -> bool {
            let analytics = match host.analytics_guarded() {
                Ok(a) => a,
                Err(_) => return false,
            };
            if func.is_curried() {
                host.log().error(&pid, "register_metric: callbacks must be plain named functions, not closures");
                return false;
            }
            let host_for_call = host.clone();
            let func_for_call = func.clone();
            let pid_for_call = pid.clone();
            let metric_fn: Arc<dyn Fn(&[PaperTrade]) -> f64 + Send + Sync> =
                Arc::new(move |trades: &[PaperTrade]| -> f64 {
                    let trades_array: Array = trades
                        .iter()
                        .map(paper_trade_to_map)
                        .map(Dynamic::from)
                        .collect();
                    match invoke_rhai_callback(&host_for_call, &func_for_call, (trades_array,)) {
                        Ok(d) => dynamic_to_f64(&d),
                        Err(e) => {
                            host_for_call.log().error(&pid_for_call, &format!("metric callback failed: {e}"));
                            f64::NAN
                        }
                    }
                });

            host.callbacks.metrics.lock().insert(name.clone(), func.clone());
            analytics.register_metric(name, pid.clone(), metric_fn).is_ok()
        });
    }

    // ---- DSL keyword registration (DslExtension capability). ----
    //
    // The script's `FnPtr` is invoked later as
    // `fn(candles_array, current_map) -> bool` from the strategy
    // evaluator.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("register_keyword", move |keyword: String, func: FnPtr| -> bool {
            let dsl = match host.dsl_guarded() {
                Ok(d) => d,
                Err(_) => return false,
            };
            if func.is_curried() {
                host.log().error(&pid, "register_keyword: callbacks must be plain named functions, not closures");
                return false;
            }
            let host_for_call = host.clone();
            let func_for_call = func.clone();
            let pid_for_call = pid.clone();
            let handler: Arc<dyn Fn(&EvalContext) -> bool + Send + Sync> =
                Arc::new(move |ctx: &EvalContext| -> bool {
                    let candles_array: Array = ctx
                        .candles
                        .iter()
                        .map(candle_to_map)
                        .map(Dynamic::from)
                        .collect();
                    let current_map = candle_to_map(ctx.current);
                    match invoke_rhai_callback(&host_for_call, &func_for_call, (candles_array, current_map)) {
                        Ok(d) => d.as_bool().unwrap_or(false),
                        Err(e) => {
                            host_for_call.log().error(&pid_for_call, &format!("keyword callback failed: {e}"));
                            false
                        }
                    }
                });
            host.callbacks.keywords.lock().insert(keyword.clone(), func.clone());
            dsl.register_keyword(keyword, pid.clone(), handler).is_ok()
        });
    }

    // ---- Cron scheduling (Scheduler capability). ----
    //
    // The task closure is spawned on a tokio task by the scheduler and
    // calls the plugin `FnPtr` as `fn()`.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("schedule", move |cron_expr: &str, func: FnPtr| -> String {
            let scheduler = match host.scheduler_guarded() {
                Ok(s) => s.clone(),
                Err(_) => return String::new(),
            };
            if func.is_curried() {
                host.log().error(&pid, "schedule: callbacks must be plain named functions, not closures");
                return String::new();
            }
            let host_for_call = host.clone();
            let func_for_call = func.clone();
            let pid_for_call = pid.clone();
            let scheduler_for_call = scheduler.clone();
            let failures = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let failures_for_call = failures.clone();
            let handle_cell = Arc::new(Mutex::new(None));
            let handle_cell_for_call = handle_cell.clone();
            let task: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let res = tokio::task::block_in_place(|| {
                    invoke_rhai_callback(&host_for_call, &func_for_call, ())
                });
                match res {
                    Ok(_) => {
                        failures_for_call.store(0, std::sync::atomic::Ordering::SeqCst);
                    }
                    Err(e) => {
                        let count = failures_for_call.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        host_for_call.log().error(&pid_for_call, &format!("scheduled task failed: {e}"));
                        if count >= 3 {
                            host_for_call.log().error(&pid_for_call, "scheduled task disabled after 3 consecutive failures");
                            if let Some(handle) = handle_cell_for_call.lock().clone() {
                                let _ = scheduler_for_call.cancel(handle);
                            }
                        }
                    }
                }
            });
            match scheduler.schedule(pid.clone(), cron_expr, task) {
                Ok(handle) => {
                    *handle_cell.lock() = Some(handle.clone());
                    host.callbacks.schedules.lock().insert(handle.clone(), func.clone());
                    handle.to_string()
                }
                Err(e) => {
                    host.log().error(&pid, &format!("schedule: {e}"));
                    String::new()
                }
            }
        });
    }
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("cancel_schedule", move |handle_string: &str| -> bool {
            let scheduler = match host.scheduler_guarded() {
                Ok(s) => s,
                Err(_) => return false,
            };
            let uuid = match uuid::Uuid::parse_str(handle_string) {
                Ok(u) => u,
                Err(_) => return false,
            };
            scheduler.cancel(ScheduleHandle(uuid)).is_ok()
        });
    }

    // ---- Event subscription (Events capability). ----
    //
    // The callback is invoked from a tokio task by the event bus and
    // calls the plugin `FnPtr` as `fn(event_map)`.
    {
        let host = host.clone();
        let pid = plugin_id.clone();
        engine.register_fn("subscribe_event", move |filter: &str, func: FnPtr| -> String {
            let bus: &Arc<EventBus> = match host.event_bus_guarded() {
                Ok(b) => b,
                Err(_) => return String::new(),
            };
            if func.is_curried() {
                host.log().error(&pid, "subscribe_event: callbacks must be plain named functions, not closures");
                return String::new();
            }
            let host_for_call = host.clone();
            let func_for_call = func.clone();
            let pid_for_call = pid.clone();
            let callback: Arc<dyn Fn(EventKind) + Send + Sync> = Arc::new(move |event: EventKind| {
                let event_map = event_to_map(&event);
                match invoke_rhai_callback(&host_for_call, &func_for_call, (event_map,)) {
                    Ok(_) => (),
                    Err(e) => {
                        host_for_call.log().error(&pid_for_call, &format!("event callback failed: {e}"));
                    }
                }
            });
            let handle = bus.subscribe(pid.clone(), parse_event_filter(filter), callback);
            host.callbacks.events.lock().insert(
                parse_event_filter(filter),
                {
                    let mut vec = Vec::new();
                    vec.push(func.clone());
                    vec
                },
            );
            handle.to_string()
        });
    }
}

struct NoopIndicator;
impl crate::plugin::api::IndicatorInstance for NoopIndicator {
    fn update(&mut self, _value: f64) {}
    fn value(&self) -> Option<f64> {
        None
    }
    fn name(&self) -> &str {
        "no-op"
    }
}

#[async_trait::async_trait]
impl Plugin for RhaiPlugin {
    fn meta(&self) -> &PluginMeta {
        &self.meta
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn on_load(&mut self, host: Arc<PluginHost>) -> PluginResult<()> {
        // Stash the host so lifecycle hooks can use it.
        self.host = Some(host.clone());

        // Build a FRESH, uniquely-owned engine so host functions can be
        // registered on `&mut Engine` before it is shared.
        let mut engine = build_hardened_engine();

        // Compile the user source with the fresh engine.
        let ast = engine
            .compile_file(self.source_path.clone())
            .map_err(|e| PluginError::LoadFailed(format!("compile_file failed: {e}")))?;

        // Populate the host's execution context.
        host.ast.set(Arc::new(ast)).expect("ast should be empty");

        register_host_functions(
            &mut engine,
            host.clone(),
            self.meta.id.clone(),
        );

        // Wrap the now fully-configured engine in an `Arc` and store it in the host.
        let engine_arc = Arc::new(engine);
        host.engine.set(engine_arc.clone()).expect("engine should be empty");
        self.engine = engine_arc;

        // Initialize the call scope.
        self.scope = Some(Scope::new());

        // Call the script's `on_load` if present.
        let ast_ref = host.ast.get().expect("just set");
        let scope = self.scope.as_mut().expect("just set");
        let result: Result<Dynamic, Box<EvalAltResult>> =
            self.engine.call_fn(scope, ast_ref, "on_load", ());
        match result {
            Ok(_) => Ok(()),
            Err(e) if matches!(*e, EvalAltResult::ErrorFunctionNotFound(_, _)) => Ok(()),
            Err(e) => Err(PluginError::LoadFailed(format!("on_load failed: {e}"))),
        }
    }

    async fn on_enable(&mut self) -> PluginResult<()> {
        let ast = self
            .ast
            .as_ref()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        let scope = self
            .scope
            .as_mut()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        let result: Result<Dynamic, Box<EvalAltResult>> =
            self.engine.call_fn(scope, ast, "on_enable", ());
        match result {
            Ok(_) => Ok(()),
            Err(e) if matches!(*e, EvalAltResult::ErrorFunctionNotFound(_, _)) => Ok(()),
            Err(e) => Err(PluginError::ApiError(format!("on_enable failed: {e}"))),
        }
    }

    async fn on_disable(&mut self) -> PluginResult<()> {
        let ast = self
            .ast
            .as_ref()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        let scope = self
            .scope
            .as_mut()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        let result: Result<Dynamic, Box<EvalAltResult>> =
            self.engine.call_fn(scope, ast, "on_disable", ());
        match result {
            Ok(_) => Ok(()),
            Err(e) if matches!(*e, EvalAltResult::ErrorFunctionNotFound(_, _)) => Ok(()),
            Err(e) => Err(PluginError::ApiError(format!("on_disable failed: {e}"))),
        }
    }

    fn on_unload(&mut self) {
        if let (Some(ast), Some(scope)) = (self.ast.as_ref(), self.scope.as_mut()) {
            // Errors from on_unload are intentionally ignored per spec.
            let _: Result<(), Box<EvalAltResult>> =
                self.engine.call_fn(scope, ast, "on_unload", ());
        }
        self.host = None;
        self.ast = None;
        self.scope = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the `Arc::get_mut`-on-a-cloned-`Arc` bug that
    /// made *every* Rhai plugin fail to load with "engine Arc unexpectedly
    /// shared". This drives a tiny `.rhai` plugin through the full
    /// `on_load` → `on_enable` → `on_unload` lifecycle against a real
    /// `PluginHost` and asserts that (1) no lifecycle call errors and
    /// (2) a `storage_set` inside `on_load` round-trips through the host.
    ///
    /// Mirrors the WASM end-to-end smoke test's host construction.
    #[tokio::test(flavor = "multi_thread")]
    async fn rhai_plugin_loads_and_drives_host_fns_end_to_end() {
        use std::path::PathBuf;

        use crate::plugin::api::analytics::SharedAnalyticsRegistry;
        use crate::plugin::api::dsl_extension::SharedDslExtensionRegistry;
        use crate::plugin::api::events::EventBus;
        use crate::plugin::api::indicator_registry::SharedIndicatorRegistry;
        use crate::plugin::api::log_file::{RateLimitedFileLog, RollingLog};
        use crate::plugin::api::scheduler::CronScheduler;
        use crate::plugin::api::storage::PluginKvStore;
        use crate::plugin::api::ui::TauriUiApi;
        use crate::plugin::api::StorageApi;
        use crate::plugin::host::PluginHostBuilder;
        use crate::plugin::manifest::PluginPermissions;
        use crate::plugin::types::{PluginId, PluginMeta, PluginVersion};

        // Tiny Rhai plugin: logs on load and writes a storage key.
        let source = r#"
            fn on_load() {
                log_info("loaded");
                storage_set("k", "v");
            }
        "#;

        let dir = tempfile::tempdir().expect("tempdir");
        let source_path: PathBuf = dir.path().join("plugin.rhai");
        let log_path: PathBuf = dir.path().join("plugin.log");
        let storage_dir: PathBuf = dir.path().join("storage");
        std::fs::create_dir_all(&storage_dir).expect("mkdir storage");
        std::fs::write(&source_path, source.as_bytes()).expect("write .rhai");

        let plugin_id = PluginId::from("rhai-smoke");
        let meta = PluginMeta {
            id: plugin_id.clone(),
            name: "rhai-smoke".into(),
            version: PluginVersion {
                major: 0,
                minor: 1,
                patch: 0,
            },
            description: "Rhai lifecycle smoke fixture".into(),
            author: "tests".into(),
        };
        let mut plugin =
            RhaiPlugin::new(meta, vec![Capability::Storage], source_path).expect("RhaiPlugin::new");

        // Host-side services the plugin never touches get minimal stubs so
        // the test does not pull in a broker.
        struct NoopMarketData;
        #[async_trait::async_trait]
        impl crate::plugin::api::MarketDataApi for NoopMarketData {
            fn subscribe_ticks(
                &self,
                _symbol: &str,
                _callback: std::sync::Arc<
                    dyn Fn(crate::plugin::api::MarketDataEvent) + Send + Sync,
                >,
            ) -> crate::plugin::PluginResult<crate::plugin::types::SubscriptionHandle> {
                Err(crate::plugin::types::PluginError::ApiError("noop".into()))
            }
            fn unsubscribe_ticks(
                &self,
                _handle: crate::plugin::types::SubscriptionHandle,
            ) -> crate::plugin::PluginResult<()> {
                Ok(())
            }
            fn latest_candle(
                &self,
                _symbol: &str,
            ) -> crate::plugin::PluginResult<crate::plugin::api::Candle> {
                Err(crate::plugin::types::PluginError::ApiError("noop".into()))
            }
        }

        let log = std::sync::Arc::new(RateLimitedFileLog::with_log(
            plugin_id.clone(),
            RollingLog::open(log_path.clone()).expect("open log file"),
        ));
        let storage = std::sync::Arc::new(
            PluginKvStore::new(plugin_id.clone(), storage_dir.clone()).expect("kv store"),
        );
        let (tauri_ui_api, _rx) = TauriUiApi::new();
        let host = PluginHostBuilder {
            id: plugin_id.clone(),
            market_data: std::sync::Arc::new(NoopMarketData),
            execution: std::sync::Arc::new(crate::plugin::api::execution::NoopExecutionApi),
            storage,
            event_bus: EventBus::new(),
            indicators: std::sync::Arc::new(SharedIndicatorRegistry::new()),
            analytics: std::sync::Arc::new(SharedAnalyticsRegistry::new()),
            dsl: std::sync::Arc::new(SharedDslExtensionRegistry::new()),
            ui: tauri_ui_api,
            scheduler: CronScheduler::new(),
            log,
            capabilities: vec![Capability::Storage],
            permissions: PluginPermissions {
                network: false,
                file_system: false,
                max_memory_mb: 8,
                allowed_symbols: Vec::new(),
            },
        }
        .build();

        // The core regression: on_load must NOT error (the old
        // Arc::get_mut path always did).
        plugin.on_load(host.clone()).await.expect("on_load");
        plugin.on_enable().await.expect("on_enable");
        plugin.on_unload();

        // The storage write inside on_load should have landed.
        let re_open =
            PluginKvStore::new(plugin_id.clone(), storage_dir.clone()).expect("re-open kv");
        assert_eq!(
            re_open.read("k").expect("read k"),
            Some(b"v".to_vec()),
            "storage_set through the Rhai host fn should leave 'k' -> 'v'"
        );
    }
}
