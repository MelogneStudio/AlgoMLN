# Plan: Upgrade wasmtime 23 → 48

Status: superseded 2026-08-28 by [wasmtime-upgrade-plan.md](wasmtime-upgrade-plan.md) (chunks 1–3 done; kept as historical reference)
Author context: written 2026-08-27 on branch `fix/b3-wasm-cpu-cap`
Latest published wasmtime at time of writing: **48.0.1** (2026-08-25); 49.0.0 unreleased.

## 1. Current state

- `Cargo.toml:29-30` pins `wasmtime = "23"` and `wasmtime-wasi = "23"`; `src-tauri/Cargo.lock` resolves both to `23.0.3`.
- The **only** file in the workspace that touches wasmtime is
  [wasm_runtime.rs](src/plugin/runtime/wasm_runtime.rs) (655 lines).
- `wasmtime-wasi` is **completely unused** — `grep` for `wasmtime_wasi` across `src/` and
  `src-tauri/src/` returns nothing. WASI is deliberately not linked (invariant 7: `WasiCtx`
  in 23 is `Send`-only, which would break the `Plugin: Send + Sync` bound).
- Local toolchain: rustc 1.98.0. wasmtime 48 needs **1.95.0**, so no toolchain work.

API surface actually used (the whole blast radius):

| Item | Where |
| --- | --- |
| `Config::{new, async_support, epoch_interruption, cranelift_opt_level}` + `OptLevel::Speed` | `WasmPlugin::new` |
| `Engine::{new, clone, increment_epoch}` | `WasmPlugin::new`, `EpochWatchdog::spawn` |
| `ResourceLimiter::{memory_growing, table_growing}` | `MemoryLimitState` |
| `Store::{new, limiter, set_epoch_deadline}` | `on_load`, `call_lifecycle` |
| `Linker::{new, func_wrap, instantiate}` | `build_linker`, `on_load` |
| `Caller::{data, get_export}`, `Extern::Memory`, `Memory::{data, data_mut}` | memory helpers + all 7 host fns |
| `Module::new` (fed `.wat` text in tests) | `on_load`, tests |
| `Instance::{new, get_typed_func}`, `TypedFunc::call` | `call_lifecycle`, tests |
| `wasmtime::Error` | `ResourceLimiter` return type |

## 2. What actually breaks

### 2.1 `ResourceLimiter::table_growing` signature — **the one certain break**

wasmtime 48 (docs.rs, confirmed):

```rust
pub trait ResourceLimiter: Send {
    fn memory_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> Result<bool>;
    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> Result<bool>;
}
```

Our impl still uses the wasmtime-23 shape `table_growing(&mut self, current: u32, desired: u32, maximum: Option<u32>)`
(`wasm_runtime.rs:73-82`). `memory_growing` is already `usize`-based and needs no change; the
doc comment on `MemoryLimitState` (lines 56-58) claiming both callbacks receive `u32` becomes wrong
and must be rewritten.

Also note the new `ResourceLimiter: Send` supertrait — `MemoryLimitState { memory_limit: u32 }` is
trivially `Send`, so no impact.

### 2.2 Things that are *not* documented as broken

The 48.0.0 release notes contain no changes to `Config::epoch_interruption`,
`cranelift_opt_level`, `async_support`, `Engine::increment_epoch`, `Store::set_epoch_deadline`,
`Store::limiter`, `Linker::func_wrap`, `Caller::get_export`, or `Module::new`-with-WAT. `wat` is
still a default feature in 48, so the two `.wat` test modules keep compiling with no new
dev-dependency. Treat this as "expected to be fine", not "verified" — the 24→47 per-release notes
live on their own release branches and were not read; the compiler is the real oracle here (step 3.2).

### 2.3 Risk areas to check empirically (not confirmed either way)

1. **Dependency-graph churn.** wasmtime 48 pulls much newer `cranelift-*`, `wasmparser`,
   `object`, `gimli`, `addr2line`. Possible duplicate-version bloat or a resolver conflict with
   Tauri 2's tree. `src-tauri/Cargo.lock` is the one that matters for the app build.
2. **Binary size / compile time.** 48's default feature set is large (28 default features:
   component-model, component-model-async, gc-*, stack-switching, pooling-allocator,
   parallel-compilation, profiling, coredump, debug-builtins, wit-parser, …). We use *none* of
   the component model, GC, or async. Trimming defaults is an optional win (step 5).
3. **`OptLevel` variants.** Enum may have gained variants; `OptLevel::Speed` is expected to
   remain but confirm at compile time.
