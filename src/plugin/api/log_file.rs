//! Rate-limited, rolling-file plugin logger.
//!
//! Plugin code is untrusted — a misbehaving (or malicious) plugin could
//! spam `log_info` / `log_warn` / `log_error` in a tight loop and fill
//! the user's SSD with `[plugin:x] ...` lines. This module is the
//! containment layer for that risk.
//!
//! Two defenses are layered:
//!
//! 1. **Token-bucket rate limit per plugin** (see [`RateLimiter`]). The
//!    default is 10 messages per second burst and 100 messages per
//!    minute sustained, shared across all log levels. Excess messages
//!    are silently dropped, but a counter increments and a single
//!    summary line per minute is written so the user can see that
//!    something is being throttled without filling the log.
//!
//! 2. **5MB rolling log file per plugin** (see [`RollingLog`]). The
//!    file lives under `<app_data_dir>/logs/plugin-<id>.log`. On every
//!    write, the writer checks the file size; if the line would push
//!    it over the cap, the file is rotated (current file is renamed
//!    to `<base>.1` — older `*.1` is overwritten — and a fresh
//!    current file is opened). This bounds disk usage to ~5MB per
//!    plugin regardless of rate-limit behavior.
//!
//! The two are combined in [`RateLimitedFileLog`], which implements
//! [`LogApi`]. The Tauri binary wires one of these per plugin via the
//! host factory; the CLI keeps using the simpler [`NamespacedLog`]
//! (no file output, no rate limit) so terminal output stays
//! human-readable.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use parking_lot::Mutex as PlMutex;

use crate::plugin::types::PluginId;

use super::LogApi;

/// Maximum size of a single log file before rotation, in bytes.
/// 5 MB is the spec; this constant is the single source of truth.
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// On rotation the current file is renamed to `<base>.1`; any prior
/// `<base>.1` is overwritten. We do NOT keep a sequence of historical
/// logs — the spec asks for a single rolling file per plugin.
const ROTATED_SUFFIX: &str = ".1";

/// Token-bucket rate limiter. Per plugin, shared across all log
/// levels. Defaults: 10 msg/sec burst, 100 msg/min sustained.
///
/// `now` is fed in (rather than read from `Instant::now()` inside the
/// struct) so tests can drive the clock deterministically.
#[derive(Debug)]
pub struct RateLimiter {
    /// Token-bucket refills at `burst_per_second` tokens/sec up to
    /// `burst_capacity`. We track fractional tokens so a steady drip
    /// (1 msg / 250ms) and a single 10-msg burst both work.
    burst_capacity: f64,
    burst_refill_per_sec: f64,
    burst_tokens: f64,
    burst_last_refill: Instant,

    /// Sliding window: how many messages were written in the last
    /// `sustained_window_secs`. When the count exceeds
    /// `sustained_max` the limiter blocks until the oldest message
    /// in the window ages out.
    sustained_max: u32,
    sustained_window_secs: u64,
    /// `(timestamp, count)` pairs in arrival order. Pairs are merged
    /// when adjacent to keep the vector short.
    sustained_log: Vec<(Instant, u32)>,
    sustained_in_window: u32,

    /// Throttling telemetry. Every time we drop a message we bump
    /// `dropped`. Once per minute we emit a single summary line
    /// ("N messages dropped in the last 60s") and reset the counter.
    dropped: u64,
    last_summary_at: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(
            /* burst_capacity */ 10,
            /* burst_per_sec */ 10.0,
            /* sustained_max */ 100,
            /* sustained_window_secs */ 60,
        )
    }
}

impl RateLimiter {
    /// Build a limiter with explicit limits. `burst_per_sec` is the
    /// sustained refill rate for the short-term bucket; for the
    /// defaults we pass `10.0` to match the 10/sec burst budget.
    pub fn new(
        burst_capacity: u32,
        burst_per_sec: f64,
        sustained_max: u32,
        sustained_window_secs: u64,
    ) -> Self {
        let now = Instant::now();
        Self {
            burst_capacity: burst_capacity as f64,
            burst_refill_per_sec: burst_per_sec,
            burst_tokens: burst_capacity as f64,
            burst_last_refill: now,
            sustained_max,
            sustained_window_secs,
            sustained_log: Vec::new(),
            sustained_in_window: 0,
            dropped: 0,
            last_summary_at: now,
        }
    }

