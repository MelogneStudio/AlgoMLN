use std::fs;
use std::path::{Path, PathBuf};

use crate::plugin::types::{PluginError, PluginId, PluginResult};

use super::StorageApi;

pub struct PluginKvStore {
    pub plugin_id: PluginId,
    pub base_dir: PathBuf,
}

impl PluginKvStore {
    pub fn new(plugin_id: PluginId, base_dir: PathBuf) -> PluginResult<Self> {
        fs::create_dir_all(&base_dir)
            .map_err(|e| PluginError::ApiError(format!("create_dir_all failed: {e}")))?;
        Ok(Self {
            plugin_id,
            base_dir,
        })
    }
}

fn sanitize_key(key: &str) -> String {
    let mut sanitized = String::with_capacity(key.len());
    for ch in key.chars() {
        match ch {
            '/' | '\\' | ':' => sanitized.push('_'),
            // Replace the literal ".." substring by tracking it separately.
            _ => sanitized.push(ch),
        }
    }
    // Replace ".." substring with "__" (two underscores).
    let sanitized = sanitized.replace("..", "__");
    // Truncate to 200 chars (by char count, not bytes).
    let truncated: String = sanitized.chars().take(200).collect();
    if truncated.is_empty() {
        "_empty_".to_string()
    } else {
        truncated
    }
}

fn path_for(base_dir: &Path, key: &str) -> PathBuf {
    let sanitized = sanitize_key(key);
    base_dir.join(format!("{sanitized}.val"))
}

#[async_trait::async_trait]
impl StorageApi for PluginKvStore {
    fn read(&self, key: &str) -> PluginResult<Option<Vec<u8>>> {
        let path = path_for(&self.base_dir, key);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PluginError::ApiError(format!("read failed: {e}"))),
        }
    }

    fn write(&self, key: &str, value: &[u8]) -> PluginResult<()> {
        let path = path_for(&self.base_dir, key);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, value)
            .map_err(|e| PluginError::ApiError(format!("write tmp failed: {e}")))?;
        // Atomic rename.
        fs::rename(&tmp, &path)
            .map_err(|e| PluginError::ApiError(format!("rename failed: {e}")))?;
        Ok(())
    }

    fn delete(&self, key: &str) -> PluginResult<()> {
        let path = path_for(&self.base_dir, key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PluginError::ApiError(format!("delete failed: {e}"))),
        }
    }

    fn list_keys(&self, prefix: &str) -> PluginResult<Vec<String>> {
        let entries = fs::read_dir(&self.base_dir)
            .map_err(|e| PluginError::ApiError(format!("read_dir failed: {e}")))?;
        let mut keys = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|e| PluginError::ApiError(format!("dir entry error: {e}")))?;
            let name = entry.file_name().to_string_lossy().to_string();
            // Only consider *.val files.
            let stripped = match name.strip_suffix(".val") {
                Some(s) => s,
                None => continue,
            };
            // Compare against the prefix by reconstructing the user-supplied "key" path.
            // We match on the raw sanitized portion which is how files are named.
            let user_prefix = sanitize_key(prefix);
            if stripped.starts_with(&user_prefix) {
                keys.push(stripped.to_string());
            }
        }
        Ok(keys)
    }
}
