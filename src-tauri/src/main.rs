use std::{env, fs, path::Path, sync::Arc};

use algomln::{
    broker::{
        symbol_map::{refresh_symbol_map, SymbolMap},
        Timeframe,
    },
    commands::{
        self,
        registry::{DeployedStrategy, StrategyMode, StrategyRegistry, StrategyStatus},
        state::AppState,
        strategy::{run_backtest_dsl, BacktestResultWire},
    },
    indices::{refresh_all_if_stale, IndexRegistry},
    models::{Candle, Quote},
    plugin::{
        api::{
            analytics::SharedAnalyticsRegistry,
            dsl_extension::SharedDslExtensionRegistry,
            events::EventBus,
                execution::NoopExecutionApi,
            indicator_registry::SharedIndicatorRegistry,
            log_file::RateLimitedFileLog,
            market_data::BrokerMarketDataApi,
            scheduler::CronScheduler,
            storage::PluginKvStore,
            ui::TauriUiApi,
        },
        registry::PluginRegistry,
    },
};
use tauri::{Emitter, State};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeployResult {
    strategy_id: String,
}

#[tauri::command]
async fn get_ohlcv(
    state: State<'_, AppState>,
    symbol: String,
    timeframe: String,
    from: i64,
    to: i64,
) -> Result<Vec<Candle>, String> {
    let timeframe = timeframe.parse::<Timeframe>()?;
    commands::data::get_ohlcv(&state.data, symbol, timeframe, from, to).await
}

#[tauri::command]
async fn get_quote(state: State<'_, AppState>, symbol: String) -> Result<Quote, String> {
    commands::data::get_quote(&state.data, symbol).await
}

#[tauri::command]
async fn subscribe_ticks(state: State<'_, AppState>, symbols: Vec<String>) -> Result<(), String> {
    commands::data::subscribe_ticks(&state.data, symbols).await
}

#[tauri::command]
async fn run_backtest(
    state: State<'_, AppState>,
    dsl_source: String,
    symbol: String,
    initial_cash: f64,
) -> Result<BacktestResultWire, String> {
    run_backtest_dsl(&dsl_source, &symbol, initial_cash, &state.data).await
}

#[tauri::command]
async fn validate_dsl(dsl_source: String) -> Result<Vec<String>, String> {
    Ok(commands::strategy::validate_dsl(&dsl_source))
}

#[tauri::command]
async fn deploy_strategy(
    state: State<'_, AppState>,
    dsl_source: String,
    name: String,
    mode: String,
) -> Result<DeployResult, String> {
    let mode = StrategyMode::parse(&mode)?;
    let strategy_id = state.strategies.deploy(&name, &dsl_source, mode).await?;
    Ok(DeployResult { strategy_id })
}

#[tauri::command]
async fn list_strategies(
    state: State<'_, AppState>,
) -> Result<Vec<DeployedStrategy>, String> {
    state.strategies.list().await
}

#[tauri::command]
async fn set_strategy_status(
    state: State<'_, AppState>,
    strategy_id: String,
    status: String,
) -> Result<(), String> {
    let status = StrategyStatus::parse(&status)?;
    state.strategies.set_status(&strategy_id, status).await
}

// ---------- Plugin IPC ----------
//
// The `#[tauri::command]` attribute generates module-private macro
// artifacts (`__cmd__name`, `__tauri_command_name_name`) that
// `tauri::generate_handler!` looks up by name. Those artifacts only
// exist in the module where the function is annotated, so the plugin
// command wrappers live here in `main.rs` and delegate to the
// plain-async implementations in `commands::plugins`.

#[tauri::command]
async fn list_plugins(
    state: State<'_, AppState>,
) -> Result<Vec<algomln::plugin::PluginListEntry>, String> {
    commands::plugins::list_plugins(&state).await
}

#[tauri::command]
async fn enable_plugin(state: State<'_, AppState>, id: String) -> Result<(), String> {
    commands::plugins::enable_plugin(&state, id).await
}

#[tauri::command]
async fn disable_plugin(state: State<'_, AppState>, id: String) -> Result<(), String> {
    commands::plugins::disable_plugin(&state, id).await
}

#[tauri::command]
async fn reload_plugins(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    commands::plugins::reload_plugins(&state).await
}

// ---------- Index IPC ----------
#[tauri::command]
fn list_indices(state: State<'_, AppState>) -> Vec<algomln::indices::IndexInfo> {
    commands::indices::list_indices(&state)
}

