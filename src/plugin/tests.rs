use std::sync::Arc;

use crate::models::Candle;
use crate::plugin::api::events::{EventBus, EventFilter, EventKind};
use crate::plugin::api::indicator_registry::{IndicatorFn, SharedIndicatorRegistry};
use crate::plugin::api::storage::PluginKvStore;
use crate::plugin::api::StorageApi;
use crate::plugin::manifest::PluginManifest;
use crate::plugin::types::{PluginError, PluginId};
use crate::strategy::execution::paper::PaperTrade;

fn dummy_candle() -> Candle {
    Candle {
        timestamp: 0,
        open: 1.0,
        high: 2.0,
        low: 0.5,
        close: 1.5,
        volume: 100.0,
    }
}

fn dummy_trade() -> PaperTrade {
    PaperTrade {
        id: "trade-1".to_string(),
        timestamp: 0,
        symbol: "NIFTY".to_string(),
        side: crate::models::OrderSide::Buy,
        quantity: 1,
        price: 100.0,
        rule_id: "rule-1".to_string(),
        pnl: None,
    }
}

#[tokio::test]
async fn storage_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        PluginKvStore::new(PluginId::from("test-plugin"), dir.path().to_path_buf()).unwrap();
    store.write("hello", b"world").unwrap();
    assert_eq!(store.read("hello").unwrap(), Some(b"world".to_vec()));
    store.delete("hello").unwrap();
    assert_eq!(store.read("hello").unwrap(), None);
    let keys = store.list_keys("").unwrap();
    assert!(keys.is_empty());
}

#[test]
fn storage_key_sanitization() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        PluginKvStore::new(PluginId::from("test-plugin"), dir.path().to_path_buf()).unwrap();
    store.write("../../etc/passwd", b"bad").unwrap();
    assert!(!std::path::Path::new("../../etc/passwd").exists());
    assert_eq!(
        store.read("../../etc/passwd").unwrap(),
        Some(b"bad".to_vec())
    );
    let keys = store.list_keys("").unwrap();
    assert!(keys.iter().all(|k| !k.contains("..")));
}

#[test]
fn indicator_registry_dedup() {
    let registry = SharedIndicatorRegistry::new();
    let pid_a = PluginId::from("plugin-a");
    let pid_b = PluginId::from("plugin-b");
    let dummy_fn: Arc<IndicatorFn> = Arc::new(|_candles: &[Candle], _period: usize| vec![]);

    assert!(registry
        .register_fn("my_ind", pid_a.clone(), dummy_fn.clone())
        .is_ok());
    assert!(registry
        .register_fn("my_ind", pid_a.clone(), dummy_fn.clone())
        .is_ok());
    assert!(matches!(
        registry.register_fn("my_ind", pid_b.clone(), dummy_fn.clone()),
        Err(PluginError::ApiError(_))
    ));

    registry.unregister_all_for(&pid_a);
    assert!(registry.list().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn event_bus_filter() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let bus = EventBus::new();
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = call_count.clone();

    bus.subscribe(
        EventFilter::TradeExecuted,
        Arc::new(move |_| {
            cc.fetch_add(1, Ordering::SeqCst);
        }),
    );

    bus.publish(EventKind::CandleProcessed(dummy_candle()));
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(call_count.load(Ordering::SeqCst), 0);

    bus.publish(EventKind::TradeExecuted(dummy_trade()));
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[test]
fn manifest_validation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plugin.rhai"), "").unwrap();

    // Valid manifest
    std::fs::write(
        dir.path().join("plugin.toml"),
        r#"id = "my-plugin"
name = "My Plugin"
version = "0.1.0"
description = "Test"
author = "Test"
capabilities = ["Indicators"]
entry = "plugin.rhai"
[permissions]
max_memory_mb = 8
network = false
file_system = false
"#,
    )
    .unwrap();
    let manifest = PluginManifest::load(dir.path()).unwrap();
    let meta = manifest.to_meta().unwrap();
    assert_eq!(meta.id.as_str(), "my-plugin");
    assert_eq!(meta.name, "My Plugin");

    // Bad version
    std::fs::write(
        dir.path().join("plugin.toml"),
        r#"id = "my-plugin"
name = "My Plugin"
version = "bad"
description = "Test"
author = "Test"
capabilities = []
entry = "plugin.rhai"
[permissions]
"#,
    )
    .unwrap();
    assert!(matches!(
        PluginManifest::load(dir.path()),
        Err(PluginError::ManifestParse(_))
    ));

    // Unknown capability
    std::fs::write(
        dir.path().join("plugin.toml"),
        r#"id = "my-plugin"
name = "My Plugin"
version = "0.1.0"
description = "Test"
author = "Test"
capabilities = ["Unknown"]
entry = "plugin.rhai"
[permissions]
"#,
    )
    .unwrap();
    assert!(matches!(
        PluginManifest::load(dir.path()),
        Err(PluginError::ManifestParse(_))
    ));

    // Bad extension: create the .dll file so we reach the extension check
    std::fs::write(dir.path().join("plugin.dll"), "").unwrap();
    std::fs::write(
        dir.path().join("plugin.toml"),
        r#"id = "my-plugin"
name = "My Plugin"
version = "0.1.0"
description = "Test"
author = "Test"
capabilities = []
entry = "plugin.dll"
[permissions]
"#,
    )
    .unwrap();
    assert!(matches!(
        PluginManifest::load(dir.path()),
        Err(PluginError::ManifestParse(_))
    ));

    // Entry file not found
    std::fs::write(
        dir.path().join("plugin.toml"),
        r#"id = "my-plugin"
name = "My Plugin"
version = "0.1.0"
description = "Test"
author = "Test"
capabilities = []
entry = "nonexistent.rhai"
[permissions]
"#,
    )
    .unwrap();
    assert!(matches!(
        PluginManifest::load(dir.path()),
        Err(PluginError::ManifestParse(_))
    ));
}
