//! WebAssembly plugin runtime.
//!
//! A `WasmPlugin` loads a `.wasm` artifact, links a small set of
//! capability-gated host functions into the `algomln` module namespace,
//! and invokes the exported `_algomln_on_load` / `_algomln_on_enable` /
//! `_algomln_on_disable` / `_algomln_on_unload` functions at the
//! corresponding lifecycle events.
//!
//! The host surface is intentionally minimal: logging, per-plugin KV
//! storage, notifications, and a panel-data emit hook. WASI is not
//! linked: `WasiCtx` in wasmtime 23 holds trait objects that are
//! `Send`-only and not `Sync`, which would prevent the resulting
//! `Store<WasmState>` (and therefore `WasmPlugin`) from satisfying the
//! `Plugin: Send + Sync` bound. Plugins interact with the platform
//! exclusively through the `algomln::*` host functions below.
//!
//! Memory is bounded by a `ResourceLimiter` that refuses linear-memory
//! growth past the configured `memory_limit_bytes`. CPU is bounded by
//! wasmtime's epoch-interruption mechanism — callers should drive the
//! engine's epoch counter from a watchdog thread if a stricter cap is
//! needed; we arm a one-tick deadline so the trap is available.

use std::path::PathBuf;
use std::sync::Arc;

use wasmtime::ResourceLimiter;
use wasmtime::{Caller, Config, Engine, Instance, Linker, Memory, Module, OptLevel, Store};

use crate::plugin::host::PluginHost;
use crate::plugin::types::{
    Capability, NotificationKind, PluginError, PluginMeta, PluginResult,
};

use super::super::Plugin;

/// A resource limiter that bounds the total linear memory a WASM module
/// may grow to. Refusing growth past the configured cap is the
/// primary memory-isolation mechanism for untrusted plugin code.
///
/// `ResourceLimiter::memory_growing` / `table_growing` receive `u32`
/// byte/page counts (not `usize`), so the limiter operates on `u32`
/// values and rejects any growth past the configured cap.
struct MemoryLimitState {
    memory_limit: u32,
}

impl ResourceLimiter for MemoryLimitState {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(desired as u64 <= self.memory_limit as u64)
    }

    fn table_growing(
        &mut self,
        _current: u32,
        desired: u32,
        _maximum: Option<u32>,
    ) -> Result<bool, wasmtime::Error> {
        // Tables are not exposed through our host surface, so a small
        // generous bound is sufficient.
        Ok(desired <= 10_000)
    }
}

/// WASM-backed plugin. Compiles and instantiates a `.wasm` artifact at
/// load time, links capability-gated host functions into the
/// `algomln` namespace, and dispatches the exported lifecycle hooks.
pub struct WasmPlugin {
    meta: PluginMeta,
    capabilities: Vec<Capability>,
    wasm_path: PathBuf,
    memory_limit_bytes: u32,
    host: Option<Arc<PluginHost>>,
    engine: Engine,
    store: Option<Store<WasmState>>,
    instance: Option<Instance>,
}

/// Per-store state: the plugin's host handle and an inline
/// resource limiter. Kept narrow so that `WasmState` (and therefore
/// `WasmPlugin`) can satisfy the `Send + Sync` bound that the
/// `Plugin` trait requires.
///
/// Note: WASI is intentionally **not** linked in this build. `WasiCtx`
/// in wasmtime 23 holds trait objects that are `Send`-only and not
/// `Sync`, which would prevent the resulting `Store<WasmState>` (and
/// therefore `WasmPlugin`) from satisfying the `Plugin: Send + Sync`
/// bound. There is therefore no `wasi` field on this struct.
pub struct WasmState {
    pub host: Arc<PluginHost>,
    #[allow(private_interfaces)]
    pub(crate) memory_limiter: MemoryLimitState,
}

