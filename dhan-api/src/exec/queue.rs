use crate::exec::types::{LocalOrderId, OrderIntent, OrderRequest};
use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    time::{Duration, Instant},
};

pub const DAILY_ORDER_CAP: u64 = 7_000;
pub const WARN_80_PCT: u64 = 5_600;
pub const STOP_ENTRIES_95_PCT: u64 = 6_650;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Exit = 0,   // Highest priority
    Cancel = 1, // Medium priority
    Entry = 2,  // Lowest priority
}

#[derive(Clone, Debug)]
pub enum QueueAction {
    Place(OrderRequest),
    Cancel {
        local_id: LocalOrderId,
        dhan_order_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct PrioritizedItem {
    pub priority: Priority,
    pub enqueued_seq: u64,
    pub action: QueueAction,
}

impl PartialEq for PrioritizedItem {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.enqueued_seq == other.enqueued_seq
    }
}

impl Eq for PrioritizedItem {}

impl Ord for PrioritizedItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower priority enum number = higher priority in BinaryHeap (so reverse)
        // Earlier sequence number = higher priority
        match (self.priority as u8).cmp(&(other.priority as u8)).reverse() {
            Ordering::Equal => self.enqueued_seq.cmp(&other.enqueued_seq).reverse(),
            other_ord => other_ord,
        }
    }
}

impl PartialOrd for PrioritizedItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterStatus {
    Normal,
    Warning80Pct,
    StopEntries95Pct,
    Exceeded,
}

/// Token bucket and daily counter governing submission rate and priority.
/// Section 11:
/// - Order APIs: 10 requests per second and 7,000 per day.
/// - Token bucket at ~8 per second.
/// - Priority: exits, then cancels, then entries.
/// - Persist daily order counter. Warn at 80% and stop entries at 95%.
pub struct SubmissionQueue {
    heap: BinaryHeap<PrioritizedItem>,
    seq_counter: u64,
    tokens: f64,
    bucket_capacity: f64,
    refill_rate_per_sec: f64,
    last_refill: Instant,
    daily_count: u64,
}

impl Default for SubmissionQueue {
    fn default() -> Self {
        Self::new(8.0, 10.0)
    }
}

impl SubmissionQueue {
    pub fn new(refill_rate_per_sec: f64, bucket_capacity: f64) -> Self {
        Self {
            heap: BinaryHeap::new(),
            seq_counter: 0,
            tokens: bucket_capacity,
            bucket_capacity,
            refill_rate_per_sec,
            last_refill: Instant::now(),
            daily_count: 0,
        }
    }

    pub fn daily_count(&self) -> u64 {
        self.daily_count
    }

    pub fn set_daily_count(&mut self, count: u64) {
        self.daily_count = count;
    }

    pub fn can_submit_entry(&self) -> bool {
        self.daily_count < STOP_ENTRIES_95_PCT
    }

    pub fn enqueue_place(&mut self, req: OrderRequest) -> Result<(), &'static str> {
        let priority = match req.intent {
            OrderIntent::Exit => Priority::Exit,
            OrderIntent::Entry => {
                if !self.can_submit_entry() {
                    return Err("entries locked: daily order count reached 95% safety threshold");
                }
                Priority::Entry
            }
        };
        self.seq_counter += 1;
        self.heap.push(PrioritizedItem {
            priority,
            enqueued_seq: self.seq_counter,
            action: QueueAction::Place(req),
        });
        Ok(())
    }

    pub fn enqueue_cancel(&mut self, local_id: LocalOrderId, dhan_order_id: String) {
        self.seq_counter += 1;
        self.heap.push(PrioritizedItem {
            priority: Priority::Cancel,
            enqueued_seq: self.seq_counter,
            action: QueueAction::Cancel {
                local_id,
                dhan_order_id,
            },
        });
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate_per_sec).min(self.bucket_capacity);
        self.last_refill = now;
    }

    /// Check if a token is available immediately.
    pub fn try_acquire_token(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Time until next token is available.
    pub fn wait_duration(&mut self) -> Duration {
        self.refill();
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let deficit = 1.0 - self.tokens;
            Duration::from_secs_f64(deficit / self.refill_rate_per_sec)
        }
    }

    /// Pop the next highest priority item if token is consumed.
    /// Drops entries if daily limit has reached the 95% stop-entries threshold.
    pub fn pop_next(&mut self) -> Option<QueueAction> {
        while let Some(item) = self.heap.pop() {
            if item.priority == Priority::Entry && !self.can_submit_entry() {
                // Entries are locked at 95% safety threshold; discard pending entry
                continue;
            }
            self.daily_count += 1;
            return Some(item.action);
        }
        None
    }

    pub fn counter_status(&self) -> CounterStatus {
        if self.daily_count >= DAILY_ORDER_CAP {
            CounterStatus::Exceeded
        } else if self.daily_count >= STOP_ENTRIES_95_PCT {
            CounterStatus::StopEntries95Pct
        } else if self.daily_count >= WARN_80_PCT {
            CounterStatus::Warning80Pct
        } else {
            CounterStatus::Normal
        }
    }
}
