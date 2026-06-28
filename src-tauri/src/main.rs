use std::{env, fs, path::Path, sync::Arc};

use algomln::{
    broker::Timeframe,
    commands::{
        self,
        registry::{DeployedStrategy, StrategyMode, StrategyRegistry, StrategyStatus},
        strategy::{run_backtest_dsl, BacktestResultWire},
    },
    models::{Candle, Quote},
};
use tauri::State;

struct AppState {
    data: commands::data::DataState,
    strategies: Arc<StrategyRegistry>,
}

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

fn main() {
    load_dotenv();

    let data = commands::data::DataState::dhan_from_env()
        .expect("Set DHAN_ACCESS_TOKEN in .env before starting the Tauri app");

    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;

            let store_dir = app
                .path()
                .app_data_dir()
                .expect("could not resolve app data dir");
            let store_path = store_dir.join("strategies.json");
            let registry = StrategyRegistry::open(store_path.clone())
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to open strategy registry at {}: {error}",
                        store_path.display()
                    )
                });
            app.manage(AppState {
                data,
                strategies: Arc::new(registry),
            });

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
            set_strategy_status
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