impl WasmPlugin {
    /// Build a new WASM plugin. The wasmtime engine is constructed
    /// eagerly; compilation and instantiation happen in `on_load`.
    pub fn new(
        meta: PluginMeta,
        capabilities: Vec<Capability>,
        wasm_path: PathBuf,
        memory_limit_mb: u32,
    ) -> PluginResult<Self> {
        let mut config = Config::new();
        config.async_support(false);
        config.epoch_interruption(true);
        config.cranelift_opt_level(OptLevel::Speed);
        let engine = Engine::new(&config).map_err(|e| {
            PluginError::LoadFailed(format!("wasmtime engine construction failed: {e}"))
        })?;

        let memory_limit_bytes = memory_limit_mb.saturating_mul(1024 * 1024);

        Ok(Self {
            meta,
            capabilities,
            wasm_path,
            memory_limit_bytes,
            host: None,
            engine,
            store: None,
            instance: None,
        })
    }
}

/// Read a UTF-8 string from WASM linear memory at `[ptr..ptr+len]`.
/// Returns lossy-decoded data on invalid UTF-8 so a buggy plugin
/// still produces a log line rather than crashing the host.
fn read_string_from_memory(caller: &mut Caller<'_, WasmState>, ptr: i32, len: i32) -> String {
    let memory = match caller.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => m,
        _ => return String::new(),
    };
    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize).min(data.len());
    if start >= data.len() {
        return String::new();
    }
    String::from_utf8_lossy(&data[start..end]).into_owned()
}

/// Write raw bytes into WASM linear memory at `ptr`. Out-of-bounds
/// writes will trap the WASM instance; we let wasmtime surface the
/// trap rather than pre-validating, matching the host's other
/// "let it trap" patterns.
fn write_bytes_to_memory(caller: &mut Caller<'_, WasmState>, ptr: i32, bytes: &[u8]) {
    let memory = match caller.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => m,
        _ => panic!("algomln host call invoked without a `memory` export"),
    };
    let mem_data = memory.data_mut(caller);
    let start = ptr as usize;
    let end = start + bytes.len();
    assert!(end <= mem_data.len(), "algomln host write out of bounds");
    mem_data[start..end].copy_from_slice(bytes);
}

fn memory_of(caller: &mut Caller<'_, WasmState>) -> Option<Memory> {
    match caller.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => Some(m),
        _ => None,
    }
}

