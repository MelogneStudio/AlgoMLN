//! `DslExtensionApi` adapter for plugins.
//!
//! Plugins can register DSL keywords that the strategy evaluator will
//! resolve at runtime. Like the indicator and analytics registries, this
//! is a process-wide shared map keyed by keyword name, with per-plugin
//! attribution so the registry can clean up a plugin's contributions
//! when it is disabled.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::plugin::types::{PluginError, PluginId, PluginResult};
use crate::strategy::runtime::context::EvalContext;

use super::DslExtensionApi;

pub type DslKeywordHandler = dyn Fn(&EvalContext) -> bool + Send + Sync;

pub struct SharedDslExtensionRegistry {
    inner: Arc<RwLock<HashMap<String, (PluginId, Arc<DslKeywordHandler>)>>>,
}

impl SharedDslExtensionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Concrete (non-trait) registration entry point. Plugins that want to
    /// attach a custom keyword handler use this directly; the trait-level
    /// `register_keyword` below shares the same backing map.
    pub fn register_handler(
        &self,
        keyword: &str,
        plugin_id: PluginId,
        handler: Arc<DslKeywordHandler>,
    ) -> PluginResult<()> {
        let mut map = self.inner.write();
        if let Some((existing_id, _)) = map.get(keyword) {
            if existing_id != &plugin_id {
                return Err(PluginError::ApiError(format!(
                    "DSL keyword '{keyword}' already registered by plugin '{existing_id}'"
                )));
            }
            // Same plugin re-registration: silent overwrite.
        }
        map.insert(keyword.to_string(), (plugin_id, handler));
        Ok(())
    }

    pub fn get_handler(&self, keyword: &str) -> Option<Arc<DslKeywordHandler>> {
        self.inner.read().get(keyword).map(|(_, h)| h.clone())
    }

    pub fn list_keywords(&self) -> Vec<String> {
        let mut names: Vec<String> = self.inner.read().keys().cloned().collect();
        names.sort();
        names
    }

    pub fn unregister_all_for(&self, plugin_id: &PluginId) {
        self.inner.write().retain(|_, (id, _)| id != plugin_id);
    }
}

impl Default for SharedDslExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl DslExtensionApi for SharedDslExtensionRegistry {
    fn register_keyword(
        &self,
        keyword: String,
        registered_by: PluginId,
        handler: std::sync::Arc<
            dyn Fn(&crate::strategy::runtime::context::EvalContext) -> bool + Send + Sync,
        >,
    ) -> PluginResult<()> {
        let mut map = self.inner.write();
        if let Some((existing, _)) = map.get(&keyword) {
            if existing != &registered_by {
                return Err(PluginError::ApiError(format!(
                    "DSL keyword '{keyword}' already registered by plugin '{existing}'"
                )));
            }
        }
        map.insert(keyword, (registered_by, handler));
        Ok(())
    }

    fn get_keyword(
        &self,
        keyword: &str,
    ) -> Option<
        std::sync::Arc<
            dyn Fn(&crate::strategy::runtime::context::EvalContext) -> bool + Send + Sync,
        >,
    > {
        self.inner
            .read()
            .get(keyword)
            .map(|(_, h)| h.clone() as std::sync::Arc<_>)
    }

    fn unregister_all_for(&self, plugin_id: &PluginId) {
        self.inner.write().retain(|_, (id, _)| id != plugin_id);
    }
}