#[tauri::command]
fn get_index_symbols(
    state: State<'_, AppState>,
    alias: String,
) -> Result<Vec<String>, String> {
    commands::indices::get_index_symbols(&state, alias)
}

#[tauri::command]
async fn refresh_indices(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<commands::indices::RefreshResult, String> {
    Ok(commands::indices::refresh_indices(&app, &state).await)
}

// ---------- Search IPC ----------
//
// Fuzzy-search the symbol universe (equities + 22 NSE indices). The
// scoring logic is pure and lives in `algomln::search`; this wrapper just
// forwards to the async body in `commands::search`.
#[tauri::command]
async fn search_symbols(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<algomln::search::SymbolMatch>, String> {
    commands::search::search_symbols_impl(&state, query).await
}

fn main() {
    load_dotenv();

    tauri::Builder::default()
        .setup(move |app| {
            use tauri::Manager;

            let store_dir = app
                .path()
                .app_data_dir()
                .expect("could not resolve app data dir");

            // ---------- Symbol map (NSE -> Dhan SECURITY_ID) ----------
            //
            // Prefer the user cache (`<app_data>/sec_id_cache.csv`); fall back
            // to the bundled seed (`sample-data/sec_id.csv` in the repo root,
            // `src-tauri/resources/sample-data/sec_id.csv` in the bundled
            // resource dir). If neither resolves, fall back to an empty map
            // so the app still boots.
            let sym_cache_path = store_dir.join("sec_id_cache.csv");
            let sym_seed_path = std::path::PathBuf::from("sample-data/sec_id.csv");
            let sym_resource_seed = app
                .path()
                .resource_dir()
                .ok()
                .map(|d| d.join("resources").join("sample-data").join("sec_id.csv"));

            let symbol_map = if sym_cache_path.exists() {
                SymbolMap::load(&sym_cache_path).unwrap_or_else(|e| {
                    eprintln!("[symbol_map] cache load failed ({e}); falling back to seed");
                    SymbolMap::load(&sym_seed_path)
                        .or_else(|_| {
                            sym_resource_seed
                                .as_ref()
                                .and_then(|p| SymbolMap::load(p).ok())
                                .ok_or_else(|| {
                                    "seed sec_id.csv missing - add sample-data/sec_id.csv"
                                        .to_string()
                                })
                        })
                        .unwrap_or_else(|e| {
                            eprintln!("[symbol_map] {e} - using empty map");
                            SymbolMap::empty()
                        })
                })
            } else {
                SymbolMap::load(&sym_seed_path)
                    .or_else(|_| {
                        sym_resource_seed
                            .as_ref()
                            .and_then(|p| SymbolMap::load(p).ok())
                            .ok_or_else(|| {
                                "seed sec_id.csv missing - add sample-data/sec_id.csv".to_string()
                            })
                    })
                    .unwrap_or_else(|e| {
                        eprintln!("[symbol_map] {e} - using empty map");
                        SymbolMap::empty()
                    })
            };
            let symbol_map = Arc::new(parking_lot::RwLock::new(symbol_map));

            let data =
                commands::data::DataState::dhan_from_env_with_symbol_map(symbol_map.clone())
                    .expect("Set DHAN_ACCESS_TOKEN in .env before starting the Tauri app");
            let store_path = store_dir.join("strategies.json");
            let registry = StrategyRegistry::open(store_path.clone())
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to open strategy registry at {}: {error}",
                        store_path.display()
                    )
                });
            let strategies = Arc::new(registry);

            // ---------- Plugin shared infrastructure ----------
            //
            // Built once and cloned into every plugin host so that indicators,
            // analytics, DSL keywords, scheduled jobs, and event-bus subscribers
            // registered by one plugin are visible to the engine and to other
            // plugins. The plugin registry, plugin host, and the strategy
            // engine all hold `Arc`s into the same maps.
            let indicator_registry = Arc::new(SharedIndicatorRegistry::new());
            let analytics_registry = Arc::new(SharedAnalyticsRegistry::new());
            let dsl_ext_registry = Arc::new(SharedDslExtensionRegistry::new());
            // `EventBus::new()` already returns `Arc<Self>`, so don't double-wrap.
            let event_bus = EventBus::new();
            let event_bus_for_state = event_bus.clone();
            let (tauri_ui_api_concrete, ui_receiver) = TauriUiApi::new();
            // `TauriUiApi::new()` already returns an `Arc<TauriUiApi>` as its
            // first element. Re-cast that to `Arc<dyn UiApi>` so the
            // builder's `ui` field accepts it. The concrete `Arc` is kept
            // (under the same name) to subscribe new receivers for the
            // forwarder below.
            let tauri_ui_api: Arc<dyn algomln::plugin::api::UiApi> =
                tauri_ui_api_concrete.clone() as Arc<dyn algomln::plugin::api::UiApi>;
            let scheduler = CronScheduler::new();

            // The plugin's "market data" capability is backed by the same
            // broker the rest of the app uses (Dhan in production). The
            // "execution" capability is a no-op stub for now — see
            // `src/plugin/api/execution.rs`.
            let broker_arc = data.broker.clone();
            let market_data_api: Arc<dyn algomln::plugin::api::MarketDataApi> =
                Arc::new(BrokerMarketDataApi::new(broker_arc));
            let execution_api: Arc<dyn algomln::plugin::api::ExecutionApi> =
                Arc::new(NoopExecutionApi);

            // Per-plugin storage lives under `<app_data>/plugins/<plugin_id>/storage`.
            let plugins_dir = store_dir.join("plugins");
            let _ = std::fs::create_dir_all(&plugins_dir);
            let plugins_dir_for_factory = plugins_dir.clone();

            // Per-plugin rolling logs live under `<app_data>/logs/`.
            // The directory is created lazily by `RateLimitedFileLog::open`,
            // but we ensure it exists now so a misbehaving plugin can't
            // spam `log_info` before the first log call hits a missing dir.
            let logs_dir = store_dir.join("logs");
            let _ = std::fs::create_dir_all(&logs_dir);
            let logs_dir_for_factory = logs_dir.clone();

            let host_factory: algomln::plugin::registry::HostFactory = Arc::new(
                move |id: algomln::plugin::PluginId,
                      caps: Vec<algomln::plugin::Capability>,
                      perms: algomln::plugin::manifest::PluginPermissions| {
                    let storage_dir = plugins_dir_for_factory
                        .join(id.as_ref())
                        .join("storage");
                    let storage = Arc::new(
                        PluginKvStore::new(id.clone(), storage_dir)
                            .expect("plugin storage dir should be creatable"),
                    );
                    // Plugins are untrusted code — every `log_*` call goes
                    // through a per-plugin token-bucket rate limiter and a
                    // 5MB rolling file under `<app_data>/logs/`. The CLI
                    // path keeps using `NamespacedLog` (terminal-friendly,
                    // no file) because the CLI does not load plugins.
                    let log: Arc<dyn algomln::plugin::api::LogApi> = Arc::new(
                        RateLimitedFileLog::open(&logs_dir_for_factory, id.clone())
                            .expect("plugin log file should be creatable"),
                    );
                    algomln::plugin::host::PluginHostBuilder {
                        id: id.clone(),
                        market_data: market_data_api.clone(),
                        execution: execution_api.clone(),
                        storage,
                        event_bus: event_bus.clone(),
                        indicators: indicator_registry.clone(),
                        analytics: analytics_registry.clone(),
                        dsl: dsl_ext_registry.clone(),
                        ui: tauri_ui_api.clone(),
                        scheduler: scheduler.clone(),
                        log,
                        capabilities: caps,
                        permissions: perms,
                    }
                    .build()
                },
            );

            let plugin_registry = PluginRegistry::new(plugins_dir.clone(), host_factory);

            // Synchronous `setup` driving the async `scan_and_load`. Tauri 2
            // installs a multi-thread tokio runtime on the builder, so
            // `tauri::async_runtime::block_on` is safe here.
            let load_results = tauri::async_runtime::block_on(plugin_registry.scan_and_load());
            for (id, result) in &load_results {
                match result {
                    Ok(()) => eprintln!("[plugins] loaded: {id}"),
                    Err(e) => eprintln!("[plugins] failed to load {id}: {e}"),
                }
            }

            // ---------- Forward plugin UI messages to the Tauri bus ----------
            //
            // Plugins call `ui.register_panel` / `ui.notify` / `emit_panel_data`
            // via the `TauriUiApi`, which broadcasts `UiMessage`s on a tokio
            // channel. We re-emit each message on the Tauri event bus as
            // `"plugin-ui-message"` so the React app can subscribe to a single
            // channel and dispatch on the `UiMessage` variant.
            let app_handle = app.handle().clone();
            let mut ui_rx = tauri_ui_api_concrete.receiver();
            tauri::async_runtime::spawn(async move {
                while let Ok(msg) = ui_rx.recv().await {
                    let _ = app_handle.emit("plugin-ui-message", &msg);
                }
            });

            // ---------- Acrylic window chrome (Windows only) ----------
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::apply_acrylic;

                let win = app
                    .get_webview_window("main")
                    .expect("main window not found");

                win.set_decorations(false)?;

                // WebView2 paints an opaque white background by default, which
                // sits *on top* of the acrylic and makes the glass look like a
                // flat muddy gray. Force the controller's default background to
                // fully transparent (A: 0) so the acrylic shows through.
                win.with_webview(|webview| {
                    use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_COLOR;
                    unsafe {
                        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller2;
                        use windows_core::Interface;

                        let controller2 = webview
                            .controller()
                            .cast::<ICoreWebView2Controller2>()
                            .expect("failed to get ICoreWebView2Controller2");

                        let _ = controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                                A: 0,
                                R: 0,
                                G: 0,
                                B: 0,
                        });
                    }
                })?;

                apply_acrylic(&win, Some((34, 34, 34, 153)))
                    .expect("Acrylic requires Windows 10 1803+");
            }

            // ---------- Index registry ----------
            //
            // The bundled seed JSON lives under `src-tauri/resources/indices/`
            // and is shipped with the app bundle via `tauri.conf.json`. The
            // `resource_dir` resolver is provided by Tauri. The cache_dir is
            // the user's app data — a background task may refresh the
            // files there on startup if they're older than 24h.
            let index_registry = Arc::new(IndexRegistry::new());
            let resource_dir = app
                .path()
                .resource_dir()
                .expect("could not resolve resource dir");
            // Resources live at `<resource_dir>/resources/indices/*.json` when
            // bundled (the `resources` prefix in tauri.conf.json is preserved).
            let index_resource_dir = resource_dir.join("resources").join("indices");
            let index_cache_dir = store_dir.join("indices");
            let _ = std::fs::create_dir_all(&index_cache_dir);
            index_registry.load_from_dirs(&index_cache_dir, &index_resource_dir);
            eprintln!(
                "[indices] loaded {} index constituent lists",
                index_registry.list_info().len()
            );

            // Spawn a background refresh. Non-fatal: failures are logged to
            // stderr and the app keeps running with the seed data.
            // 90-day staleness window per the multi-symbol trade spec.
            let refresh_registry = index_registry.clone();
            let refresh_cache_dir = index_cache_dir.clone();
            tauri::async_runtime::spawn(async move {
                let outcomes = refresh_all_if_stale(
                    refresh_registry,
                    refresh_cache_dir,
                    std::time::Duration::from_secs(90 * 24 * 60 * 60),
                )
                .await;
                let ok = outcomes.iter().filter(|o| o.success).count();
                eprintln!("[indices] background refresh: {}/{} ok", ok, outcomes.len());
            });


            // Background 7-day staleness check for the symbol map.
            let bg_sym_map = symbol_map.clone();
            let bg_sym_cache = sym_cache_path.clone();
            tauri::async_runtime::spawn(async move {
                if !algomln::indices::is_stale(
                    &bg_sym_cache,
                    std::time::Duration::from_secs(7 * 24 * 60 * 60),
                ) {
                    return;
                }
                match refresh_symbol_map(&bg_sym_cache).await {
                    Ok(new_map) => {
                        eprintln!("[symbol_map] background refresh: {} symbols", new_map.len());
                        *bg_sym_map.write() = new_map;
                    }
                    Err(e) => eprintln!("[symbol_map] background refresh failed: {e}"),
                }
            });

            app.manage(AppState {
                data,
                strategies,
                plugin_registry,
                event_bus: event_bus_for_state,
                ui_receiver,
                index_registry,
                symbol_map,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_ohlcv,
            get_quote,
            subscribe_ticks,
            run_backtest,
            validate_dsl,
            deploy_strategy,
            list_strategies,
            set_strategy_status,
            list_plugins,
            enable_plugin,
            disable_plugin,
            reload_plugins,
            list_indices,
            get_index_symbols,
            refresh_indices,
            search_symbols,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AlgoMLN");
}

fn load_dotenv() {
    for path in [Path::new(".env"), Path::new("../.env")] {
        let Ok(contents) = fs::read_to_string(path) else {
            continue;
        };

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            if env::var(key.trim()).is_err() {
                env::set_var(key.trim(), value.trim().trim_matches('"'));
            }
        }

        break;
    }
}