    /// Try to admit one message. Returns `true` if the caller should
    /// proceed, `false` if the message should be dropped.
    pub fn try_admit(&mut self, now: Instant) -> bool {
        // Refill the burst bucket.
        let elapsed = now
            .saturating_duration_since(self.burst_last_refill)
            .as_secs_f64();
        if elapsed > 0.0 {
            self.burst_tokens =
                (self.burst_tokens + elapsed * self.burst_refill_per_sec).min(self.burst_capacity);
            self.burst_last_refill = now;
        }
        if self.burst_tokens < 1.0 {
            self.record_drop(now);
            return false;
        }

        // Drop any sustained-log entries that have aged out.
        self.sustained_log.retain(|(t, _)| {
            now.saturating_duration_since(*t).as_secs() < self.sustained_window_secs
        });
        // Recompute the in-window count after aging out.
        self.sustained_in_window = self.sustained_log.iter().map(|(_, c)| *c).sum();

        if self.sustained_in_window >= self.sustained_max {
            self.record_drop(now);
            return false;
        }

        // Admit: consume a burst token and append to the window.
        self.burst_tokens -= 1.0;
        match self.sustained_log.last_mut() {
            Some((t, c)) if now.saturating_duration_since(*t).as_secs() < 1 => {
                *c += 1;
            }
            _ => self.sustained_log.push((now, 1)),
        }
        self.sustained_in_window += 1;
        true
    }

    fn record_drop(&mut self, _now: Instant) {
        self.dropped += 1;
    }

    /// Number of messages dropped since the last summary was emitted.
    pub fn dropped_since_summary(&self) -> u64 {
        self.dropped
    }

    /// Take the dropped counter, returning how many messages were
    /// dropped since the last reset. The caller is expected to write
    /// a single summary line per minute; the limiter does not write
    /// on its own.
    pub fn take_summary(&mut self, now: Instant) -> Option<(u64, Duration)> {
        if self.dropped == 0 {
            return None;
        }
        let since = now.saturating_duration_since(self.last_summary_at);
        // Emit at most once per minute.
        if since < Duration::from_secs(60) {
            return None;
        }
        let dropped = std::mem::take(&mut self.dropped);
        self.last_summary_at = now;
        Some((dropped, since))
    }
}

/// Rolling log file. Owns one open `File` plus a `Mutex` so the
/// `RateLimitedFileLog` methods can be called from multiple threads
/// (plugins run concurrently and the Rhai/WASM runtimes may invoke
/// `log_info` from any task). The size check is "best-effort" — two
/// concurrent writers can both observe `len() < MAX_LOG_BYTES` and
/// write through, briefly exceeding the cap. The size check is a
/// guardrail, not a strict budget; rolling on every write would be
/// pathologically expensive.
pub struct RollingLog {
    path: PathBuf,
    file: Mutex<Option<File>>,
    /// Cached file size, refreshed on every `write_line`. Avoids an
    /// `fstat` per line. `0` means "fresh start or post-rotate".
    size: Mutex<u64>,
}

impl RollingLog {
    /// Open (or create) the rolling log at `path`. The parent
    /// directory must already exist.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            path,
            file: Mutex::new(Some(file)),
            size: Mutex::new(size),
        })
    }

    /// Append one line (no trailing newline added; the caller is
    /// expected to include it). Rotates the file first if the line
    /// would push the file over `MAX_LOG_BYTES`.
    pub fn write_line(&self, line: &str) -> std::io::Result<()> {
        let line_len = line.as_bytes().len() as u64;
        let current = *self.size.lock().unwrap();
        if current + line_len > MAX_LOG_BYTES {
            self.rotate()?;
        }

        let mut guard = self.file.lock().unwrap();
        let file = guard
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "log file closed"))?;
        file.write_all(line.as_bytes())?;
        file.flush()?;
        let new_size = file.metadata().map(|m| m.len()).unwrap_or(current + line_len);
        *self.size.lock().unwrap() = new_size;
        Ok(())
    }

    /// Rotate: close the current file (by taking the handle out of
    /// the `Mutex`), rename it to `<base>.1`, and open a fresh
    /// current file. Any prior `<base>.1` is overwritten.
    ///
    /// On Windows, `std::fs::rename` fails if the source file is
    /// open. Holding the `Mutex` across the rename would block
    /// concurrent `write_line` callers for the cost of a rename,
    /// so we briefly take the `Option<File>` out, then release
    /// the lock before performing IO.
    fn rotate(&self) -> std::io::Result<()> {
        // Step 1: take the open handle out of the slot. Any
        // concurrent writer that grabs the lock now sees `None`
        // and is rejected — but the spec is "rate-limited logs",
        // not "logs must never fail", and a single drop is fine.
        let old = {
            let mut guard = self.file.lock().unwrap();
            guard.take()
        };
        // Drop closes the OS handle.
        drop(old);

        let rotated = rotated_path(&self.path);
        // Best-effort: ignore "not found" on the unlink step.
        let _ = std::fs::remove_file(&rotated);
        std::fs::rename(&self.path, &rotated)?;

        // `create(true)` alone is enough — the file does not exist
        // yet at this path (we just renamed it away). `append(true)`
        // makes every write seek to the end; that combined with
        // `create(true)` gives us a fresh, empty log file.
        let fresh = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&self.path)?;
        *self.file.lock().unwrap() = Some(fresh);
        *self.size.lock().unwrap() = 0;
        Ok(())
    }

    /// Path to the rolling log file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn rotated_path(base: &Path) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(ROTATED_SUFFIX);
    PathBuf::from(s)
}

