# Rhai Plugin Callback Architecture

## Implementation Plan and Coding Instructions

You are implementing a narrowly scoped change to the AlgoMLN Rust plugin system.

This document is the authoritative implementation specification.

The decisions below are intentional architectural decisions, not suggestions. Preserve them unless the existing code creates a direct contradiction. Where the existing implementation differs from the terminology or pseudocode below, map the design onto the actual codebase rather than blindly introducing duplicate abstractions.

---

# 1. Mission

Implement the new/fixed Rhai callback architecture for:

- `register_metric`
- `register_keyword`
- `schedule`
- `subscribe_event`

The architecture must guarantee that these callback registrations are:

1. **Stateless by design**
2. Based on **plain named Rhai functions**
3. Invoked with a **fresh Rhai execution scope per invocation**
4. Persisted across calls only through the existing plugin storage API
5. Owned by the `PluginHost` belonging to one specific plugin load
6. Never allowed to survive a plugin reload while referencing the previous load's Engine/AST/scope
7. Safe to invoke concurrently against the same `Arc<Engine>` when Rhai is built with `sync`
8. Dispatched from Tokio's synchronous callback slots using `block_in_place`
9. Explicitly isolated from the existing indicator/analytics callback architecture

Do not redesign unrelated plugin functionality.

---

# 2. FIRST STEP — REQUIRED CODEBASE RECONNAISSANCE

Before modifying any code, inspect the actual repository.

Do not start implementing the callback registry until this inspection is complete.

## 2.1 Verify the Rhai dependency

Confirm the actual `Cargo.toml` contains:

```toml
rhai = { version = "1", features = ["sync", "no_closure"] }

```

The existing architecture assumes both:

- `sync`
- `no_closure`

are enabled.

If this is not true in the actual checkout, determine why before proceeding.

Do not silently implement an architecture that depends on `Send + Sync` Rhai values without verifying the feature configuration.

---

# 3. CRITICAL STOP-AND-REPORT STEP: INSPECT `scope`

The existing `RhaiPlugin` contains:

```rust
scope: Option<Scope<'static>>

```

This is an unresolved architectural fact.

Do NOT guess what it means.

Before making any changes:

1. Find every read of `self.scope`.
2. Find every write to `self.scope`.
3. Find every place the contained `Scope` is passed to `Engine::call_fn`, `eval_ast_with_scope`, or equivalent Rhai APIs.
4. Find every place the scope is mutated between lifecycle/indicator invocations.
5. Determine whether the same Scope instance is reused across multiple invocations.

Then classify the existing behavior as:

### A

The Scope is genuinely shared/reused across calls and script state persists between invocations.

### B

The Scope is only used for one-time/top-level initialization/lifecycle execution and is not reused as persistent state across callback/indicator invocations.

### C

Some other behavior exists.

Report which case is true before proceeding.

## HARD STOP

If the result is **A**, STOP implementation and report the finding.

Do not decide independently how to reconcile shared scope semantics with this plan.

This plan assumes B.

The new callback architecture itself is still required to be stateless regardless of the existing scope behavior, but if existing `scope` usage turns out to be shared mutable state, the broader architecture requires human review before implementation continues.

If the result is B, continue.

---

# 4. Existing Architecture to Preserve

The actual codebase currently has approximately this architecture:

```text
AlgoMLN
└── src/plugin
    ├── api/
    ├── host.rs
    ├── registry.rs
    └── runtime/
        ├── rhai_runtime.rs
        └── wasm_runtime.rs

```

The relevant existing types include:

```text
RhaiPlugin
PluginHost
PluginRegistry
PluginEntry
PluginId
PluginError
PluginResult
CronScheduler
ScheduleHandle
EventBus
EventFilter
EventKind
SharedIndicatorRegistry
SharedAnalyticsRegistry

```

Do not assume these exact definitions without inspecting the actual files.

Use the repository's actual definitions and names as the source of truth.

---

# 5. NON-GOALS

This task is NOT a general plugin-system rewrite.

Explicitly do NOT:

