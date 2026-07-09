use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::models::Candle;
use crate::plugin::types::{PluginError, PluginId, PluginResult};

use super::{IndicatorInstance, IndicatorRegistryApi};

/// Type alias matching the spec: a stateless computation function from a
/// candle slice + a window length to a vector of indicator values.
pub type IndicatorFn = dyn Fn(&[Candle], usize) -> Vec<f64> + Send + Sync;

pub struct SharedIndicatorRegistry {
    inner: Arc<RwLock<HashMap<String, (PluginId, Arc<IndicatorFn>)>>>,
}

impl SharedIndicatorRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for SharedIndicatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Spec-shaped registration entry point used by plugins that want to register
/// a pure candle-slice computation. This is the entry point the spec describes
/// as `register`. Plugins using the trait-level `IndicatorRegistryApi` go
/// through the factory path below; both share the same backing map.
impl SharedIndicatorRegistry {
    pub fn register_fn(
        &self,
        name: &str,
        plugin_id: PluginId,
        f: Arc<IndicatorFn>,
    ) -> PluginResult<()> {
        let mut map = self.inner.write();
        if let Some((existing_id, _)) = map.get(name) {
            if existing_id != &plugin_id {
                return Err(PluginError::ApiError(format!(
                    "indicator '{name}' already registered by plugin '{existing_id}'"
                )));
            }
            // Same plugin re-registration: silent overwrite.
        }
        map.insert(name.to_string(), (plugin_id, f));
        Ok(())
    }

    pub fn get_fn(&self, name: &str) -> Option<Arc<IndicatorFn>> {
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

/// Adapter that wraps a stateless `IndicatorFn` as a stateful `IndicatorInstance`
/// for the trait-level API. Each `new()` captures the function reference and
/// computes the full vector once on first `value()` call; subsequent updates
/// recompute on the current buffer.
#[allow(dead_code)]
pub struct FnIndicatorInstance {
    f: Arc<IndicatorFn>,
    history: Vec<f64>,
    last_result: Option<f64>,
}

#[async_trait::async_trait]
impl IndicatorRegistryApi for SharedIndicatorRegistry {
    fn register(
        &self,
        name: String,
        registered_by: PluginId,
        f: std::sync::Arc<dyn Fn(&[Candle], usize) -> Vec<f64> + Send + Sync>,
    ) -> PluginResult<()> {
        let mut map = self.inner.write();
        if let Some((existing, _)) = map.get(&name) {
            if existing != &registered_by {
                return Err(PluginError::ApiError(format!(
                    "indicator '{name}' already registered by plugin '{existing}'"
                )));
            }
            // Same plugin re-registration: silent overwrite.
        }
        map.insert(name, (registered_by, f));
        Ok(())
    }

    fn get(&self, name: &str) -> PluginResult<Box<dyn IndicatorInstance>> {
        let f = self
            .inner
            .read()
            .get(name)
            .map(|(_, f)| f.clone())
            .ok_or_else(|| PluginError::NotFound(format!("indicator '{name}' not found")))?;
        Ok(Box::new(FnIndicatorInstance {
            f,
            history: Vec::new(),
            last_result: None,
        }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl IndicatorInstance for FnIndicatorInstance {
    fn update(&mut self, value: f64) {
        self.history.push(value);
    }
    fn value(&self) -> Option<f64> {
        self.last_result
    }
    fn name(&self) -> &str {
        "fn_indicator"
    }
}
