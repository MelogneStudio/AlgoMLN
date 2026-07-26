use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

/// One entry in the immutable live trade log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeLogEntry {
    pub id: String,
    pub timestamp: String,
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    pub side: String,
    pub quantity: i64,
    pub price: f64,
    pub order_id: String,
    pub order_status: String,
    pub mode: String,
    pub rule_id: String,
    pub notes: String,
}

/// Append-only writer. The file is JSONL: one JSON object per line, never truncated.
#[derive(Debug)]
pub struct TradeLog {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

impl TradeLog {
    /// Open or create the log at `path`. Creates parent dirs.
    pub fn open(path: PathBuf) -> Result<Self, std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    /// Append one entry. Takes the file lock, writes one JSON line + '\n', flushes.
    pub fn append(&self, entry: TradeLogEntry) -> Result<(), std::io::Error> {
        let json = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("trade log file lock poisoned"))?;
        writeln!(file, "{json}")?;
        file.flush()
    }

    /// Read all entries from disk (for the IPC get_trade_log command).
    /// Skips malformed lines with an eprintln! warning.
    pub fn read_all(path: &PathBuf) -> Result<Vec<TradeLogEntry>, std::io::Error> {
        let file = OpenOptions::new().read(true).open(path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<TradeLogEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(_) => eprintln!("trade_log: skipping malformed line: {line}"),
            }
        }

        Ok(entries)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn entry(id: &str, symbol: &str) -> TradeLogEntry {
        TradeLogEntry {
            id: id.to_string(),
            timestamp: "2026-07-25T00:00:00Z".to_string(),
            strategy_id: "strategy_1".to_string(),
            strategy_name: "Momentum".to_string(),
            symbol: symbol.to_string(),
            side: "BUY".to_string(),
            quantity: 10,
            price: 123.45,
            order_id: format!("order_{id}"),
            order_status: "TRADED".to_string(),
            mode: "live".to_string(),
            rule_id: "rule_1".to_string(),
            notes: String::new(),
        }
    }

    #[test]
    fn test_append_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trade_log.jsonl");
        let log = TradeLog::open(path.clone()).unwrap();

        log.append(entry("1", "NIFTY")).unwrap();
        log.append(entry("2", "BANKNIFTY")).unwrap();

        let entries = TradeLog::read_all(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "1");
        assert_eq!(entries[0].symbol, "NIFTY");
        assert_eq!(entries[0].order_status, "TRADED");
        assert_eq!(entries[1].id, "2");
        assert_eq!(entries[1].symbol, "BANKNIFTY");
    }

    #[test]
    fn test_open_is_append_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trade_log.jsonl");

        TradeLog::open(path.clone())
            .unwrap()
            .append(entry("1", "NIFTY"))
            .unwrap();
        TradeLog::open(path.clone())
            .unwrap()
            .append(entry("2", "BANKNIFTY"))
            .unwrap();

        let entries = TradeLog::read_all(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_read_skips_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trade_log.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::to_string(&entry("1", "NIFTY")).unwrap()
        )
        .unwrap();
        writeln!(file).unwrap();
        writeln!(file, "{{malformed").unwrap();

        let entries = TradeLog::read_all(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "1");
    }
}
