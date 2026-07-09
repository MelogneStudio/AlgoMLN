pub mod analytics;
pub mod dsl_extension;
pub mod events;
pub mod execution;
pub mod indicator_registry;
pub mod log;
pub mod market_data;
pub mod scheduler;
pub mod storage;
pub mod ui;

use std::sync::Arc;

use crate::plugin::types::{NotificationKind, PluginId, PluginResult, ScheduleHandle, SubscriptionHandle};

#[async_trait::async_trait]
pub trait MarketDataApi: Send + Sync {
    fn subscribe_ticks(
        &self,
        symbol: &str,
        callback: Arc<dyn Fn(MarketDataEvent) + Send + Sync>,
    ) -> PluginResult<SubscriptionHandle>;
    fn unsubscribe_ticks(&self, handle: SubscriptionHandle) -> PluginResult<()>;
    fn latest_candle(&self, symbol: &str) -> PluginResult<Candle>;
}

#[derive(Debug, Clone)]
pub struct MarketDataEvent {
    pub symbol: String,
    pub kind: MarketEventKind,
}

#[derive(Debug, Clone, Copy)]
pub enum MarketEventKind {
    Candle,
    Tick,
}

#[derive(Debug, Clone)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub timestamp_ms: i64,
}

#[async_trait::async_trait]
pub trait ExecutionApi: Send + Sync {
    async fn submit_order(&self, order: OrderRequest) -> PluginResult<String>;
    async fn cancel_order(&self, order_id: &str) -> PluginResult<()>;
    fn positions(&self) -> PluginResult<Vec<Position>>;
}

#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: u32,
    pub order_type: OrderType,
    pub price: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub quantity: i64,
    pub average_price: f64,
}

#[async_trait::async_trait]
pub trait StorageApi: Send + Sync {
    fn read(&self, key: &str) -> PluginResult<Option<Vec<u8>>>;
    fn write(&self, key: &str, value: &[u8]) -> PluginResult<()>;
    fn delete(&self, key: &str) -> PluginResult<()>;
    fn list_keys(&self, prefix: &str) -> PluginResult<Vec<String>>;
}

#[async_trait::async_trait]
pub trait IndicatorRegistryApi: Send + Sync {
    fn register(
        &self,
        name: String,
        registered_by: crate::plugin::PluginId,
        f: std::sync::Arc<dyn Fn(&[crate::models::Candle], usize) -> Vec<f64> + Send + Sync>,
    ) -> crate::plugin::PluginResult<()>;
    fn get(&self, name: &str) -> PluginResult<Box<dyn IndicatorInstance>>;
    /// Downcast hook so plugin runtimes (e.g. Rhai) can recover the
    /// concrete registry type to access plugin-id-attributed APIs
    /// like `register_fn` that the trait-level surface cannot express.
    fn as_any(&self) -> &dyn std::any::Any;
}

pub trait IndicatorInstance: Send + Sync {
    fn update(&mut self, value: f64);
    fn value(&self) -> Option<f64>;
    fn name(&self) -> &str;
}

#[async_trait::async_trait]
pub trait AnalyticsApi: Send + Sync {
    fn register_metric(
        &self,
        name: String,
        registered_by: crate::plugin::PluginId,
        f: std::sync::Arc<dyn Fn(&[crate::models::PaperTrade]) -> f64 + Send + Sync>,
    ) -> crate::plugin::PluginResult<()>;
    fn get_metric(
        &self,
        name: &str,
    ) -> Option<std::sync::Arc<dyn Fn(&[crate::models::PaperTrade]) -> f64 + Send + Sync>>;
    fn list_metrics(&self) -> Vec<String>;
}

#[async_trait::async_trait]
pub trait DslExtensionApi: Send + Sync {
    fn register_keyword(
        &self,
        keyword: String,
        registered_by: crate::plugin::PluginId,
        handler: std::sync::Arc<
            dyn Fn(&crate::strategy::runtime::context::EvalContext) -> bool + Send + Sync,
        >,
    ) -> crate::plugin::PluginResult<()>;
    fn get_keyword(
        &self,
        keyword: &str,
    ) -> Option<
        std::sync::Arc<
            dyn Fn(&crate::strategy::runtime::context::EvalContext) -> bool + Send + Sync,
        >,
    >;
    fn unregister_all_for(&self, plugin_id: &crate::plugin::PluginId);
}

#[async_trait::async_trait]
pub trait UiApi: Send + Sync {
    fn register_panel(&self, panel: UiPanel) -> PluginResult<()>;
    fn notify(&self, kind: NotificationKind, message: &str) -> PluginResult<()>;
    fn list_panels(&self) -> Vec<(String, String)>;
    /// Downcast hook so plugin runtimes (e.g. WASM) can recover the
    /// concrete `TauriUiApi` type to call `emit_panel_data`, which
    /// lives outside the trait surface.
    fn as_any(&self) -> &dyn std::any::Any;
}

#[derive(Debug, Clone)]
pub struct UiPanel {
    pub id: String,
    pub title: String,
    pub route: String,
}

#[async_trait::async_trait]
pub trait SchedulerApi: Send + Sync {
    fn schedule(
        &self,
        cron: &str,
        task: Arc<dyn Fn() + Send + Sync>,
    ) -> PluginResult<ScheduleHandle>;
    fn cancel(&self, handle: ScheduleHandle) -> PluginResult<()>;
    fn cancel_all_for(&self, plugin_id: &PluginId);
}

#[async_trait::async_trait]
pub trait LogApi: Send + Sync {
    fn debug(&self, plugin_id: &PluginId, message: &str);
    fn info(&self, plugin_id: &PluginId, message: &str);
    fn warn(&self, plugin_id: &PluginId, message: &str);
    fn error(&self, plugin_id: &PluginId, message: &str);
}