- redesign the plugin API
- redesign plugin manifests
- redesign capability handling
- redesign storage
- redesign broker APIs
- redesign `SharedIndicatorRegistry`
- redesign `SharedAnalyticsRegistry`
- migrate indicators onto the new callback registry
- migrate analytics onto the new callback registry
- replace `CronScheduler` with a new scheduler
- replace `EventBus` with a new event system
- change scheduler callback signatures to async futures
- change event callback signatures to async futures
- introduce persistent Rhai callback Scopes
- introduce closure-based callback state
- change existing script-facing registration syntax
- make registration capabilities hard-error merely for consistency with execution capabilities
- make unrelated public API changes

Only directly related internal code may be refactored when necessary.

---

# 6. CORE OWNERSHIP MODEL

The most important architectural invariant is:

> One plugin load owns one complete callback/runtime state.

`PluginHost` is the ownership boundary for this state.

There should NOT be both:

```text
PluginHost
+
LoadedRhai

```

as two competing ownership abstractions.

`PluginHost` fills the role of the per-load bundle.

Conceptually, the per-load state must remain associated:

```text
PluginHost/load N
├── Engine N
├── AST N
├── callback registry N
└── Engine lifetime mechanism N

```

Never produce:

```text
PluginHost N
├── Engine N
└── callbacks from load N-1

```

or:

```text
PluginHost N
├── Engine N
└── callbacks pointing at Engine N-1

```

That is a correctness bug.

---

# 7. CALLBACK REGISTRY

Add one callback registry associated with `PluginHost`.

Conceptually:

```rust
#[derive(Default)]
pub(crate) struct PluginCallbacks {
    metrics: Mutex<HashMap<String, rhai::FnPtr>>,
    keywords: Mutex<HashMap<String, rhai::FnPtr>>,
    schedules: Mutex<HashMap<ScheduleHandle, rhai::FnPtr>>,
    events: Mutex<HashMap<EventKind, Vec<rhai::FnPtr>>>,
}

```

Adapt exact visibility and key types to the existing code.

Do not blindly use `ScheduleId` if the actual codebase uses `ScheduleHandle`.

The existing decision is to preserve the codebase's `ScheduleHandle` terminology.

## Required registry properties

There is:

- one registry
- four callback categories
- one lifecycle owner

Do not create four separately owned registries.

The registry should be behind the appropriate shared ownership mechanism, conceptually:

```rust
Arc<PluginCallbacks>

```

The exact synchronization primitive may follow project conventions, but concurrent registration/dispatch must be safe.

---

# 8. REQUIRED DOCUMENTATION ON `PluginCallbacks`

The struct itself must contain a Rust doc comment documenting the stateless callback contract.

The documentation should clearly state that:

- callbacks are plain functions, not stateful closures
- callback invocation gets a fresh execution scope
- callbacks must not rely on Rhai local variables persisting between invocations
- persistent state belongs in `storage_get` / `storage_set`

This documentation belongs on the code structure itself.

Do not rely solely on this implementation document to communicate the invariant.

---

# 9. CALLBACK REGISTRATION

The four Rhai-facing registration APIs remain:

```text
register_metric(name, callback)
register_keyword(name, callback)
schedule(...)
subscribe_event(...)

```

Preserve the existing script-facing signatures.

Only additive/internal implementation changes are allowed.

Registration must insert into the callback registry associated with the current `PluginHost`.

The registration functions must NOT directly manipulate the Engine.

They should not mutate some unrelated global callback state.

---

# 10. PLUGIN-SCOPED NAMES

Callback names are plugin-scoped.

For example:

```text
plugin-A -> metric "foo"
plugin-B -> metric "foo"

```

is valid.

The callback registry belongs to one plugin's `PluginHost`, so a string name is sufficient within that registry.

Do not introduce a global compound key unless the actual architecture makes it unavoidable.

---

# 11. CLOSURE POLICY

The callback policy is:

> Named Rhai functions only.

However, understand how enforcement actually works.

The Rhai dependency already enables:

```toml
features = ["sync", "no_closure"]

```

`no_closure` is the primary enforcement mechanism.

A capturing closure such as:

```rhai
let x = 42;

register_metric("foo", |trades| {
    x
});

```

should fail during Rhai parsing/compilation rather than becoming a valid callback registration.

