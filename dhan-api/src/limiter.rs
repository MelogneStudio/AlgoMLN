use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct Limiter {
    inner: Arc<Mutex<Bucket>>,
    cap: usize,
    concurrency: usize,
}

struct Bucket {
    tokens: f64,
    last: std::time::Instant,
    tight_until: Option<std::time::Instant>,
}

impl Limiter {
    // Dhan documents 5 requests/second; operate at 4 with a 250 ms minimum interval.
    pub fn provisional() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Bucket {
                tokens: 1.0,
                last: std::time::Instant::now(),
                tight_until: None,
            })),
            cap: 4,
            concurrency: 2,
        }
    }

    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    pub fn rate_limit(&self) {
        if let Ok(mut b) = self.inner.lock() {
            b.tokens = 0.0;
            b.tight_until = Some(std::time::Instant::now() + Duration::from_secs(300));
            b.tokens = 0.0;
        }
    }

    pub fn daily_limit_remaining(&self) -> u64 {
        90_000
    }

    pub fn wait(&self) {
        let delay = {
            let mut b = self.inner.lock().expect("limiter poisoned");
            let now = std::time::Instant::now();
            let cap = if b.tight_until.is_some_and(|t| t > now) {
                2.0
            } else {
                self.cap as f64
            };
            if b.tight_until.is_some_and(|t| t <= now) {
                b.tight_until = None;
            }
            if now > b.last {
                let elapsed = (now - b.last).as_secs_f64();
                b.tokens = (b.tokens + elapsed * cap).min(cap);
                b.last = now;
            }
            if b.tokens >= 1.0 {
                b.tokens -= 1.0;
                Duration::ZERO
            } else {
                let needed = 1.0 - b.tokens;
                let wait_time = Duration::from_secs_f64(needed / cap);
                b.tokens = 0.0;
                let scheduled = b.last + wait_time;
                b.last = scheduled;
                scheduled.saturating_duration_since(now)
            }
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}
