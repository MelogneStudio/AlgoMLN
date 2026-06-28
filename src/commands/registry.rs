use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::strategy::dsl::{AstValidator, Lexer, Parser};

/// Execution target a deployed strategy is intended for. Serializes as
/// `"paper"` / `"live"` to match the TS union in `src/types/strategy.ts`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StrategyMode {
    Paper,
    Live,
}

impl StrategyMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "paper" => Ok(Self::Paper),
            "live" => Ok(Self::Live),
            other => Err(format!("unknown mode '{other}' — expected 'paper' or 'live'")),
        }
    }
}

/// User-controlled run state. The UI only ever sends `"running"` or `"paused"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StrategyStatus {
    Running,
    Paused,
}

impl StrategyStatus {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            other => Err(format!(
                "unknown status '{other}' — expected 'running' or 'paused'"
            )),
        }
    }
}

/// Wire shape returned to the UI. Mirrors `DeployedStrategy` in
/// `src/types/strategy.ts` field-for-field. The persisted `DeployedStrategyRecord`
/// holds the same data plus a sort key; this is the externally-visible form.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedStrategy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub total_pnl: f64,
    pub total_trades: usize,
    pub modes: Vec<StrategyMode>,
    pub status: StrategyStatus,
    pub dsl_source: String,
}

/// On-disk shape. Identical fields to `DeployedStrategy` plus `deployed_at`
/// (a millisecond timestamp used for stable ordering) and a single `mode`
/// (since each record was deployed once with one mode).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeployedStrategyRecord {
    id: String,
    name: String,
    mode: StrategyMode,
    status: StrategyStatus,
    dsl_source: String,
    description: String,
    total_pnl: f64,
    total_trades: usize,
    deployed_at: i64,
}

impl From<&DeployedStrategyRecord> for DeployedStrategy {
    fn from(record: &DeployedStrategyRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            description: record.description.clone(),
            total_pnl: record.total_pnl,
            total_trades: record.total_trades,
            modes: vec![record.mode],
            status: record.status,
            dsl_source: record.dsl_source.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct RegistryFile {
    strategies: Vec<DeployedStrategyRecord>,
}

/// In-memory registry of deployed strategies, persisted as a single JSON file
/// under `app_data_dir/strategies.json`.
///
/// This is the storage side of deploy/list/set_status only — it does NOT
/// schedule ticks or run a live engine. Real execution is wired separately.
pub struct StrategyRegistry {
    store_path: PathBuf,
    inner: Mutex<HashMap<String, DeployedStrategyRecord>>,
    counter: AtomicU64,
}

impl StrategyRegistry {
    /// Opens (or creates) the registry at `store_path`. Creates the parent
    /// directory if needed and reads any existing file. Missing file is not
    /// an error — it just yields an empty registry.
    pub fn open(store_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = store_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create registry directory {}: {}",
                    parent.display(),
                    error
                )
            })?;
        }

        let records = if Path::new(&store_path).exists() {
            let raw = std::fs::read_to_string(&store_path)
                .map_err(|error| format!("failed to read {}: {}", store_path.display(), error))?;
            if raw.trim().is_empty() {
                Vec::new()
            } else {
                let parsed: RegistryFile = serde_json::from_str(&raw).map_err(|error| {
                    format!(
                        "failed to parse {}: {}",
                        store_path.display(),
                        error
                    )
                })?;
                parsed.strategies
            }
        } else {
            Vec::new()
        };

        let counter = records.len() as u64;
        let mut map = HashMap::with_capacity(records.len());
        for record in records {
            map.insert(record.id.clone(), record);
        }

        Ok(Self {
            store_path,
            inner: Mutex::new(map),
            counter: AtomicU64::new(counter),
        })
    }

    /// Validates the DSL, generates an id, persists a new record, and returns
    /// the id. New strategies default to `Paused` so they don't auto-start;
    /// the user flips to `Running` via `set_strategy_status`.
    pub async fn deploy(
        &self,
        name: &str,
        dsl_source: &str,
        mode: StrategyMode,
    ) -> Result<String, String> {
        let errors = validate_dsl_local(dsl_source);
        if !errors.is_empty() {
            return Err(format!("strategy validation failed: {}", errors.join("; ")));
        }

        let id = self.generate_id();
        let deployed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock before unix epoch: {error}"))?
            .as_millis() as i64;

        let record = DeployedStrategyRecord {
            id: id.clone(),
            name: name.to_string(),
            mode,
            status: StrategyStatus::Paused,
            dsl_source: dsl_source.to_string(),
            description: String::new(),
            total_pnl: 0.0,
            total_trades: 0,
            deployed_at,
        };

        let snapshot = {
            let mut guard = self.inner.lock().await;
            guard.insert(id.clone(), record);
            guard.values().cloned().collect::<Vec<_>>()
        };
        self.persist(&snapshot)?;
        Ok(id)
    }

    /// Returns all deployed strategies sorted by `deployed_at` ascending so
    /// newer entries appear at the bottom of the list.
    pub async fn list(&self) -> Result<Vec<DeployedStrategy>, String> {
        let guard = self.inner.lock().await;
        let mut records = guard.values().cloned().collect::<Vec<_>>();
        records.sort_by_key(|record| record.deployed_at);
        Ok(records.iter().map(DeployedStrategy::from).collect())
    }

    /// Flips the status of the matching strategy and persists.
    pub async fn set_status(&self, id: &str, status: StrategyStatus) -> Result<(), String> {
        let snapshot = {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(id)
                .ok_or_else(|| format!("strategy '{id}' not found"))?;
            record.status = status;
            guard.values().cloned().collect::<Vec<_>>()
        };
        self.persist(&snapshot)
    }

    fn generate_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("strat-{millis}-{n:04}")
    }

    fn persist(&self, snapshot: &[DeployedStrategyRecord]) -> Result<(), String> {
        let file = RegistryFile {
            strategies: snapshot.to_vec(),
        };
        let serialized = serde_json::to_string_pretty(&file)
            .map_err(|error| format!("failed to serialize registry: {error}"))?;
        std::fs::write(&self.store_path, serialized).map_err(|error| {
            format!(
                "failed to write {}: {}",
                self.store_path.display(),
                error
            )
        })?;
        Ok(())
    }
}

