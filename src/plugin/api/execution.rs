//! `ExecutionApi` adapter for plugins.
//!
//! Two implementations live here:
//!
//! - [`NoopExecutionApi`] — the default. Used when no live session is active
//!   and the plugin capability list is otherwise static. Every call is a
//!   structured error explaining why.
//! - [`ReadOnlyLiveExecutionApi`] — wired into a real, in-flight live
//!   session. Plugins can **inspect** open positions but **cannot submit
//!   orders**. Live order submission is reserved for the strategy engine
//!   so every real order is funnelled through `MAX_DAILY_LOSS`, market-
//!   hours, pause, and trade-log gates.
//!
//! Phase 7 deliberately does not expose a live plugin `submit_order`. A
//! properly gated plugin order gateway that re-runs every engine gate is
//! Phase 8 work. See CLAUDE.md invariant 18 (this file).

use crate::plugin::types::{PluginError, PluginResult};
use crate::strategy::execution::DhanBroker;

use super::{ExecutionApi, OrderRequest, Position};

/// Map the broker's `Position` (richer: includes LTP, realised/unrealised
/// PnL) to the plugin-visible subset (`symbol`, `quantity`,
/// `average_price`). The plugin layer never sees PnL because the API
/// contract is intentionally minimal.
impl From<crate::models::Position> for Position {
    fn from(p: crate::models::Position) -> Self {
        Position {
            symbol: p.symbol,
            quantity: p.quantity,
            average_price: p.average_price,
        }
    }
}

/// No-op `ExecutionApi`. Used as the default for every plugin host when no
/// live session is in flight, and as a fallback for plugins that don't
/// declare the `Execution` capability at all.
pub struct NoopExecutionApi;

#[async_trait::async_trait]
impl ExecutionApi for NoopExecutionApi {
    async fn submit_order(&self, _order: OrderRequest) -> PluginResult<String> {
        Err(PluginError::ApiError(
            "plugin order submission is not wired: live orders may only originate \
             from the strategy engine"
                .to_string(),
        ))
    }

    async fn cancel_order(&self, _order_id: &str) -> PluginResult<()> {
        Err(PluginError::ApiError(
            "plugin order submission is not wired: cancel_order is not supported \
             outside the strategy engine"
                .to_string(),
        ))
    }

    fn positions(&self) -> PluginResult<Vec<Position>> {
        Ok(Vec::new())
    }
}

/// Read-only live execution API. Plugins can query the open-position
/// snapshot during an active live session but cannot place or cancel
/// orders. See module docs for the Phase-7 rationale.
///
/// `handle` is captured at host-factory construction time (not at call
/// time) because the plugin callback may run on a non-tokio thread. Using
/// `block_in_place` requires the **multi-thread** tokio runtime; Tauri 2
/// ships one by default but `spawn_blocking` would be the safer escape
/// hatch for any current-thread context. We assert at construction time
/// and document the assumption — see `src-tauri/src/main.rs` for the
/// factory wiring.
pub struct ReadOnlyLiveExecutionApi {
    broker: Arc<DhanBroker>,
    handle: tokio::runtime::Handle,
}

impl ReadOnlyLiveExecutionApi {
    pub fn new(broker: Arc<DhanBroker>, handle: tokio::runtime::Handle) -> Self {
        // Debug-build assertion that the runtime we captured is multi-
        // threaded. If a single-threaded runtime ever wires this in,
        // `block_in_place` would panic at the first `positions()` call;
        // surface the misconfiguration loudly at boot instead.
        let runtime_type = handle.runtime_flavor();
        debug_assert!(
            matches!(runtime_type, tokio::runtime::RuntimeFlavor::MultiThread),
            "ReadOnlyLiveExecutionApi requires a multi-thread tokio runtime \
             (got {runtime_type:?}); block_in_place would panic"
        );
        Self { broker, handle }
    }
}

#[async_trait::async_trait]
impl ExecutionApi for ReadOnlyLiveExecutionApi {
    async fn submit_order(&self, _order: OrderRequest) -> PluginResult<String> {
        Err(PluginError::ApiError(
            "plugin order submission is disabled in Phase 7; live orders may only \
             originate from the strategy engine. A properly gated plugin order \
             gateway is Phase 8 work."
                .to_string(),
        ))
    }

    async fn cancel_order(&self, _order_id: &str) -> PluginResult<()> {
        Err(PluginError::ApiError(
            "plugin order submission is disabled in Phase 7; cancel is not \
             supported outside the strategy engine."
                .to_string(),
        ))
    }

    fn positions(&self) -> PluginResult<Vec<Position>> {
        // Async → sync bridge. The trait's `positions()` is sync; the
        // underlying `DhanBroker::get_positions()` is async. Use
        // `block_in_place` so we don't stall the multi-thread runtime's
        // worker pool, then run the future on the captured handle.
        let broker = self.broker.clone();
        tokio::task::block_in_place(|| {
            self.handle.block_on(async move {
                broker
                    .get_positions()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))
                    .map(|ps| ps.into_iter().map(Into::into).collect())
            })
        })
    }
}