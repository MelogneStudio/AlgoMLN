use crate::exec::{
    state::{OrderState, OrderTracker},
    types::{LocalOrderId, OrderRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalRecord {
    pub local_id: LocalOrderId,
    pub correlation_id: String,
    pub dhan_order_id: Option<String>,
    pub state: OrderState,
    pub request: OrderRequest,
    pub cum_traded_qty: u64,
    pub ts_ms: i64,
}

pub struct OrderWal {
    path: PathBuf,
    file: Option<File>,
}

impl OrderWal {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let p = path.as_ref().to_path_buf();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&p)?;

        Ok(Self {
            path: p,
            file: Some(file),
        })
    }

    /// In-memory WAL for tests or dry runs where no disk persistence is required.
    pub fn memory() -> Self {
        Self {
            path: PathBuf::new(),
            file: None,
        }
    }

    pub fn append(
        &mut self,
        tracker: &OrderTracker,
    ) -> Result<(), std::io::Error> {
        let Some(ref mut f) = self.file else {
            return Ok(());
        };

        let now_ms = chrono::Utc::now().timestamp_millis();
        let record = WalRecord {
            local_id: tracker.local_id.clone(),
            correlation_id: tracker.correlation_id.clone(),
            dhan_order_id: tracker.dhan_order_id.clone(),
            state: tracker.state,
            request: tracker.request.clone(),
            cum_traded_qty: tracker.cum_traded_qty,
            ts_ms: now_ms,
        };

        let json = serde_json::to_string(&record).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?;
        writeln!(f, "{}", json)?;
        f.flush()?;
        Ok(())
    }

    /// Read the WAL file and rebuild the latest state for all orders.
    pub fn recover_all(&self) -> Result<BTreeMap<LocalOrderId, OrderTracker>, std::io::Error> {
        if self.file.is_none() || !self.path.exists() {
            return Ok(BTreeMap::new());
        }

        let f = File::open(&self.path)?;
        let reader = BufReader::new(f);
        let mut orders: BTreeMap<LocalOrderId, OrderTracker> = BTreeMap::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<WalRecord>(&line) {
                let tracker = orders.entry(rec.local_id.clone()).or_insert_with(|| {
                    OrderTracker::new(rec.request.clone(), rec.correlation_id.clone())
                });
                tracker.state = rec.state;
                tracker.dhan_order_id = rec.dhan_order_id;
                tracker.cum_traded_qty = rec.cum_traded_qty;
                tracker.remaining_qty = tracker.order_qty.saturating_sub(rec.cum_traded_qty);
            }
        }

        Ok(orders)
    }

    /// Return open/pending orders that need broker reconciliation after crash recovery.
    pub fn recover_open_orders(&self) -> Result<Vec<OrderTracker>, std::io::Error> {
        let all = self.recover_all()?;
        Ok(all
            .into_values()
            .filter(|o| o.state.is_open_or_pending())
            .collect())
    }
}