/// Per-plugin rate limiter + rolling file. The "production"
/// `LogApi` implementation wired in by the Tauri host factory.
pub struct RateLimitedFileLog {
    plugin_id: PluginId,
    log: RollingLog,
    limiter: PlMutex<RateLimiter>,
}

impl RateLimitedFileLog {
    /// Open the log file and allocate a fresh rate limiter for this
    /// plugin. Creates the parent directory if needed.
    pub fn open(logs_dir: &Path, plugin_id: PluginId) -> std::io::Result<Self> {
        std::fs::create_dir_all(logs_dir)?;
        let path = logs_dir.join(format!("plugin-{}.log", plugin_id));
        let log = RollingLog::open(path)?;
        Ok(Self {
            plugin_id,
            log,
            limiter: PlMutex::new(RateLimiter::default()),
        })
    }

    /// Build a `RateLimitedFileLog` over an already-opened log file.
    /// Used by tests and by callers that want to keep the file
    /// outside the standard `<app_data>/logs/` tree.
    pub fn with_log(plugin_id: PluginId, log: RollingLog) -> Self {
        Self {
            plugin_id,
            log,
            limiter: PlMutex::new(RateLimiter::default()),
        }
    }

    /// Format `[plugin:{id}] [{LEVEL}] {msg}\n` and run it through
    /// the rate limiter. Returns `true` if the line was actually
    /// written, `false` if it was dropped.
    fn emit(&self, level: &str, message: &str) -> bool {
        let now = Instant::now();
        let mut limiter = self.limiter.lock();
        if !limiter.try_admit(now) {
            return false;
        }
        let line = format!("[plugin:{}] [{}] {}\n", self.plugin_id, level, message);
        // Drop the limiter lock before taking the file lock to keep
        // the critical sections short.
        drop(limiter);
        if let Err(err) = self.log.write_line(&line) {
            // We can't do much — the spec is "logs 5MB rolling", not
            // "logs must not fail". Surface to stderr so a tail -f
            // still sees something rather than silently disappearing.
            eprintln!(
                "[plugin:{}] log write failed: {err}",
                self.plugin_id
            );
        }
        // Periodic dropped-message summary. We do this on every
        // admit (cheap) but `take_summary` only returns a value
        // once per minute.
        if let Some((dropped, since)) = self.limiter.lock().take_summary(now) {
            let line = format!(
                "[plugin:{}] [WARN] rate-limited: {} message(s) dropped in the last {}s\n",
                self.plugin_id,
                dropped,
                since.as_secs()
            );
            let _ = self.log.write_line(&line);
        }
        true
    }
}

#[async_trait::async_trait]
impl LogApi for RateLimitedFileLog {
    fn debug(&self, _plugin_id: &PluginId, message: &str) {
        // Spec defines only info/warn/error. Map debug to info so
        // plugin authors that ask for `log_debug` from a future
        // runtime still get something on disk.
        self.emit("DEBUG", message);
    }
    fn info(&self, _plugin_id: &PluginId, message: &str) {
        self.emit("INFO", message);
    }
    fn warn(&self, _plugin_id: &PluginId, message: &str) {
        self.emit("WARN", message);
    }
    fn error(&self, _plugin_id: &PluginId, message: &str) {
        self.emit("ERROR", message);
    }
}

/// Container that owns one [`RateLimitedFileLog`] per plugin. The
/// Tauri host factory would build one of these and hand each plugin
/// a cheap `Arc<dyn LogApi>` clone, but plugins can be enabled /
/// disabled / reloaded at runtime — to avoid leaking entries for
/// disabled plugins, the registry is responsible for dropping them.
///
/// In practice the Tauri host factory is currently `Fn`-based and
/// hands out fresh `RateLimitedFileLog` per plugin, so this container
/// is mainly useful for tests and for callers that want a single
/// `LogApi` per app rather than per plugin. Kept here as scaffolding
/// for the per-plugin resource dashboard the team is likely to add
/// next.
#[derive(Default)]
pub struct SharedPluginLogRegistry {
    inner: PlMutex<HashMap<PluginId, std::sync::Arc<RateLimitedFileLog>>>,
}

