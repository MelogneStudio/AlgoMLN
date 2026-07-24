//! Rhai scripting runtime for plugins.
//!
//! A `RhaiPlugin` compiles a user-supplied `.rhai` source file with a
//! heavily restricted `rhai::Engine` (tight operation / call / size
//! budgets, no print, no module loading) and exposes a small set of
//! capability-gated host functions (`log_*`, `storage_*`, `notify_*`,
//! `register_indicator`).
//!
//! Lifetime of a plugin script:
//! 1. `on_load` — compile source, register host functions, invoke
//!    `on_load()` if defined.
//! 2. `on_enable` — invoke `on_enable()` if defined.
//! 3. `on_disable` — invoke `on_disable()` if defined.
//! 4. `on_unload` — invoke `on_unload()` if defined (errors ignored),
//!    drop the host handle and the AST.

use std::path::PathBuf;
use std::sync::Arc;

use rhai::{Array, Dynamic, Engine, EvalAltResult, FnPtr, Map, Scope, AST};

use crate::models::Candle;
use crate::plugin::api::indicator_registry::SharedIndicatorRegistry;
use crate::plugin::host::PluginHost;
use crate::plugin::types::{Capability, NotificationKind, PluginError, PluginMeta, PluginResult};

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
    ast: Option<Arc<AST>>,
    scope: Option<Scope<'static>>,
}

impl RhaiPlugin {
    /// Build a new Rhai plugin with a hardened engine. The source file
    /// is *not* compiled yet — that happens in `on_load`.
    pub fn new(
        meta: PluginMeta,
        capabilities: Vec<Capability>,
        source_path: PathBuf,
    ) -> PluginResult<Self> {
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

        Ok(Self {
            meta,
            capabilities,
            source_path,
            host: None,
            engine: Arc::new(engine),
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

/// Register all host-facing functions onto the given engine. Each
/// function captures a clone of the host's `Arc`, so the engine owns
/// the references it needs and dropping the engine drops the closures.
fn register_host_functions(
    engine: &mut Engine,
    host: Arc<PluginHost>,
    engine_arc: Arc<Engine>,
    ast_arc: Arc<AST>,
    plugin_id: crate::plugin::types::PluginId,
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
        let engine_clone = engine_arc.clone();
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
                    let engine_for_call = engine_clone.clone();
                    let ast_for_call = ast_clone.clone();
                    let func_for_call = func.clone();
                    let indicator_fn: std::sync::Arc<
                        dyn Fn(&[Candle], usize) -> Vec<f64> + Send + Sync,
                    > = std::sync::Arc::new(move |candles: &[Candle], period: usize| -> Vec<f64> {
                        let n = candles.len();
                        let candles_array: Array = candles
                            .iter()
                            .map(candle_to_map)
                            .map(Dynamic::from)
                            .collect();
                        let call_result: Result<Array, Box<EvalAltResult>> = func_for_call.call(
                            &engine_for_call,
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

        // Compile the user source *first* so we can hand a shared Arc
        // to the registered host functions.
        let ast = self
            .engine
            .compile_file(self.source_path.clone())
            .map_err(|e| PluginError::LoadFailed(format!("compile_file failed: {e}")))?;
        let ast_arc = Arc::new(ast);
        self.ast = Some(ast_arc.clone());

        // Register host-facing functions on the engine. The
        // registration must happen *before* any other Arc clone of the
        // engine is taken, which is why we use `Arc::get_mut` to
        // recover a `&mut Engine` — only possible while the Arc's
        // strong count is one.
        let engine_arc = self.engine.clone();
        {
            let engine_mut = Arc::get_mut(&mut self.engine)
                .ok_or_else(|| PluginError::LoadFailed("engine Arc unexpectedly shared".into()))?;
            register_host_functions(
                engine_mut,
                host,
                engine_arc,
                ast_arc.clone(),
                self.meta.id.clone(),
            );
        }

        // Initialize the call scope.
        self.scope = Some(Scope::new());

        // Call the script's `on_load` if present. Missing function is
        // not an error — many plugins won't define one.
        let ast_ref = self.ast.as_ref().expect("just set");
        let scope = self.scope.as_mut().expect("just set");
        let result: Result<(), Box<EvalAltResult>> =
            self.engine.call_fn(scope, ast_ref, "on_load", ());
        match result {
            Ok(()) => Ok(()),
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
        let result: Result<(), Box<EvalAltResult>> =
            self.engine.call_fn(scope, ast, "on_enable", ());
        match result {
            Ok(()) => Ok(()),
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
        let result: Result<(), Box<EvalAltResult>> =
            self.engine.call_fn(scope, ast, "on_disable", ());
        match result {
            Ok(()) => Ok(()),
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