4. **WASI Send/Sync premise.** Modern `wasmtime-wasi` was reworked (p2/p3 split, `WasiCtxView`).
   The invariant-7 rationale ("`WasiCtx` is `Send`-only") may no longer hold in 48. This upgrade
   does **not** attempt to link WASI — but the comment should stop asserting a version-specific
   fact as if it were permanent (see step 4).

## 3. Execution steps

### 3.1 Bump and drop the unused crate

`Cargo.toml`:

```diff
-wasmtime = "23"
-wasmtime-wasi = "23"
+wasmtime = "48"
```

Removing `wasmtime-wasi` entirely is correct — it is an unused dependency today, and keeping a
23-pinned WASI crate alongside wasmtime 48 would fail to resolve anyway. If WASI is wanted later
it comes back as a separate, scoped piece of work.

### 3.2 Let the compiler enumerate the rest

```bash
cargo check --workspace --all-targets
```

Fix the reported breakages. Expected: exactly one — `table_growing`. Anything beyond that is a
signal from §2.3 and should be handled in the same commit rather than worked around.

### 3.3 Apply the known fix

In `src/plugin/runtime/wasm_runtime.rs`:

- `table_growing`: change all three params to `usize`, keep the `desired <= 10_000` bound
  (compare against a `usize` literal now).
- Rewrite the `MemoryLimitState` doc comment: both callbacks take `usize`; `memory_growing`
  args are page-aligned byte counts, `table_growing` args are element counts.
- Optional hardening, now that the trait exposes them: implement the provided `instances()` /
  `tables()` / `memories()` caps instead of relying on the 10,000 defaults. Out of scope unless
  cheap — note it and move on.

### 3.4 Verify

```bash
cargo test --workspace
```

The two B3 guards are the load-bearing checks and must stay green:

- `epoch_watchdog_traps_infinite_loop` — proves epoch interruption still traps a `(loop br 0)`
  export under 48. This is the single most important signal in the whole upgrade: it exercises
  `Config::epoch_interruption`, `Engine::increment_epoch`, `Store::set_epoch_deadline`,
  `Module::new`-with-WAT, and `TypedFunc::call` trapping in one test.
- `epoch_watchdog_stops_on_drop` — watchdog thread join.

Then the app build, since `src-tauri/Cargo.lock` is a separate resolution:

```bash
npm run tauri dev
```

Load a `.wasm` plugin and confirm `on_load` / `on_enable` dispatch, a `log_info` line lands in
`<app_data>/logs/plugin-<id>.log`, and `storage_get` / `storage_set` round-trip. There is no WASM
plugin fixture in-repo, so this step needs a hand-built artifact — if none is available, say so
explicitly rather than claiming the runtime was exercised end to end.

### 3.5 Commit both lockfiles

`Cargo.lock` and `src-tauri/Cargo.lock` both move. Review the diff for accidental major bumps of
unrelated shared crates (`object`, `gimli`, `indexmap`, `hashbrown`) pulled forward by wasmtime.

## 4. Docs to update (per CLAUDE.md instruction 1)

- **`CLAUDE.md` invariant 7** — change "wasmtime 23" to "wasmtime 48" in the plugin-runtime
  sentence. Re-word the WASI parenthetical so it states our design choice (plugins talk only to
  `algomln::*`; WASI is not linked) rather than pinning the rationale to a `WasiCtx` Send/Sync
  detail that may already be stale.
- **`BACKEND.md`** — same version reference in the plugin-runtime narrative.
- **`ARCHITECTURE.md`** — only if it names the version in a lookup row.
- **`wasm_runtime.rs` module header (lines 10-15, 155-159)** — two separate copies of the
  "WASI is not linked because wasmtime 23's `WasiCtx`…" rationale. Both need the same treatment.
- **`README.md`** — only if it lists the wasmtime version.

## 5. Optional follow-up: trim default features

Not part of the upgrade; a separate commit if compile time or binary size regresses.

```toml
wasmtime = { version = "48", default-features = false, features = [
  "runtime", "cranelift", "std", "wat", "addr2line", "demangle", "parallel-compilation",
] }
```

Drops component-model(+async), all four `gc-*`, stack-switching, coredump, profiling,
debug-builtins, wit-parser, cache, pooling-allocator — none of which the `algomln::*` host
surface uses. Must be validated by the same test run as §3.4; `wat` is required by the two tests
and `cranelift` by `Module::new` at runtime.

## 6. Rollback

Single-commit revert. The upgrade touches one source file, one manifest, and two lockfiles;
nothing in the plugin ABI (`_algomln_on_*` exports, `algomln::*` imports) changes, so existing
`.wasm` plugin artifacts stay compatible either way.
