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
    fn record_metric(
        &self,
        name: &str,
        value: f64,
        tags: &[(&str, &str)],
    ) -> PluginResult<()> {
        // The trait-level API is fire-and-forget. We log to stderr so the
        // plugin author can see the metric; the registry's plugin-id-aware
        // dedup logic is exposed via `register_fn` for the structured case.
        let tag_str: Vec<String> = tags
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        eprintln!(
            "[analytics] {name}={value} tags=[{}]",
            tag_str.join(", ")
        );
        Ok(())
    }

    fn record_event(&self, name: &str, properties: &[(&str, &str)]) -> PluginResult<()> {
        let prop_str: Vec<String> = properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        eprintln!(
            "[analytics] event={name} props=[{}]",
            prop_str.join(", ")
        );
        Ok(())
    }
}