/// Local copy of the lex+parse+validate pipeline so the registry doesn't
/// depend on `commands::strategy::validate_dsl` (which would create a cyclic
/// module dependency in the future when commands::strategy uses registry).
fn validate_dsl_local(dsl_source: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let tokens = match Lexer::tokenize(dsl_source) {
        Ok(tokens) => tokens,
        Err(error) => {
            errors.push(format!(
                "line {} col {}: {}",
                error.line, error.col, error.message
            ));
            return errors;
        }
    };

    let node = match Parser::new(tokens).parse() {
        Ok(node) => node,
        Err(error) => {
            errors.push(format!(
                "line {} col {}: {}",
                error.line, error.col, error.message
            ));
            return errors;
        }
    };

    for error in AstValidator::validate(&node) {
        errors.push(error.message);
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("algomln-registry-test-{}-{}", name, std::process::id()));
        // Use a deterministic suffix so concurrent tests don't collide.
        dir
    }

    #[tokio::test]
    async fn deploy_list_set_status_round_trip() {
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);

        let registry = StrategyRegistry::open(path.clone()).unwrap();
        let dsl = "WHEN rsi(14) < 30\nBUY 1";
        let id = registry.deploy("Test", dsl, StrategyMode::Paper).await.unwrap();

        let listed = registry.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].name, "Test");
        assert_eq!(listed[0].modes, vec![StrategyMode::Paper]);
        assert_eq!(listed[0].status, StrategyStatus::Paused);

        registry.set_status(&id, StrategyStatus::Running).await.unwrap();
        let listed = registry.list().await.unwrap();
        assert_eq!(listed[0].status, StrategyStatus::Running);

        // Persistence: a fresh registry should see the same state.
        let again = StrategyRegistry::open(path.clone()).unwrap();
        let listed = again.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, StrategyStatus::Running);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn deploy_rejects_invalid_dsl() {
        let path = temp_path("invalid");
        let _ = std::fs::remove_file(&path);
        let registry = StrategyRegistry::open(path).unwrap();
        let err = registry
            .deploy("Bad", "WHEN rsi(0) < 30\nBUY 0", StrategyMode::Paper)
            .await
            .unwrap_err();
        assert!(err.contains("validation failed"));
    }

    #[tokio::test]
    async fn set_status_unknown_id_returns_error() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        let registry = StrategyRegistry::open(path).unwrap();
        let err = registry
            .set_status("does-not-exist", StrategyStatus::Running)
            .await
            .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn open_missing_file_yields_empty_registry() {
        let path = temp_path("missing-file");
        let _ = std::fs::remove_file(&path);
        let registry = StrategyRegistry::open(path).unwrap();
        assert!(registry.list().await.unwrap().is_empty());
    }

    #[test]
    fn strategy_mode_parses_lowercase() {
        assert_eq!(StrategyMode::parse("paper").unwrap(), StrategyMode::Paper);
        assert_eq!(StrategyMode::parse("LIVE").unwrap(), StrategyMode::Live);
        assert!(StrategyMode::parse("sideways").is_err());
    }

    #[test]
    fn strategy_status_parses_lowercase() {
        assert_eq!(StrategyStatus::parse("running").unwrap(), StrategyStatus::Running);
        assert_eq!(StrategyStatus::parse("Paused").unwrap(), StrategyStatus::Paused);
        assert!(StrategyStatus::parse("stopped").is_err());
    }
}