fn write_i32(caller: &mut Caller<'_, WasmState>, ptr: i32, value: i32) {
    if let Some(mem) = memory_of(caller) {
        let data = mem.data_mut(caller);
        let slot = ptr as usize;
        if slot + 4 <= data.len() {
            data[slot..slot + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
}

fn read_bytes_from_memory(
    caller: &mut Caller<'_, WasmState>,
    ptr: i32,
    len: i32,
) -> Option<Vec<u8>> {
    let mem = memory_of(caller)?;
    let data = mem.data(caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize).min(data.len());
    if start >= data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
}

/// Build a linker pre-populated with the `algomln` host functions.
/// The host functions are pure capability bridges — they only return
/// data the plugin's declared capabilities allow.
fn build_linker(engine: &Engine) -> PluginResult<Linker<WasmState>> {
    let mut linker: Linker<WasmState> = Linker::new(engine);

    // ---- Logging (unguarded; every plugin may log). ----
    linker
        .func_wrap(
            "algomln",
            "log_info",
            |mut caller: Caller<'_, WasmState>, ptr: i32, len: i32| {
                let msg = read_string_from_memory(&mut caller, ptr, len);
                let host = caller.data().host.clone();
                let pid = host.id.clone();
                host.log().info(&pid, &msg);
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::log_info: {e}")))?;

    linker
        .func_wrap(
            "algomln",
            "log_warn",
            |mut caller: Caller<'_, WasmState>, ptr: i32, len: i32| {
                let msg = read_string_from_memory(&mut caller, ptr, len);
                let host = caller.data().host.clone();
                let pid = host.id.clone();
                host.log().warn(&pid, &msg);
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::log_warn: {e}")))?;

    linker
        .func_wrap(
            "algomln",
            "log_error",
            |mut caller: Caller<'_, WasmState>, ptr: i32, len: i32| {
                let msg = read_string_from_memory(&mut caller, ptr, len);
                let host = caller.data().host.clone();
                let pid = host.id.clone();
                host.log().error(&pid, &msg);
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::log_error: {e}")))?;

    // ---- Per-plugin storage (Storage capability). ----
    //
    // `StorageApi::read` and `StorageApi::write` are synchronous
    // (the `async_trait` is on the trait only for forward compat), so
    // we call them directly without going through `block_on`.
    linker
        .func_wrap(
            "algomln",
            "storage_get",
            |mut caller: Caller<'_, WasmState>,
             key_ptr: i32,
             key_len: i32,
             out_ptr: i32,
             out_len_ptr: i32|
             -> i32 {
                let key = read_string_from_memory(&mut caller, key_ptr, key_len);
                let storage = match caller.data().host.storage_guarded() {
                    Ok(s) => s.clone(),
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log().error(&pid, &format!("storage_get: {e}"));
                        return -1;
                    }
                };
                let result = storage.read(&key);
                match result {
                    Ok(Some(bytes)) => {
                        write_bytes_to_memory(&mut caller, out_ptr, &bytes);
                        write_i32(&mut caller, out_len_ptr, bytes.len() as i32);
                        0
                    }
                    Ok(None) => {
                        write_i32(&mut caller, out_len_ptr, 0);
                        1
                    }
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log().error(&pid, &format!("storage_get: {e}"));
                        -1
                    }
                }
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::storage_get: {e}")))?;

    linker
        .func_wrap(
            "algomln",
            "storage_set",
            |mut caller: Caller<'_, WasmState>,
             key_ptr: i32,
             key_len: i32,
             val_ptr: i32,
             val_len: i32|
             -> i32 {
                let key = read_string_from_memory(&mut caller, key_ptr, key_len);
                let val = match read_bytes_from_memory(&mut caller, val_ptr, val_len) {
                    Some(v) => v,
                    None => return -1,
                };
                let storage = match caller.data().host.storage_guarded() {
                    Ok(s) => s.clone(),
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log().error(&pid, &format!("storage_set: {e}"));
                        return -1;
                    }
                };
                let result = storage.write(&key, &val);
                match result {
                    Ok(()) => 0,
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log().error(&pid, &format!("storage_set: {e}"));
                        -1
                    }
                }
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::storage_set: {e}")))?;

    // ---- UI notifications (UiPanels capability). ----
    linker
        .func_wrap(
            "algomln",
            "notify",
            |mut caller: Caller<'_, WasmState>, msg_ptr: i32, msg_len: i32, kind: i32| {
                let msg = read_string_from_memory(&mut caller, msg_ptr, msg_len);
                let ui = match caller.data().host.ui_guarded() {
                    Ok(u) => u.clone(),
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log().error(&pid, &format!("notify: {e}"));
                        return;
                    }
                };
                let mapped = match kind {
                    0 => NotificationKind::Info,
                    1 => NotificationKind::Warning,
                    _ => NotificationKind::Error,
                };
                let _ = ui.notify(mapped, &msg);
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::notify: {e}")))?;

    // ---- Panel data emit (UiPanels capability; downcasts to the
    // Tauri-backed impl so the broadcast channel picks the value up). ----
    linker
        .func_wrap(
            "algomln",
            "emit_panel_data",
            |mut caller: Caller<'_, WasmState>,
             panel_id_ptr: i32,
             panel_id_len: i32,
             json_ptr: i32,
             json_len: i32|
             -> i32 {
                let panel_id = read_string_from_memory(&mut caller, panel_id_ptr, panel_id_len);
                let json_str = read_string_from_memory(&mut caller, json_ptr, json_len);
                let value: serde_json::Value = match serde_json::from_str(&json_str) {
                    Ok(v) => v,
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log()
                            .error(&pid, &format!("emit_panel_data: invalid json: {e}"));
                        return -1;
                    }
                };
                let ui = match caller.data().host.ui_guarded() {
                    Ok(u) => u.clone(),
                    Err(e) => {
                        let host = caller.data().host.clone();
                        let pid = host.id.clone();
                        host.log().error(&pid, &format!("emit_panel_data: {e}"));
                        return -1;
                    }
                };
                if let Some(tauri_ui) =
                    ui.as_any().downcast_ref::<crate::plugin::api::ui::TauriUiApi>()
                {
                    match tauri_ui.emit_panel_data(panel_id, value) {
                        Ok(()) => 0,
                        Err(e) => {
                            let host = caller.data().host.clone();
                            let pid = host.id.clone();
                            host.log().error(&pid, &format!("emit_panel_data: {e}"));
                            -1
                        }
                    }
                } else {
                    let host = caller.data().host.clone();
                    let pid = host.id.clone();
                    host.log()
                        .error(&pid, "emit_panel_data: host ui is not a TauriUiApi");
                    -1
                }
            },
        )
        .map_err(|e| PluginError::LoadFailed(format!("link algomln::emit_panel_data: {e}")))?;

    Ok(linker)
}

/// Call an exported lifecycle function on the WASM instance, if it
/// exists. Missing exports are silently treated as "no-op plugin";
/// any other trap bubbles up as `PluginError::LoadFailed`.
fn call_lifecycle(
    store: &mut Store<WasmState>,
    instance: &Instance,
    name: &str,
) -> PluginResult<()> {
    let func = match instance.get_typed_func::<(), ()>(&mut *store, name) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    func.call(&mut *store, ())
        .map_err(|e| PluginError::LoadFailed(format!("{name} failed: {e}")))?;
    Ok(())
}

#[async_trait::async_trait]
impl Plugin for WasmPlugin {
    fn meta(&self) -> &PluginMeta {
        &self.meta
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn on_load(&mut self, host: Arc<PluginHost>) -> PluginResult<()> {
        self.host = Some(host.clone());

        let bytes = std::fs::read(&self.wasm_path).map_err(|e| {
            PluginError::LoadFailed(format!(
                "failed to read wasm artifact {}: {e}",
                self.wasm_path.display()
            ))
        })?;

        let module = Module::new(&self.engine, &bytes)
            .map_err(|e| PluginError::LoadFailed(format!("module compile failed: {e}")))?;

        let linker = build_linker(&self.engine)?;

        let memory_limit_bytes = self.memory_limit_bytes;
        let state = WasmState {
            host,
            memory_limiter: MemoryLimitState {
                memory_limit: memory_limit_bytes,
            },
        };
        let mut store = Store::new(&self.engine, state);

        // Wire the resource limiter by handing wasmtime a mutable
        // reference to the inline `MemoryLimitState` field on
        // `WasmState`. This avoids any per-call allocation and keeps
        // the limit configuration co-located with the rest of the
        // store state.
        store.limiter(|s: &mut WasmState| &mut s.memory_limiter);

        // Arm the epoch-interruption check. The host should drive the
        // engine's epoch counter from a watchdog thread if a CPU cap
        // is required; we set the deadline to 1 so the check is
        // available on the next instruction.
        store.set_epoch_deadline(1);

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| PluginError::LoadFailed(format!("instantiate failed: {e}")))?;

        // Dispatch the lifecycle hook.
        call_lifecycle(&mut store, &instance, "_algomln_on_load")?;

        self.store = Some(store);
        self.instance = Some(instance);
        Ok(())
    }

    async fn on_enable(&mut self) -> PluginResult<()> {
        let store = self
            .store
            .as_mut()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        let instance = self
            .instance
            .as_ref()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        call_lifecycle(store, instance, "_algomln_on_enable")
    }

    async fn on_disable(&mut self) -> PluginResult<()> {
        let store = self
            .store
            .as_mut()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        let instance = self
            .instance
            .as_ref()
            .ok_or_else(|| PluginError::ApiError("plugin not loaded".into()))?;
        call_lifecycle(store, instance, "_algomln_on_disable")
    }

    fn on_unload(&mut self) {
        if let (Some(mut store), Some(instance)) = (self.store.take(), self.instance.take()) {
            // Best-effort: errors from on_unload are ignored per spec.
            let _ = call_lifecycle(&mut store, &instance, "_algomln_on_unload");
        }
        self.host = None;
    }
}
