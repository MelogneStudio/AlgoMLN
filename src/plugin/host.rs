use std::sync::Arc;

use crate::plugin::manifest::PluginPermissions;
use crate::plugin::types::{Capability, PluginError, PluginId, PluginResult};

use super::api::{
    AnalyticsApi, DslExtensionApi, ExecutionApi, IndicatorRegistryApi, LogApi, MarketDataApi,
    SchedulerApi, StorageApi, UiApi,
};

/// The runtime handle given to a plugin when it is loaded. The host exposes
/// capability-gated access to platform services — plugins must request the
/// `*_guarded` accessor corresponding to each capability listed in their
/// manifest, and any undeclared access is denied.
pub struct PluginHost {
    pub id: PluginId,
    pub market_data: Arc<dyn MarketDataApi>,
    pub execution: Arc<dyn ExecutionApi>,
    pub storage: Arc<dyn StorageApi>,
    pub event_bus: Arc<crate::plugin::api::events::EventBus>,
    pub indicators: Arc<dyn IndicatorRegistryApi>,
    pub analytics: Arc<dyn AnalyticsApi>,
    pub dsl: Arc<dyn DslExtensionApi>,
    pub ui: Arc<dyn UiApi>,
    pub scheduler: Arc<dyn SchedulerApi>,
    pub log: Arc<dyn LogApi>,
    pub(crate) capabilities: Vec<Capability>,
    pub(crate) permissions: PluginPermissions,
}

impl PluginHost {
    /// Verify that the host was built with a capability matching `cap`.
    /// Matching is done by variant discriminant so that
    /// `Capability::MarketData` matches a registered `Capability::MarketData`
    /// regardless of any future associated payload.
    pub fn require_capability(&self, cap: &Capability) -> PluginResult<()> {
        let needle = std::mem::discriminant(cap);
        let granted = self
            .capabilities
            .iter()
            .any(|c| std::mem::discriminant(c) == needle);
        if granted {
            Ok(())
        } else {
            Err(PluginError::PermissionDenied {
                capability: cap.to_string(),
                plugin_id: self.id.to_string(),
            })
        }
    }

    pub fn market_data_guarded(&self) -> PluginResult<&Arc<dyn MarketDataApi>> {
        self.require_capability(&Capability::MarketData)?;
        Ok(&self.market_data)
    }

    pub fn execution_guarded(&self) -> PluginResult<&Arc<dyn ExecutionApi>> {
        self.require_capability(&Capability::Execution)?;
        Ok(&self.execution)
    }

    pub fn storage_guarded(&self) -> PluginResult<&Arc<dyn StorageApi>> {
        self.require_capability(&Capability::Storage)?;
        Ok(&self.storage)
    }

    pub fn event_bus_guarded(&self) -> PluginResult<&Arc<crate::plugin::api::events::EventBus>> {
        self.require_capability(&Capability::Events)?;
        Ok(&self.event_bus)
    }

    pub fn indicators_guarded(&self) -> PluginResult<&Arc<dyn IndicatorRegistryApi>> {
        self.require_capability(&Capability::Indicators)?;
        Ok(&self.indicators)
    }

    pub fn analytics_guarded(&self) -> PluginResult<&Arc<dyn AnalyticsApi>> {
        self.require_capability(&Capability::Analytics)?;
        Ok(&self.analytics)
    }

    pub fn dsl_guarded(&self) -> PluginResult<&Arc<dyn DslExtensionApi>> {
        self.require_capability(&Capability::DslExtension)?;
        Ok(&self.dsl)
    }

    pub fn ui_guarded(&self) -> PluginResult<&Arc<dyn UiApi>> {
        self.require_capability(&Capability::UiPanels)?;
        Ok(&self.ui)
    }

    pub fn scheduler_guarded(&self) -> PluginResult<&Arc<dyn SchedulerApi>> {
        self.require_capability(&Capability::Scheduler)?;
        Ok(&self.scheduler)
    }

    /// Logging is intentionally unguarded — every plugin can always log,
    /// regardless of declared capabilities.
    pub fn log(&self) -> &Arc<dyn LogApi> {
        &self.log
    }
}

/// Builder for `PluginHost`. Mirrors the host's fields; `build` consumes
/// the builder and produces an `Arc<PluginHost>` suitable for handing to
/// `Plugin::on_load`.
pub struct PluginHostBuilder {
    pub id: PluginId,
    pub market_data: Arc<dyn MarketDataApi>,
    pub execution: Arc<dyn ExecutionApi>,
    pub storage: Arc<dyn StorageApi>,
    pub event_bus: Arc<crate::plugin::api::events::EventBus>,
    pub indicators: Arc<dyn IndicatorRegistryApi>,
    pub analytics: Arc<dyn AnalyticsApi>,
    pub dsl: Arc<dyn DslExtensionApi>,
    pub ui: Arc<dyn UiApi>,
    pub scheduler: Arc<dyn SchedulerApi>,
    pub log: Arc<dyn LogApi>,
    pub capabilities: Vec<Capability>,
    pub permissions: PluginPermissions,
}

impl PluginHostBuilder {
    pub fn build(self) -> Arc<PluginHost> {
        Arc::new(PluginHost {
            id: self.id,
            market_data: self.market_data,
            execution: self.execution,
            storage: self.storage,
            event_bus: self.event_bus,
            indicators: self.indicators,
            analytics: self.analytics,
            dsl: self.dsl,
            ui: self.ui,
            scheduler: self.scheduler,
            log: self.log,
            capabilities: self.capabilities,
            permissions: self.permissions,
        })
    }
}
