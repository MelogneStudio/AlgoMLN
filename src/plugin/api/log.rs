use crate::plugin::types::PluginId;

use super::LogApi;

pub struct NamespacedLog {
    pub plugin_id: String,
}

impl NamespacedLog {
    pub fn new(plugin_id: PluginId) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl LogApi for NamespacedLog {
    fn debug(&self, _plugin_id: &PluginId, message: &str) {
        eprintln!("[plugin:{}] [DEBUG] {}", self.plugin_id, message);
    }
    fn info(&self, _plugin_id: &PluginId, message: &str) {
        eprintln!("[plugin:{}] [INFO] {}", self.plugin_id, message);
    }
    fn warn(&self, _plugin_id: &PluginId, message: &str) {
        eprintln!("[plugin:{}] [WARN] {}", self.plugin_id, message);
    }
    fn error(&self, _plugin_id: &PluginId, message: &str) {
        eprintln!("[plugin:{}] [ERROR] {}", self.plugin_id, message);
    }
}
