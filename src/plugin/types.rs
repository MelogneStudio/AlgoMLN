use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PluginId(pub String);

impl PluginId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for PluginId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for PluginId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for PluginId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for PluginId {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PluginVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl TryFrom<&str> for PluginVersion {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let parts: Vec<&str> = value.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("invalid version string: {value}"));
        }
        let major = parts[0]
            .parse::<u32>()
            .map_err(|e| format!("invalid major: {e}"))?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|e| format!("invalid minor: {e}"))?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|e| format!("invalid patch: {e}"))?;
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub id: PluginId,
    pub name: String,
    pub version: PluginVersion,
    pub description: String,
    pub author: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Capability {
    MarketData,
    Execution,
    Storage,
    Events,
    Indicators,
    Analytics,
    DslExtension,
    UiPanels,
    Scheduler,
}

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::MarketData => "marketData",
            Capability::Execution => "execution",
            Capability::Storage => "storage",
            Capability::Events => "events",
            Capability::Indicators => "indicators",
            Capability::Analytics => "analytics",
            Capability::DslExtension => "dslExtension",
            Capability::UiPanels => "uiPanels",
            Capability::Scheduler => "scheduler",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for Capability {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let normalized = value.to_ascii_lowercase();
        match normalized.as_str() {
            "marketdata" | "market_data" | "market-data" => Ok(Capability::MarketData),
            "execution" => Ok(Capability::Execution),
            "storage" => Ok(Capability::Storage),
            "events" => Ok(Capability::Events),
            "indicators" => Ok(Capability::Indicators),
            "analytics" => Ok(Capability::Analytics),
            "dslextension" | "dsl_extension" | "dsl-extension" => Ok(Capability::DslExtension),
            "uipanels" | "ui_panels" | "ui-panels" => Ok(Capability::UiPanels),
            "scheduler" => Ok(Capability::Scheduler),
            other => Err(format!("unknown capability: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginStatus {
    Loaded,
    Enabled,
    Disabled,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginError {
    ManifestParse(String),
    LoadFailed(String),
    ApiError(String),
    PermissionDenied {
        capability: String,
        plugin_id: String,
    },
    Timeout(String),
    NotFound(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::ManifestParse(m) => write!(f, "manifest parse error: {m}"),
            PluginError::LoadFailed(m) => write!(f, "load failed: {m}"),
            PluginError::ApiError(m) => write!(f, "api error: {m}"),
            PluginError::PermissionDenied {
                capability,
                plugin_id,
            } => {
                write!(
                    f,
                    "permission denied for capability {capability} in plugin {plugin_id}"
                )
            }
            PluginError::Timeout(m) => write!(f, "timeout: {m}"),
            PluginError::NotFound(m) => write!(f, "not found: {m}"),
        }
    }
}

impl std::error::Error for PluginError {}

pub type PluginResult<T> = Result<T, PluginError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SubscriptionHandle(pub uuid::Uuid);

impl fmt::Display for SubscriptionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ScheduleHandle(pub uuid::Uuid);

impl fmt::Display for ScheduleHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    Info,
    Warning,
    Error,
}

impl fmt::Display for NotificationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotificationKind::Info => f.write_str("info"),
            NotificationKind::Warning => f.write_str("warning"),
            NotificationKind::Error => f.write_str("error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginListEntry {
    pub meta: PluginMeta,
    pub status: PluginStatus,
    pub capabilities: Vec<Capability>,
}