This means the registration-time `FnPtr` check is defense-in-depth, not the primary safety guarantee.

---

# 12. REGISTRATION-TIME `FnPtr` CHECK

For the four new registration functions:

- `register_metric`
- `register_keyword`
- `schedule`
- `subscribe_event`

perform the appropriate `FnPtr` plain-function validation where technically supported by the actual locked Rhai API.

The implementation must inspect the actual Rhai API/version rather than assume a method's exact semantics.

`FnPtr::is_curried()` may be used as the defense-in-depth check if it is applicable to the actual Rhai version/API.

Do NOT retrofit this validation into unrelated existing APIs such as `register_indicator` merely because it appears similar.

If an invalid callback is detected:

```text
PluginError::ApiError(...)

```

must be used.

The error should tell the plugin author that callbacks must be plain named functions rather than closures and that persistent state should use storage.

The exact string can follow existing project conventions.

---

# 13. DO NOT BUILD ELABORATE SIGNATURE VALIDATION

Rhai is dynamically typed.

Do not build a complicated callback-signature reflection/validation framework.

Where the Rhai API makes cheap validation practical, use it.

For example, if callback arity is directly and reliably available in the actual version, it may be validated.

If it is not cheaply available:

- do not invent reflection machinery
- let `FnPtr::call` report invalid invocation arguments

A bad invocation must become the normal callback execution error path.

If a cheap registration-time validation fails, reject that registration only.

Do NOT make an otherwise valid plugin load fail merely because one optional callback registration has an invalid signature.

---

# 14. CALLBACK INVOCATION

Use:

```rust
FnPtr::call(...)

```

for callback invocation.

Do not replace the new callback invocation path with direct:

```rust
Engine::call_fn(...)

```

The reason is architectural:

`FnPtr::call` provides the fresh invocation scope needed for the stateless callback contract.

Do not manually retain a callback Scope between invocations.

Do not add a callback-specific persistent `Scope`.

The callback lifecycle is:

```text
FnPtr
  ↓
FnPtr::call
  ↓
fresh invocation scope
  ↓
Rhai function execution
  ↓
scope discarded

```

Persistent state:

```text
Rhai callback
  ↓
storage_get / storage_set
  ↓
plugin storage

```

---

# 15. ENGINE / AST LIFETIME

The callback registry must retain enough per-load state to invoke the callback against the correct Engine and AST.

The existing architecture already has:

```rust
Arc<Engine>
Arc<AST>
Arc<OnceLock<Weak<Engine>>>

```

or equivalent.

Preserve the `Weak<Engine>` lifetime mechanism where appropriate.

Conceptually:

```rust
let engine = engine_cell
    .get()
    .and_then(|w| w.upgrade())
    .ok_or(PluginError::Unloaded)?;

```

If the Engine can no longer be upgraded, the callback belongs to an unloaded/dead plugin load.

Do not substitute the new active plugin's Engine.

That would violate reload isolation.

---

# 16. RELOAD ISOLATION

Reload is the central lifetime invariant.

Reload must create a completely new per-load `PluginHost`.

Conceptually:

```text
OLD LOAD
PluginHost A
├── Engine A
├── AST A
├── callbacks A
└── schedules/events A

        ↓ unload

OLD HOST TORN DOWN
├── schedules cancelled
├── event subscriptions removed
└── old owned state eventually dropped

        ↓

NEW LOAD
PluginHost B
├── Engine B
├── AST B
├── callbacks B
└── schedules/events B

```

Do NOT mutate an existing `Arc<PluginHost>` by swapping only its Engine.

Do NOT retain the old callback registry while replacing the Engine.

Do NOT construct a new Engine and attach the old registry.

The entire ownership unit must be replaced.

---

# 17. RELOAD ORDER

The desired lifecycle is:

1. Tear down the old plugin host's externally registered callback resources.
2. Cancel its schedules.
3. Remove its event subscriptions.
4. Remove/release its registrations as appropriate.
5. Drop the old `PluginHost` ownership.
6. Construct the new plugin host.
7. Construct the new Engine/AST/runtime state.
8. Run the new plugin's load path.
9. Make the new load active.

