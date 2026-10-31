# Step 5b §6: concurrency semaphore — design note (deferred)

Status: deferred by design. The serial fetch+write path is left intact and
correct. This note specifies exactly how to add the default-4 in-flight
semaphore later, where it goes, and why spacing stays correct.

## Why it is deferred

The spec (§6) wants up to 4 in-flight requests while the shared `Limiter` keeps
request *starts* >=250 ms apart. The win is only real when per-request latency
exceeds 250 ms (so several requests overlap on the wire).

The blocker is ownership, not timing:

- `Updater` owns a single `Catalog` (one `rusqlite::Connection`) and a `Store`.
  `Updater::update` interleaves, per chunk: budget-increment (own txn), network
  fetch, `complete_intraday`/daily filter, `check_overlap` against existing
  partitions read back from disk, Parquet write, and a coverage/session txn.
- `rusqlite::Connection` is not `Sync`; a single connection cannot be shared
  across fetch threads. The write side (coverage, sessions, budget, partitions)
  must stay serial on one connection or correctness guarantees from §5/§8
  (budget counted before send, atomic rename then catalog txn, no double-count)
  break.
- The overlap/corporate-action logic reads the *current* on-disk partitions for
  the series before writing. If two chunks of the same series are written
  concurrently, that read-modify-write races. So any concurrency must be
  **bounded to network fetch only**, never the write path.

Performing this refactor without a compiler is high risk. A correct deferral
beats a broken parallel path.

## Target shape: fetch-ahead with one writer

Keep one SQLite connection and one writer thread. Add a bounded fetch-ahead in
front of it. The scheduler (the thing the spec says "owns all timing") already
exists physically as the shared `Limiter` inside `DhanClient` (250 ms spacing +
DH-904 backoff); the semaphore is layered around the *fetch*, not the limiter.

```
planner ──> [Request{security_id, tf, from, to}] (mpsc, serial, per spec 4 order)
                     │
        N fetch workers (N = semaphore permits, default 4)
          each: limiter.wait()  (shared Limiter => >=250 ms between *starts*)
                fetch intraday/daily, budget_add BEFORE send
                     │
          completed batches (mpsc, carry security_id/tf/from/to + Vec<Bar>)
                     │
        single writer thread (owns the one Catalog + Store):
          complete_intraday/daily filter, check_overlap, write_partitions,
          coverage/session txn, clear_failure/record_failure
```

### Where the semaphore goes

- Add `pub concurrency: usize` to `BulkConfig` (default 4). `Limiter` already
  carries a `concurrency()` accessor (currently 2) — reuse or override it.
- Wrap it as `Arc<Semaphore>` with `concurrency` permits (either a small
  hand-rolled counting semaphore over `std::sync::{Mutex, Condvar}`, or
  `std::sync::Arc<tokio::sync::Semaphore>` since `dhan-api` already pulls in
  tokio). A fetch worker: `let _permit = sem.acquire(); client.wait(); fetch()`.
- Critically, `limiter.wait()` is called **inside** the permit, so even with 4
  permits the shared single `Limiter` serialises the *start* of each request to
  one every 250 ms. The semaphore only caps how many may be simultaneously
  *in flight* (awaiting their HTTP response). This is exactly §6's "the
  semaphore never overrides the 250 ms spacing."

### Spacing invariant (why it still holds)

`Limiter::wait` takes the inner `Mutex<Bucket>` and only returns when a token is
available, advancing `last`. With one shared `Limiter` cloned into every fetch
worker, concurrent `wait()` calls contend on that mutex and are granted tokens
no faster than `cap` (4/s => 250 ms). The DH-904 path (`rate_limit()` → 300 s
tight window, cap drops to 2) likewise applies globally because the bucket is
shared. So concurrency changes only the number of outstanding responses, never
the inter-start spacing.

### What must stay serial (hard requirements)

- Budget increment per request (its own WAL txn) stays on the writer or uses a
  dedicated short-lived connection; failed requests still count (§5).
- Per-series write ordering: never let two batches of the *same* series reach
  the writer out of order relative to the overlap read. Simplest safe rule:
  partition fetch-ahead by series — allow concurrency *across* different
  security_ids, but serialise chunks *within* one security_id. Since §4 already
  processes "symbol by symbol, finish all requests for one symbol then commit,"
  the natural unit of concurrency is across symbols' chunk fetches while a
  single writer commits per symbol.
- Resume/commit boundary (§4/§7): a stop (Ctrl-C) or budget hit must still leave
  whole symbols behind. With one writer committing per symbol, drain the
  in-flight fetches for the current symbol, commit it, then stop — identical to
  today's between-symbol `stop` check.

## Estimated effort

Moderate: new `concurrency` config field, a semaphore type, a writer thread with
two mpsc channels, and careful teardown on stop/budget/auth. All of it needs a
compiler to validate `Send`/lifetime bounds on `make_client` and the channels.
Recommend implementing once the crate can be built and the §11 acceptance tests
(now present) can guard the behaviour — in particular test 11.2 (250 ms spacing)
must still pass with concurrency > 1, which is the key regression gate.