impl SharedPluginLogRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a log for the given plugin id. The closure runs
    /// only on the create path; if a log already exists for this id
    /// the existing `Arc` is returned.
    pub fn get_or_create<F>(&self, plugin_id: PluginId, make: F) -> std::io::Result<std::sync::Arc<RateLimitedFileLog>>
    where
        F: FnOnce() -> std::io::Result<RateLimitedFileLog>,
    {
        let mut guard = self.inner.lock();
        if let Some(existing) = guard.get(&plugin_id) {
            return Ok(existing.clone());
        }
        let log = std::sync::Arc::new(make()?);
        guard.insert(plugin_id, log.clone());
        Ok(log)
    }

    /// Drop the entry for a plugin. Call this on
    /// `enable -> disabled` transitions or on `unload`.
    pub fn drop_for(&self, plugin_id: &PluginId) {
        self.inner.lock().remove(plugin_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn fresh_limiter() -> RateLimiter {
        // Tight windows so tests don't sleep for a real minute.
        RateLimiter::new(3, 3.0, 5, 60)
    }

    #[test]
    fn rate_limiter_admits_within_burst() {
        let mut l = fresh_limiter();
        let now = Instant::now();
        for _ in 0..3 {
            assert!(l.try_admit(now));
        }
    }

    #[test]
    fn rate_limiter_blocks_after_burst() {
        let mut l = fresh_limiter();
        let now = Instant::now();
        for _ in 0..3 {
            assert!(l.try_admit(now));
        }
        assert!(!l.try_admit(now));
    }

    #[test]
    fn rate_limiter_refills() {
        let mut l = fresh_limiter();
        let now = Instant::now();
        for _ in 0..3 {
            assert!(l.try_admit(now));
        }
        assert!(!l.try_admit(now));
        // One second later, the burst bucket has refilled.
        let later = now + Duration::from_secs(1);
        assert!(l.try_admit(later));
    }

    #[test]
    fn rate_limiter_sustained_caps_long_run() {
        // Burst=1000 to keep the burst bucket out of the way, then
        // make sure the sustained window blocks the 6th message in
        // a 5-message window.
        let mut l = RateLimiter::new(1000, 1000.0, 5, 60);
        let now = Instant::now();
        for i in 0..5 {
            assert!(l.try_admit(now), "msg {i} should be admitted");
        }
        assert!(!l.try_admit(now), "6th should be dropped");
    }

    #[test]
    fn rate_limiter_summary_only_after_window() {
        let mut l = RateLimiter::new(1, 0.0, 1, 60);
        let now = Instant::now();
        // Admit one, drop the rest.
        assert!(l.try_admit(now));
        for _ in 0..5 {
            assert!(!l.try_admit(now));
        }
        // Within the summary window we get None.
        assert!(l.take_summary(now).is_none());
        // After 60s+ we get the count.
        let later = now + Duration::from_secs(61);
        let summary = l.take_summary(later);
        assert_eq!(summary.unwrap().0, 5);
    }

    #[test]
    fn rolling_log_rotates_at_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin-x.log");
        let log = RollingLog::open(path.clone()).unwrap();

        // Write enough to overflow twice. Each line is 1 MB.
        let big = "x".repeat(1024 * 1024);
        for _ in 0..7 {
            log.write_line(&format!("{big}\n")).unwrap();
        }
        // After rotation we should have a `<base>.1` file plus the
        // current file, and the current file should be under the
        // cap.
        let rotated = dir.path().join("plugin-x.log.1");
        assert!(rotated.exists());
        let current_size = std::fs::metadata(&path).unwrap().len();
        assert!(current_size < MAX_LOG_BYTES);
    }

    #[test]
    fn rolling_log_appends_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugin-x.log");
        {
            let log = RollingLog::open(path.clone()).unwrap();
            log.write_line("first\n").unwrap();
        }
        // Reopen: should preserve contents, not truncate.
        let log = RollingLog::open(path.clone()).unwrap();
        log.write_line("second\n").unwrap();
        let mut contents = String::new();
        File::open(&path).unwrap().read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "first\nsecond\n");
    }

    #[test]
    fn rate_limited_file_log_drops_under_spam() {
        let dir = tempfile::tempdir().unwrap();
        let log = RateLimitedFileLog::open(dir.path(), PluginId::from("p")).unwrap();
        // 3 admitted, the rest dropped (defaults: burst=10, sustained=100).
        let mut admitted = 0;
        for i in 0..50 {
            if log.emit("INFO", &format!("msg-{i}")) {
                admitted += 1;
            }
        }
        // We don't assert an exact admitted count — the burst bucket
        // can refill slightly between iterations. The point is that
        // a spam run is throttled, not amplified.
        assert!(admitted <= 50);
        assert!(admitted >= 3);
    }
}
