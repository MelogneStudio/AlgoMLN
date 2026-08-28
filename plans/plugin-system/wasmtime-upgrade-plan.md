# Plan: wasmtime 23 → 48 (chunked)

Status: proposed (not started)
Parent plan: [wasmtime-23-to-48-upgrade.md](wasmtime-23-to-48-upgrade.md) (single-pass version)
Author context: written 2026-08-27 on branch `fix/b3-wasm-cpu-cap`
Latest published wasmtime at time of writing: **48.0.1** (2026-08-25).

## Why chunk it

The parent plan covers the whole upgrade in one mental pass: bump, fix, verify, document. That works as a thought experiment but produces one large commit where the `table_growing` signature fix and six markdown edits are buried in the same review surface. The chunks below split the work so each one is a single focused commit with a clean diff, an explicit test command, and a list of load-bearing tests that must stay green.

The two B3 epoch-watchdog tests are the tripwire for the whole upgrade — if `epoch_watchdog_traps_infinite_loop` or `epoch_watchdog_stops_on_drop` go red, the upgrade is wrong regardless of how clean the rest looks.

## Execution order

```
Chunk 1 (drop WASI) ─┐
                     ├─→ Chunk 2 (bump + fix) ─→ Chunk 3 (smoke) ─→ Chunk 4 (docs)
                     │                                                       │
                     └────────────────────────────────────── optional ┘     └─→ Chunk 5 (trim)
```

Chunks 1 and 2 could be combined into a single "bump + cleanup" commit, but splitting them keeps the upgrade diff small enough to review in one sitting. Chunk 5 is independent of all others and can land any time after Chunk 2.

---

## Chunk 1 — Drop `wasmtime-wasi` (preparation)

**Why first**: `wasmtime-wasi` is a no-op dep today — `grep -r wasmtime_wasi src src-tauri` returns nothing. Removing it before the version bump keeps the upgrade commit focused on the wasmtime change, not a dead-dep removal.

### Files
- `Cargo.toml:30` — remove `wasmtime-wasi = "23"`
- `Cargo.lock` — auto-removed by `cargo update -p wasmtime-wasi` (or by the next build)
- `src-tauri/Cargo.lock` — same

### Diff
```diff
-wasmtime = "23"
-wasmtime-wasi = "23"
+wasmtime = "23"
```

### Test plan
1. **No in-source use**
   ```bash
   git grep -nE 'wasmtime_wasi' src src-tauri
   ```
   Expected: no output. If anything hits, abort and investigate — the parent plan's assumption that the crate is unused is wrong.

2. **Compiles**
   ```bash
   cargo check --workspace --all-targets
   ```
   Must succeed. Failure here means a transitive re-export we missed.

3. **Full test suite is green**
   ```bash
   cargo test --workspace
   ```
   This is the safety net — the change should be a no-op for behavior, and any test that flips is a signal that the dep was load-bearing in a way the parent plan didn't anticipate.

### Risk
Very low. Dead-dep removal; no semantic change.

### Rollback
Single-commit revert — re-adds the line and restores the lockfile entry.

---

## Chunk 2 — Bump wasmtime to 48 and fix `table_growing`

**Why this commit**: the actual upgrade. One `Cargo.toml` line, one trait signature, one lockfile pair, and the doc comments that no longer describe the new API correctly.

### Files
- `Cargo.toml:29` — `wasmtime = "23"` → `wasmtime = "48"`
- `Cargo.lock` — auto
- `src-tauri/Cargo.lock` — auto
- `src/plugin/runtime/wasm_runtime.rs`:
  - `MemoryLimitState::table_growing` (lines 73-82) — change `(current: u32, desired: u32, maximum: Option<u32>)` to `(current: usize, desired: usize, maximum: Option<usize>)`; the `desired <= 10_000` bound check becomes a `usize` literal.
  - `MemoryLimitState` doc comment (lines 56-58) — rewrite to say both callbacks take `usize`; clarify that `memory_growing` arguments are page-aligned byte counts, `table_growing` arguments are element counts.
  - Module header (lines 10-15, 155-159) — drop the "WASI is not linked because `WasiCtx` in wasmtime 23 is `Send`-only" rationale. Replace with the design choice: plugins talk only to the `algomln::*` host surface, WASI is not linked. The Send/Sync reason was a 23-specific fact; in 48 `wasmtime_wasi` was reworked (`WasiCtxView`, p2/p3 split) and the original premise may no longer hold. Do not assert a new version-specific premise in its place.