Use the actual `PluginRegistry` lifecycle mechanisms where possible.

The current registry uses plugin entries and schedule handles; integrate with those rather than creating a second lifecycle system.

---

# 18. OLD CALLBACKS AFTER UNLOAD

Existing callback objects may temporarily remain alive because something such as an in-flight task owns them.

That is acceptable.

What is NOT acceptable is allowing them to acquire the new Engine.

Expected behavior:

```text
old callback
    ↓
Weak<Engine>
    ↓
upgrade succeeds while old Engine is still alive
    ↓
finish safely against old Engine

```

or:

```text
old callback
    ↓
Weak<Engine>
    ↓
upgrade fails
    ↓
PluginError::NotFound / Unloaded-equivalent
    ↓
no panic

```

Never:

```text
old callback
    ↓
find active plugin
    ↓
use new Engine

```

---

# 19. SCHEDULE LIFECYCLE

The host generates the `ScheduleHandle`.

The handle is returned to the Rhai plugin in the existing representation expected by the API, such as a `Dynamic`/string if that is what the existing interface uses.

Do not replace the existing `ScheduleHandle` naming.

Schedules belong to the `PluginHost` load that registered them.

On unload/reload:

```text
all old schedule tasks are cancelled

```

Do not leave old schedules running indefinitely.

Do not silently rebind an old schedule to the new plugin load.

If the new plugin registers a schedule with the same logical name/configuration, it is a new registration belonging to the new load.

---

# 20. EVENT SUBSCRIPTIONS

Event subscriptions are also load-owned.

On plugin unload/reload:

```text
old PluginHost
    ↓
unsubscribe old EventBus registrations

```

Do not leave old subscriptions in the EventBus and rely on their Weak Engine failing later.

The lifecycle must actively remove them.

Use the existing `EventFilter` and EventBus mechanisms.

Do not redesign EventBus.

---

# 21. METRICS AND KEYWORDS

Metrics and keywords are plugin-scoped callback registrations.

They should be stored in the per-load callback registry.

Their callback failures should be observable to the immediate caller where the existing invocation API permits it.

Do not add automatic failure-based disabling for metrics or keywords.

The caller already has a synchronous result path and should receive an `ApiError` when applicable.

---

# 22. FAILURE POLICY

All callback execution failures must be logged where the execution context permits logging.

Logs should contain at minimum:

- Plugin ID
- callback identity/name
- useful error information

Use the project's existing logging abstraction.

Do not invent a second logging framework.

The existing project attributes plugin logs to `PluginId`.

---

# 23. SCHEDULE FAILURE COUNTER

Only scheduled callbacks receive consecutive-failure auto-disable behavior.

Do NOT apply this automatic disabling policy to:

- metrics
- keywords
- events

Reason:

- events are externally triggered and can burst
- metrics/keywords have immediate callers
- automatically disabling those callback types would alter caller behavior in undesirable ways

---

# 24. FAILURE COUNTER LOCATION

Failure state belongs in scheduler state, not in `PluginCallbacks`.

The callback registry answers:

> What callback is registered?

The scheduler answers:

> Is this scheduled task healthy?

The scheduler already owns per-task lifecycle through its cancellation token and schedule handle, so failure state belongs there.

Conceptually:

```text
ScheduleHandle
    ↓
scheduler task state
    ├── CancellationToken
    ├── callback
    └── consecutive_failures

```

Use the actual scheduler structures rather than blindly copying this representation.

---

# 25. FAILURE COUNTER SEMANTICS

Threshold:

```text
3 consecutive failures

```

On success:

```text
failure_count = 0

```

Example:

```text
failure → 1
failure → 2
success  → 0
failure → 1

```

When the callback reaches 3 consecutive failures:

```text
schedule becomes permanently disabled

```

It remains disabled until the plugin is reloaded.

Do not automatically re-enable it after a timeout.

---

# 26. SCHEDULE ERROR BOUNDARY

The current scheduler callback type is synchronous:

```rust
Arc<dyn Fn() + Send + Sync>

```

There is no `Result` in this callback signature.

Therefore, scheduler callback failures cannot be returned through the scheduler callback type.

