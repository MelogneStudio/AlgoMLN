use std::path::Path;

use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;

use super::types::{
    Capability, PluginError, PluginId, PluginMeta, PluginResult, PluginVersion,
};

lazy_static! {
    static ref PLUGIN_ID_RE: Regex =
        Regex::new(r"^[a-z0-9][a-z0-9\-]*[a-z0-9]$").expect("valid plugin id regex");
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub capabilities: Vec<String>,
    pub entry: String,
    pub permissions: PluginPermissions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginPermissions {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub file_system: bool,
    #[serde(default = "default_memory")]
    pub max_memory_mb: u32,
    #[serde(default)]
    pub allowed_symbols: Vec<String>,
}

fn default_memory() -> u32 {
    32
}

impl PluginManifest {
    pub fn load(plugin_dir: &Path) -> PluginResult<Self> {
        let manifest_path = plugin_dir.join("plugin.toml");
        let raw = std::fs::read_to_string(&manifest_path)
            .map_err(|e| PluginError::ManifestParse(format!("read {}: {e}", manifest_path.display())))?;

        let manifest: PluginManifest = toml::from_str(&raw)
            .map_err(|e| PluginError::ManifestParse(format!("toml parse: {e}")))?;

        if manifest.id.len() < 2 || !PLUGIN_ID_RE.is_match(&manifest.id) {
            return Err(PluginError::ManifestParse(
                "id must be kebab-case".into(),
            ));
        }

        let _ = PluginVersion::try_from(manifest.version.as_str())
            .map_err(|e| PluginError::ManifestParse(e))?;

        let mut unknown = Vec::new();
        for cap in &manifest.capabilities {
            if let Err(e) = Capability::try_from(cap.as_str()) {
                unknown.push(format!("{} ({e})", cap));
            }
        }
        if !unknown.is_empty() {
            return Err(PluginError::ManifestParse(format!(
                "unknown capabilities: {}",
                unknown.join(", ")
            )));
        }

        let entry_path = plugin_dir.join(&manifest.entry);
        if !entry_path.is_file() {
            return Err(PluginError::ManifestParse(format!(
                "entry file '{}' not found",
                manifest.entry
            )));
        }

        let ext = entry_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ext != "rhai" && ext != "wasm" {
            return Err(PluginError::ManifestParse(
                "entry must be .rhai or .wasm".into(),
            ));
        }

        Ok(manifest)
    }

    pub fn to_meta(&self) -> PluginResult<PluginMeta> {
        let version = PluginVersion::try_from(self.version.as_str())
            .map_err(|e| PluginError::ManifestParse(e))?;
        Ok(PluginMeta {
            id: PluginId::from(self.id.clone()),
            name: self.name.clone(),
            version,
            description: self.description.clone(),
            author: self.author.clone(),
        })
    }

    pub fn to_capabilities(&self) -> PluginResult<Vec<Capability>> {
        self.capabilities
            .iter()
            .map(|c| {
                Capability::try_from(c.as_str())
                    .map_err(|e| PluginError::ManifestParse(e))
            })
            .collect()
    }
}
