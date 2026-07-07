//! Plugin registry: holds all loaded plugins, dispatches lifecycle
//! transitions, and exposes a snapshot for the UI.
//!
//! The registry stores a manifest alongside each plugin so the original
//! `plugin.toml` (and the permissions it declared) is available even
//! after the boxed plugin has taken ownership of the meta/capabilities
//! it needs internally.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::plugin::host::PluginHost;
use crate::plugin::loader::PluginLoader;
use crate::plugin::manifest::{PluginManifest, PluginPermissions};
use crate::plugin::types::{
    PluginError, PluginId, PluginListEntry, PluginResult, PluginStatus, ScheduleHandle,
};

use super::Plugin;

/// One loaded plugin and the bookkeeping the registry needs to keep
/// it alive across lifecycle transitions.
pub struct PluginEntry {
    pub plugin: Box<dyn Plugin>,
    pub status: PluginStatus,
    pub manifest: PluginManifest,
    pub schedule_handles: Vec<ScheduleHandle>,
}

/// In-memory registry of all loaded plugins. Wrapped in an `Arc` so
/// callers (Tauri commands, the strategy engine, the plugin host
/// factory) can hold shared references.
pub struct PluginRegistry {
    plugins: Arc<RwLock<HashMap<PluginId, PluginEntry>>>,
    plugins_dir: PathBuf,
    host_factory: Arc<
        dyn Fn(PluginId, Vec<crate::plugin::types::Capability>, PluginPermissions) -> Arc<PluginHost>
            + Send
            + Sync,
    >,
}

/// Host-factory signature: given a plugin id, declared capabilities,
/// and permissions, produce a fully constructed `Arc<PluginHost>`.
pub type HostFactory = Arc<
    dyn Fn(PluginId, Vec<crate::plugin::types::Capability>, PluginPermissions) -> Arc<PluginHost>
        + Send
        + Sync,
>;

impl PluginRegistry {
    pub fn new(plugins_dir: PathBuf, host_factory: HostFactory) -> Arc<Self> {
        Arc::new(Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            plugins_dir,
            host_factory,
        })
    }

    /// Scan the plugins directory and load every subdirectory that
    /// contains a valid `plugin.toml`. Each entry is loaded
    /// independently — a failure in one plugin does not block the
    /// others. Returns one `(dir_name, result)` pair per attempted
    /// load so callers can surface per-plugin errors to the UI.
    pub async fn scan_and_load(&self) -> Vec<(String, PluginResult<()>)> {
        let mut results: Vec<(String, PluginResult<()>)> = Vec::new();

        let entries = match std::fs::read_dir(&self.plugins_dir) {
            Ok(e) => e,
            Err(err) => {
                results.push((
                    self.plugins_dir.display().to_string(),
                    Err(PluginError::LoadFailed(format!(
                        "read plugins_dir: {err}"
                    ))),
                ));
                return results;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = entry
                .file_name()
                .to_string_lossy()
                .to_string();

            let plugin_result = PluginLoader::load_from_dir(Path::new(&path));
            let mut plugin = match plugin_result {
                Ok(p) => p,
                Err(e) => {
                    results.push((dir_name, Err(e)));
                    continue;
                }
            };

            // Re-parse the manifest so we can stash it on the entry
            // and so we have permissions to hand to the host factory.
            // The loader has already validated everything, so this
            // re-parse is purely a lookup.
            let manifest = match PluginManifest::load(Path::new(&path)) {
                Ok(m) => m,
                Err(e) => {
                    results.push((dir_name, Err(e)));
                    continue;
                }
            };

            let id = plugin.meta().id.clone();
            let caps: Vec<crate::plugin::types::Capability> =
                plugin.capabilities().to_vec();
            let permissions = manifest.permissions.clone();
            let host = (self.host_factory)(id.clone(), caps.clone(), permissions.clone());

            let load_result = plugin.on_load(host).await;
            match load_result {
                Ok(()) => {
                    let id_for_results = id.to_string();
                    let entry = PluginEntry {
                        plugin,
                        status: PluginStatus::Loaded,
                        manifest,
                        schedule_handles: Vec::new(),
                    };
                    self.plugins.write().insert(id, entry);
                    results.push((id_for_results, Ok(())));
                }
                Err(e) => {
                    let id_for_results = id.to_string();
                    let entry = PluginEntry {
                        plugin,
                        status: PluginStatus::Failed(e.to_string()),
                        manifest,
                        schedule_handles: Vec::new(),
                    };
                    self.plugins.write().insert(id.clone(), entry);
                    results.push((id_for_results, Err(e)));
                }
            }
        }

        results
    }

    /// Move a loaded plugin into the `Enabled` state.
    pub async fn enable(&self, id: &PluginId) -> PluginResult<()> {
        let mut guard = self.plugins.write();
        let entry = guard
            .get_mut(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;
        if let PluginStatus::Failed(msg) = &entry.status {
            return Err(PluginError::ApiError(format!(
                "cannot enable failed plugin: {msg}"
            )));
        }
        entry.plugin.on_enable().await?;
        entry.status = PluginStatus::Enabled;
        Ok(())
    }

    /// Move an enabled plugin back to `Disabled`.
    pub async fn disable(&self, id: &PluginId) -> PluginResult<()> {
        let mut guard = self.plugins.write();
        let entry = guard
            .get_mut(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;
        entry.plugin.on_disable().await?;
        entry.status = PluginStatus::Disabled;
        Ok(())
    }

    /// Tear down a plugin and remove it from the registry.
    pub async fn unload(&self, id: &PluginId) -> PluginResult<()> {
        let mut guard = self.plugins.write();
        let mut entry = guard
            .remove(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;
        entry.plugin.on_unload();
        Ok(())
    }

    /// Snapshot of every loaded plugin for the UI.
    pub fn list(&self) -> Vec<PluginListEntry> {
        let guard = self.plugins.read();
        let mut out: Vec<PluginListEntry> = guard
            .values()
            .map(|entry| PluginListEntry {
                meta: entry.plugin.meta().clone(),
                status: entry.status.clone(),
                capabilities: entry.plugin.capabilities().to_vec(),
            })
            .collect();
        out.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
        out
    }

    /// Current status for a single plugin, or `None` if unknown.
    pub fn get_status(&self, id: &PluginId) -> Option<PluginStatus> {
        self.plugins.read().get(id).map(|e| e.status.clone())
    }
}