At this boundary:

```text
callback execution
    ↓
error
    ↓
log
    ↓
increment failure counter
    ↓
possibly disable schedule

```

Do not invent a fake `Result` propagation mechanism through `Fn()`.

---

# 27. EVENT ERROR BOUNDARY

The EventBus callback currently has a synchronous shape equivalent to:

```rust
Arc<dyn Fn(EventKind) + Send + Sync>

```

It likewise cannot return a `Result`.

Therefore:

```text
FnPtr::call failure
    ↓
log
    ↓
swallow at EventBus callback boundary

```

Do not change EventBus's public callback signature as part of this task.

The callback registry/invocation implementation may internally produce an error, but the existing EventBus boundary ultimately logs and consumes it.

---

# 28. ASYNC → RHai DISPATCH

This is an important implementation constraint.

The existing scheduler and EventBus callback signatures are synchronous closures invoked from Tokio tasks.

Therefore the required bridge is:

```rust
tokio::task::block_in_place(|| {
    fn_ptr.call(...)
})

```

NOT:

```rust
spawn_blocking(...)

```

for these existing callback slots.

Why:

- the scheduler callback is `Fn()`
- the EventBus callback is `Fn(EventKind)`
- those closures cannot `.await`
- changing them to futures would be a larger API refactor

That larger refactor is explicitly out of scope.

---

# 29. `block_in_place` RULE

Use `block_in_place` when the synchronous Rhai callback is being invoked from an existing Tokio worker/task context where the callback would otherwise execute synchronously on the async worker.

Conceptually:

```rust
tokio::task::block_in_place(|| {
    fn_ptr.call::<Dynamic>(&engine, &ast, args)
})

```

Adapt the exact return type and argument representation to the actual callback.

Do not call `block_in_place` when the caller is already a normal synchronous context outside Tokio.

In a non-Tokio synchronous context:

```rust
fn_ptr.call(...)

```

directly.

`block_in_place` outside an appropriate Tokio runtime context is invalid and must not be introduced indiscriminately.

---

# 30. METRIC / KEYWORD DISPATCH

Inspect the actual invocation call sites.

If a metric or keyword callback is already invoked inside a Tokio task, apply the same `block_in_place` boundary.

Do not assume this blindly.

If it is invoked from a synchronous context, call the Rhai function directly.

The implementation must determine this from the actual call site.

---

# 31. DO NOT CHANGE SCHEDULER/EVENT CALLBACK TYPES

Explicitly out of scope:

```text
Fn()

```

→

```text
async Fn()

```

or:

```text
Fn() -> BoxFuture<...>

```

or any equivalent future-based redesign.

Likewise for EventBus.

That could be a future architectural improvement but is NOT part of this task.

---

# 32. CAPABILITY DENIAL

The four registration capabilities are intentionally soft-fail:

- `register_metric`
- `register_keyword`
- `schedule`
- `subscribe_event`

This is deliberate.

Execution capabilities such as order placement are safety-critical.

Registration capabilities are not.

Do NOT "fix" this asymmetry merely to make the APIs look consistent.

When capability permission is denied:

1. Preserve the existing soft-fallback behavior.
2. Log the denial.
3. Do not convert it into a hard plugin-load failure unless the existing API already requires that behavior.

Add a concise code comment explaining that this asymmetry is intentional.

A future maintainer must not mistake it for an accidental inconsistency.

---

# 33. EXISTING INDICATOR / ANALYTICS ARCHITECTURE

Do not migrate:

```text
SharedIndicatorRegistry
SharedAnalyticsRegistry

```

to `PluginCallbacks`.

Do not refactor their callback architecture as part of this task.

They are explicitly out of scope.

If their implementation looks similar, resist the temptation to "clean everything up."

Only modify them if a direct dependency makes a minimal related change unavoidable.

---

# 34. PUBLIC API RULE

Preserve existing public APIs.

Allowed:

- internal/private struct changes
- internal helper functions
- internal ownership changes
- internal callback storage changes
- internal reload implementation changes

Not allowed without stopping:

