//! Plugin loader: reads a plugin directory and returns a boxed `Plugin`.
//!
//! The loader is intentionally thin — it doesn't hold any state. It
//! just inspects the manifest, dispatches to the right runtime, and
//! hands back a fully constructed `Plugin` ready to be registered.

use crate::plugin::manifest::PluginManifest;
use crate::plugin::runtime::{rhai_runtime::RhaiPlugin, wasm_runtime::WasmPlugin};
use crate::plugin::types::{PluginError, PluginResult};

use super::Plugin;

pub struct PluginLoader;

impl PluginLoader {
    /// Load a plugin from a directory containing `plugin.toml` and the
    /// entry file referenced by the manifest. Dispatches to either
    /// the Rhai or WASM runtime based on the entry file's extension.
    pub fn load_from_dir(dir: &std::path::Path) -> PluginResult<Box<dyn Plugin>> {
        let manifest = PluginManifest::load(dir)?;
        let meta = manifest.to_meta()?;
        let capabilities = manifest.to_capabilities()?;
        let entry = dir.join(&manifest.entry);

        let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "rhai" => Ok(Box::new(RhaiPlugin::new(meta, capabilities, entry)?)),
            "wasm" => Ok(Box::new(WasmPlugin::new(
                meta,
                capabilities,
                entry,
                manifest.permissions.max_memory_mb,
            )?)),
            other => Err(PluginError::LoadFailed(format!(
                "unknown entry type: {}",
                other
            ))),
        }
    }
}
