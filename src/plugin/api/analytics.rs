use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::plugin::types::{PluginError, PluginId, PluginResult};
use crate::strategy::execution::paper::PaperTrade;

use super::AnalyticsApi;

pub type AnalyticsFn = dyn Fn(&[PaperTrade]) -> f64 + Send + Sync;

pub struct SharedAnalyticsRegistry {
    inner: Arc<RwLock<HashMap<String, (PluginId, Arc<AnalyticsFn>)>>>,
}

impl SharedAnalyticsRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for SharedAnalyticsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedAnalyticsRegistry {
    pub fn register_fn(
        &self,
        name: &str,
        plugin_id: PluginId,
        f: Arc<AnalyticsFn>,
    ) -> PluginResult<()> {
        let mut map = self.inner.write();
        if let Some((existing, _)) = map.get(name) {
            if existing != &plugin_id {
                return Err(PluginError::ApiError(format!(
                    "analytic '{name}' already registered by plugin '{existing}'"
                )));
            }
        }
        map.insert(name.to_string(), (plugin_id, f));
        Ok(())
    }

    pub fn get_fn(&self, name: &str) -> Option<Arc<AnalyticsFn>> {
        self.inner.read().get(name).map(|(_, f)| f.clone())
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.inner.read().keys().cloned().collect();
        names.sort();
        names
    }

    pub fn unregister_all_for(&self, plugin_id: &PluginId) {
        self.inner.write().retain(|_, (id, _)| id != plugin_id);
    }
}

#[async_trait::async_trait]
impl AnalyticsApi for SharedAnalyticsRegistry {
    fn register_metric(
        &self,
        name: String,
        registered_by: PluginId,
        f: std::sync::Arc<dyn Fn(&[PaperTrade]) -> f64 + Send + Sync>,
    ) -> PluginResult<()> {
        let mut map = self.inner.write();
        if let Some((existing, _)) = map.get(&name) {
            if existing != &registered_by {
                return Err(PluginError::ApiError(format!(
                    "metric '{name}' already registered by plugin '{existing}'"
                )));
            }
            // Same plugin re-registration: silent overwrite.
        }
        map.insert(name, (registered_by, f));
        Ok(())
    }

    fn get_metric(
        &self,
        name: &str,
    ) -> Option<std::sync::Arc<dyn Fn(&[PaperTrade]) -> f64 + Send + Sync>> {
        self.inner.read().get(name).map(|(_, f)| f.clone())
    }

    fn list_metrics(&self) -> Vec<String> {
        let mut names: Vec<String> = self.inner.read().keys().cloned().collect();
        names.sort();
        names
    }
}