### Diff (`wasm_runtime.rs` only)
```diff
-    fn table_growing(&mut self, current: u32, desired: u32, maximum: Option<u32>) -> Result<bool> {
+    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> Result<bool> {
         // 10,000 table elements is a generous upper bound for our host fn surface.
-        let limit_reached = desired > 10_000;
+        let limit_reached = desired > 10_000usize;
         ...
     }
```

### Test plan
1. **Compiler as oracle (parent plan §3.2)**
   ```bash
   cargo check --workspace --all-targets
   ```
   Expected: exactly one breakage (`table_growing`). Anything beyond that is a §2.3 risk signal from the parent plan (dependency-graph churn, `OptLevel` variants, default-feature surprise) and should be fixed in this same commit rather than worked around.

2. **Full test suite — must pass**
   ```bash
   cargo test --workspace
   ```

3. **Load-bearing B3 guards — must stay green**
   These two tests are the upgrade's proof that the CPU cap still works. If either flips red, do not merge.
   - `epoch_watchdog_traps_infinite_loop` — `src/plugin/runtime/wasm_runtime.rs::tests`
     Exercises `Config::epoch_interruption`, `Engine::increment_epoch`, `Store::set_epoch_deadline`, `Module::new`-with-WAT, and `TypedFunc::call` trapping. If this passes under 48, the entire CPU-cap mechanism is still intact.
   - `epoch_watchdog_stops_on_drop` — proves the watchdog thread still joins cleanly on plugin unload. A regression here is a leak in the runtime.

4. **Broader plugin tests**
   - All test modules in `src/plugin/runtime/wasm_runtime.rs::tests` (load, instantiate, host fn surface, error paths).
   - `src/plugin/registry.rs::tests` — the registry still drives `on_load` / `on_enable` correctly.
   - `src/plugin/loader.rs::tests` — the loader still dispatches `.wasm` files to `WasmPlugin`.

5. **Lockfile diff review**
   ```bash
   git diff Cargo.lock src-tauri/Cargo.lock | head -400
   ```
   Look for accidental major bumps of unrelated shared crates (`object`, `gimli`, `indexmap`, `hashbrown`) pulled forward by wasmtime 48's transitive deps. If anything suspicious jumps, call it out in the PR description rather than silently shipping it.

### Risk
Low. One known signature change, compiler-guided for anything else.

### Rollback
Single-commit revert. The plugin ABI (`_algomln_on_*` exports, `algomln::*` imports) is unchanged, so any existing `.wasm` plugin artifacts stay compatible either way.

---

## Chunk 3 — App-level smoke test (verification only)

**Why separate**: `src-tauri/Cargo.lock` resolves independently of `Cargo.lock`; `cargo test` does not exercise the Tauri resolution path. The parent plan's §3.4 step calls this out explicitly. This chunk is "did the upgrade actually work in the app" — not a code change.

### Files
None (verification only).

Optionally: a hand-rolled `.wasm` plugin fixture checked in under `src-tauri/resources/test-fixtures/` if we want to make this step reproducible rather than dev-by-dev manual.

### Test plan
Manual smoke test under `npm run tauri dev`:

1. Build a `.wasm` plugin artifact that exports `_algomln_on_load` and calls `algomln_log_info` once. (No fixture in-repo today — see parent plan §3.4 caveat.)
2. Drop it into the plugin dir, enable it from the Settings screen.
3. Confirm:
   - The plugin's status goes `Loaded` → `Enabled` (proves `on_load` / `on_enable` dispatch).
   - A line lands in `<app_data>/logs/plugin-<id>.log` (proves the LogApi path; also exercises the rate-limiter and 5MB rolling-file logic from invariant 7a).
   - `storage_get` / `storage_set` round-trip works (proves the KV path).

If no `.wasm` fixture is available at the time of the upgrade, do not claim the runtime was exercised end-to-end — say so explicitly in the commit message and track fixture creation as follow-up.

### Risk
Low. Manual-only, no source change.

### Rollback
N/A.

---

## Chunk 4 — Documentation updates (per CLAUDE.md instruction 1)

**Why separate**: per the project rule, every edit ships with its docs in the same commit, but the doc updates are spread across five files and warrant a focused review pass. Co-locating them in one chunk means the upgrade commit (Chunk 2) doesn't have to drag markdown edits through review.

### Files
- **`CLAUDE.md` invariant 7** — change "wasmtime 23" → "wasmtime 48" in the plugin-runtime sentence. Reword the WASI parenthetical from "wasmtime 23's `WasiCtx` is `Send`-only and would violate the `Plugin: Send + Sync` bound" to "WASI is intentionally not linked — plugins talk only to the `algomln::*` host surface."
- **`BACKEND.md`** — same version reference in the plugin-runtime narrative.
- **`ARCHITECTURE.md`** — only if it names a wasmtime version in a lookup row. Open and grep first; skip if absent.
- **`README.md`** — only if it lists the wasmtime version. Open and grep first; skip if absent.
- **`plans/plugin-system/wasmtime-23-to-48-upgrade.md`** — once Chunks 1–3 land, prepend a "Status: superseded 2026-MM-DD by `wasmtime-upgrade-plan.md`" line. Keep the file as historical reference; do not delete it.