- changing `PluginRegistry` public signatures
- changing `RhaiPlugin` public API
- changing script-facing callback syntax
- changing public scheduler API
- changing public EventBus API

If the implementation genuinely requires a public API change, STOP and report the contradiction rather than silently changing it.

---

# 35. ERROR TYPES

The existing project has:

```text
PluginError
PluginResult<T>

```

with an existing:

```text
ApiError(String)

```

Use `ApiError` for callback execution/registration errors.

Do NOT invent:

```text
ScriptError

```

if it does not exist.

For example:

```rust
PluginError::ApiError(error.to_string())

```

is the appropriate conceptual conversion.

Use existing Rhai → `EvalAltResult` conversion conventions for errors crossing from Rust host functions into Rhai.

---

# 36. ENGINE CONCURRENCY

The Rhai Engine is configured with:

```text
sync

```

Therefore the intended architecture permits concurrent use of the shared Engine.

The implementation must support:

```text
Arc<Engine>
      │
      ├── callback A
      │
      └── callback B

```

executing concurrently.

Do not introduce a global mutex around all Rhai execution merely because it is easier.

Such a mutex would undermine the concurrency model this task is explicitly intended to establish.

Only add synchronization where it is actually required for mutable shared Rust state.

---

# 37. CALLBACK STATE MODEL

A callback must NOT capture state.

The intended model is:

```rhai
fn my_metric(trades) {
    let local = ...;
    ...
}

```

Each call gets fresh execution state.

If the plugin needs persistence:

```rhai
fn scheduled_task() {
    let value = storage_get("counter");
    ...
    storage_set("counter", ...);
}

```

Do not provide an alternative persistent callback-scope mechanism.

Do not add:

```text
callback_scope_cache

```

or equivalent.

---

# 38. TEST PLAN

Implement tests in this order.

## Test 1 — Named callback registration

Create a test plugin/script containing a plain named function.

Register it through the real Rhai-facing API.

Fire it through the actual callback dispatch path.

Assert:

- registration succeeds
- callback executes
- returned value is correct

Do not test only the registry in isolation.

Exercise the production registration/invocation path.

---

# 39. Test 2 — Closure rejection

Because `no_closure` is enabled, verify the actual behavior at script compilation/registration boundaries.

Use a script that attempts to use a capturing closure.

Expected behavior:

```text
script fails to compile/load

```

Also exercise the registration-level defense-in-depth check where the actual Rhai API permits construction of a curried/captured `FnPtr`.

Expected behavior:

```text
PluginError::ApiError(...)

```

Do not falsely claim that `is_curried()` is the primary enforcement mechanism.

The test suite should demonstrate that:

1. `no_closure` blocks capturing closure syntax.
2. registration still rejects an invalid/curried FnPtr if one reaches the registration API.

---

# 40. Test 3 — Concurrent callback execution

Create two different named Rhai callbacks.

Invoke them concurrently through the production-style Tokio boundary:

```text
tokio::spawn
    ↓
block_in_place
    ↓
FnPtr::call

```

Use the same:

```text
Arc<Engine>
Arc<AST>

```

where production code does so.

Assert:

- both callbacks complete
- return values are correct
- no panic
- no deadlock
- callback A cannot corrupt callback B
- shared Engine usage is safe

Do not add a separate artificial "AST concurrency" assertion; correct concurrent Engine execution and callback results exercise the same safety property.

---

# 41. Test 4 — Deterministic reload isolation

This test is critical.

Do NOT rely on arbitrary sleeps or timing races.

Use a synchronization primitive such as:

```text
channel
barrier
oneshot
Notify

```

or the project's existing test synchronization convention.

Desired sequence:

```text
Load 1
  ↓
register schedule/callback
  ↓
begin executing old callback
  ↓
pause deterministically inside test-controlled callback path
  ↓
reload plugin
  ↓
old PluginHost is torn down
  ↓
new PluginHost is constructed
  ↓
release old callback
  ↓
old callback completes against old Engine, if its Engine is still alive

```

Then verify:

- old callback never uses the new Engine
- old callback never uses the new AST
- old callback does not panic
- old schedule is cancelled
- old event subscription is removed
- new load owns only new registrations

If the old Engine is already dropped before an attempted old callback invocation:

