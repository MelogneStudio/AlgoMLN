pub mod api;
pub mod host;
pub mod manifest;
pub mod runtime;
pub mod types;
// pub mod loader;
// pub mod registry;

pub use types::*;

#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    fn meta(&self) -> &PluginMeta;
    fn capabilities(&self) -> &[Capability];
    async fn on_load(&mut self, host: std::sync::Arc<host::PluginHost>) -> PluginResult<()>;
    async fn on_enable(&mut self) -> PluginResult<()>;
    async fn on_disable(&mut self) -> PluginResult<()>;
    fn on_unload(&mut self);

    fn has_capability(&self, cap: &Capability) -> bool {
        self.capabilities()
            .iter()
            .any(|c| std::mem::discriminant(c) == std::mem::discriminant(cap))
    }
}
