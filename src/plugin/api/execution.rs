//! `ExecutionApi` adapter for plugins.
//!
//! Strategy execution (live / paper) is owned by `StrategyEngine` +
//! `ExecutionTarget` in the strategy layer. The plugin layer does not run
//! strategies itself, so for now the plugin-visible `ExecutionApi` is a
//! no-op stub: any `submit_order` returns an `ApiError` and `positions`
//! returns an empty list. A future revision can wire this into a real
//! broker-agnostic execution facade that proxies to the paper/live
//! targets the engine uses.

use crate::plugin::types::{PluginError, PluginResult};

use super::{ExecutionApi, OrderRequest, Position};

pub struct NoopExecutionApi;

#[async_trait::async_trait]
impl ExecutionApi for NoopExecutionApi {
    async fn submit_order(&self, _order: OrderRequest) -> PluginResult<String> {
        Err(PluginError::ApiError(
            "plugin execution is a no-op stub: submit_order is not yet wired into the strategy engine"
                .to_string(),
        ))
    }

    async fn cancel_order(&self, _order_id: &str) -> PluginResult<()> {
        Err(PluginError::ApiError(
            "plugin execution is a no-op stub: cancel_order is not yet wired into the strategy engine"
                .to_string(),
        ))
    }

    fn positions(&self) -> PluginResult<Vec<Position>> {
        Ok(Vec::new())
    }
}
