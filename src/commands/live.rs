use tauri::State;

use crate::{
    commands::state::AppState,
    live::trade_log::{TradeLog, TradeLogEntry},
};

/// Returns all entries from the trade log, newest first.
pub async fn get_trade_log(state: State<'_, AppState>) -> Result<Vec<TradeLogEntry>, String> {
    TradeLog::read_all(&state.trade_log_path)
        .map(|mut v| {
            v.reverse();
            v
        })
        .map_err(|e| e.to_string())
}