### Test plan
1. **No stale version references**
   ```bash
   git grep -nE 'wasmtime[- ]?2[3]' -- '*.md' '*.rs' ':!plans/plugin-system/wasmtime-23-to-48-upgrade.md'
   ```
   Expected: no output. The exception clause is for the parent plan file, which legitimately documents the 23 baseline.

2. **No version-specific WASI rationale**
   ```bash
   git grep -nE 'WasiCtx.*Send'
   ```
   Expected: no output. The Send/Sync reason was a 23-specific fact; the new docs should describe the design choice, not a wasmtime version detail.

3. **Sanity re-run of the test suite**
   ```bash
   cargo test --workspace
   ```
   Catches the edge case where a doc edit accidentally lands in a code block (markdown files can carry fenced Rust).

### Risk
Very low. Doc-only.

### Rollback
Per-file `git checkout` of the affected markdown files.

---

## Chunk 5 (optional) — Trim wasmtime default features

**Why optional**: 48's default feature set is large (28 features: component-model, component-model-async, gc-*, stack-switching, pooling-allocator, parallel-compilation, profiling, coredump, debug-builtins, wit-parser, …). We use none of the component model, GC, or async features. Trimming should reduce compile time and binary size. Skip if compile time is acceptable as-is — this is a polish commit, not a correctness one.

### Files
- `Cargo.toml:29` — switch to `default-features = false` with an explicit feature list.

### Diff
```diff
-wasmtime = "48"
+wasmtime = { version = "48", default-features = false, features = [
+  "runtime", "cranelift", "std", "wat", "addr2line", "demangle", "parallel-compilation",
+] }
```

The `wat` feature is required by the two `.wat` test modules; `cranelift` is required by `Module::new` at runtime; `std` is required by the `Engine::new` config path. If a test fails after the trim, the failure tells us which feature to add back.

### Test plan
1. **Compiles**
   ```bash
   cargo check --workspace --all-targets
   ```

2. **Full test suite — must pass**
   ```bash
   cargo test --workspace
   ```
   Load-bearing:
   - `epoch_watchdog_traps_infinite_loop` — proves `cranelift` and `wat` are still wired in.
   - `epoch_watchdog_stops_on_drop` — proves the watchdog is unaffected.

3. **(Optional) Binary size sanity check**
   ```bash
   cargo bloat --release --bin algomln --crates wasmtime
   ```
   Or compare clean `cargo build --release` wall time before and after. If no measurable improvement, revert the trim — the optional status means "only land if it actually helps."

### Risk
Medium. Easy to drop a feature we implicitly need. The two B3 tests catch the obvious misses; runtime smoke (Chunk 3) catches anything that compiles but breaks at instantiation.

### Rollback
Single-line revert in `Cargo.toml`.

---

## Verification matrix

| Check | Required by |
| --- | --- |
| `git grep wasmtime_wasi` returns nothing | Chunk 1 |
| `cargo check --workspace --all-targets` | Chunks 1, 2, 5 |
| `cargo test --workspace` (full) | Chunks 1, 2, 4, 5 |
| `epoch_watchdog_traps_infinite_loop` | Chunks 2, 5 |
| `epoch_watchdog_stops_on_drop` | Chunks 2, 5 |
| `src/plugin/registry.rs::tests` + `src/plugin/loader.rs::tests` | Chunk 2 |
| Manual `npm run tauri dev` plugin load smoke | Chunk 3 |
| `git grep wasmtime 23` returns nothing (excluding history) | Chunk 4 |
| `git grep WasiCtx.*Send` returns nothing | Chunk 4 |
| (Optional) `cargo bloat` size comparison | Chunk 5 |

## Out of scope

- Linking `wasmtime-wasi` to expose a filesystem or process surface to plugins. Plugins talk only to `algomln::*`; if WASI is wanted later it comes back as a separate, scoped piece of work.
- The `instances()` / `tables()` / `memories()` caps that the new `ResourceLimiter` provides via default methods. The 10,000-element table cap is already a reasonable bound; tightening the per-instance or per-table memory cap is a separate hardening pass.
- Migrating to wasmtime's component model or any of the GC features. We use the core module model; component-model is dead weight in our binary.