```text
Weak::upgrade()

```

must fail cleanly.

Do not rely on sleeps to establish this ordering.

---

# 42. TEST 5 — Schedule failure disabling

Add a deterministic scheduler test that causes the callback to fail three consecutive times.

Expected:

```text
failure 1 → still active
failure 2 → still active
failure 3 → disabled

```

Then verify:

```text
success

```

resets the counter.

Also verify:

```text
failure
failure
success
failure

```

leaves the count at one, not three.

Verify that after auto-disable the schedule remains disabled until plugin reload.

---

# 43. TEST 6 — Capability soft failure

Verify that denied registration capabilities retain their existing soft-fallback semantics.

At minimum verify the four new registration capabilities.

Expected:

```text
capability denied
    ↓
log denial
    ↓
no hard plugin-load failure

```

Do not turn these into execution-style hard errors.

---

# 44. IMPLEMENTATION ORDER

Implement in this order.

## Phase 1 — Reconnaissance

1. Verify Cargo/Rhai features.
2. Inspect every `scope` field use.
3. Stop if scope behavior is case A.
4. Inspect actual `PluginHost`.
5. Inspect actual `RhaiPlugin`.
6. Inspect reload path.
7. Inspect scheduler.
8. Inspect EventBus.
9. Inspect all four registration paths.
10. Inspect all four invocation paths.

## Phase 2 — Ownership

11. Add `PluginCallbacks`.
12. Make it owned by `PluginHost`.
13. Ensure a new `PluginHost` is created per plugin load.
14. Ensure callback registry, Engine, AST, and lifetime mechanism belong to the same load.

## Phase 3 — Registration

15. Move/fix the four new callback registrations to use the per-load registry.
16. Preserve script-facing APIs.
17. Add closure defense-in-depth.
18. Preserve capability soft-failure.
19. Add intentional-asymmetry comment.

## Phase 4 — Invocation

20. Implement common internal callback invocation logic where useful.
21. Use `FnPtr::call`.
22. Resolve Engine through the correct per-load lifetime mechanism.
23. Convert callback errors to `PluginError::ApiError`.
24. Add `block_in_place` only at Tokio synchronous callback boundaries.

## Phase 5 — Lifecycle

25. Ensure schedules are cancelled during unload.
26. Ensure EventBus subscriptions are removed during unload.
27. Ensure old callback registries cannot attach to new Engine/AST state.
28. Ensure reload replaces the entire per-load ownership unit.

## Phase 6 — Failure policy

29. Add scheduler consecutive-failure state.
30. Reset on success.
31. Disable after three consecutive failures.
32. Keep disabled until reload.
33. Log every callback failure.

## Phase 7 — Tests

34. Named callback test.
35. Closure/no\_closure test.
36. Concurrent callback test.
37. Deterministic reload test.
38. Schedule failure test.
39. Capability soft-failure test.
40. Run the full relevant test suite.

---

# 45. IMPLEMENTATION STYLE

Prefer small internal helpers over duplicating Engine/AST/Weak upgrade/error conversion logic four times.

For example, an internal helper conceptually equivalent to:

```rust
fn call_callback(
    engine_cell: &OnceLock<Weak<Engine>>,
    ast: &AST,
    fn_ptr: &FnPtr,
    args: impl rhai::FuncArgs,
) -> PluginResult<Dynamic>

```

may be appropriate.

But do not introduce an abstraction solely because this document contains pseudocode.

Use the project's actual types and ownership structure.

The final code should look native to the existing codebase.

---

# 46. IMPORTANT: DO NOT COPY PSEUDOCODE BLINDLY

The snippets in this document describe semantics.

They are not guaranteed to match exact type names in the checkout.

For example:

```text
ScheduleId

```

should become the existing:

```text
ScheduleHandle

```

where appropriate.

Likewise, if synchronization currently uses `parking_lot`, follow existing conventions unless there is a concrete reason not to.

Use the repository's real definitions.

---

# 47. STOP-AND-ASK CONDITIONS

You have permission to resolve ordinary implementation details yourself.

If the code differs from this plan, make the smallest compatible change and document the deviation.

However, STOP and ask the human if:

1. `RhaiPlugin.scope` turns out to be genuinely reused/shared mutable state across calls.
2. A public API change is genuinely required.
3. The existing script-facing callback API cannot be preserved.
4. The existing plugin reload lifecycle fundamentally cannot provide per-load `PluginHost` ownership without changing public behavior.

Do NOT stop for minor naming/type differences.

Do NOT ask about things that can be determined by reading the actual Rhai API or repository.

---

# 48. ACCEPTANCE CRITERIA

The implementation is complete only if all of the following are true.

## Architecture

-  `PluginCallbacks` exists as one registry containing the four callback categories.
-  Registry belongs to `PluginHost`.
-  `PluginHost` is the per-load ownership boundary.
-  New load creates a new callback registry.
-  New load creates a new Engine/AST association.
-  Old and new load state cannot be accidentally mixed.

## Statelessness

-  New callbacks use `FnPtr::call`.
-  New callbacks receive fresh execution scope semantics.
-  No persistent callback Scope is introduced.
-  Persistent state uses storage.
-  `PluginCallbacks` documents this invariant.

## Closure safety

-  Rhai `no_closure` is confirmed.
-  Capturing closure syntax cannot compile.
-  Registration has appropriate defense-in-depth for invalid/curried FnPtrs.
-  Invalid registration uses `PluginError::ApiError`.

## Async boundary

-  Scheduler → Rhai uses `block_in_place` where invoked from Tokio.
-  EventBus → Rhai uses `block_in_place` where invoked from Tokio.
-  Metric/keyword invocation uses it only when the actual call site is inside Tokio.
-  Synchronous non-Tokio callers call Rhai directly.
-  `CronScheduler` and `EventBus` callback signatures were not redesigned.

## Reload

-  Old schedules are cancelled.
-  Old event subscriptions are removed.
-  Old callback objects cannot acquire the new Engine.
-  In-flight old callbacks can safely finish against the old Engine if it remains alive.
-  Dead old Engine produces a clean error rather than a panic.
-  No old callback/registry is attached to the new PluginHost.

## Failure policy

-  Every callback error is logged where applicable.
-  Logs contain plugin ID and callback identity.
-  Schedule failures are counted.
-  Counter resets on success.
-  Three consecutive failures disable the schedule.
-  Disabled schedules remain disabled until reload.
-  Metrics, keywords, and events are not auto-disabled.

## Capability behavior

-  Registration capability denial remains a soft failure.
-  Denial is logged.
-  It is not converted into a hard error merely for consistency with execution capabilities.

## Scope

-  Existing indicator/analytics registries were not migrated.
-  No unrelated plugin API was redesigned.
-  No unrelated public API was changed.
-  No scheduler/event architectural rewrite was introduced.

## Tests

-  Named callback works.
-  Closure/no\_closure behavior is tested.
-  Concurrent callback execution works.
-  No panic/deadlock occurs.
-  Return values remain correct.
-  Reload isolation is deterministic and tested.
-  Schedule failure threshold is tested.
-  Capability soft-failure is tested.
-  Relevant existing tests still pass.

---

# 49. FINAL HARD INVARIANT

The implementation must NEVER allow a plugin reload to leave any callback, schedule, or event subscription referencing an Engine, AST, or Scope from a previous load while being treated as part of the new load.

Every load is a distinct ownership generation.

Conceptually:

```text
LOAD N
PluginHost N
    ├── Engine N
    ├── AST N
    ├── callbacks N
    ├── schedules N
    └── events N

          ↓ reload

LOAD N is torn down

          ↓

LOAD N+1
PluginHost N+1
    ├── Engine N+1
    ├── AST N+1
    ├── callbacks N+1
    ├── schedules N+1
    └── events N+1

```

There must never be a valid state where:

```text
PluginHost N+1
    └── callback from N
            └── Engine N

```

or:

```text
PluginHost N+1
    └── callback from N
            └── Engine N+1

```

because the latter silently rebinds old state to a new runtime generation.

The first is a stale-reference bug.

The second is an even more dangerous lifecycle-isolation bug.

The implementation must preserve the generation boundary exactly.