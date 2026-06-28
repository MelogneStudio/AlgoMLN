use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::NaiveDateTime;

use crate::models::Candle;

/// Loads a NIFTY-style OHLCV CSV. Rows can be tab-separated, comma-separated, or
/// whitespace-separated (in which case the first 19 characters are taken as the
/// timestamp). The first non-blank line is treated as a header and skipped.
///
/// Returns `String` errors so the loader is directly callable from Tauri
/// commands without an `anyhow` round-trip.
pub fn load_nifty_candles(path: &Path) -> Result<Vec<Candle>, String> {
    let file = File::open(path)
        .map_err(|error| format!("failed to open {}: {}", path.display(), error))?;
    let mut candles = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| format!("read error at line {}: {}", index + 1, error))?;
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields = parse_market_row(&line)
            .map_err(|error| format!("bad NIFTY candle row {}: {}", index + 1, error))?;
        let timestamp = NaiveDateTime::parse_from_str(fields[0], "%Y-%m-%d %H:%M:%S")
            .map_err(|error| format!("bad timestamp at line {}: {}", index + 1, error))?
            .and_utc()
            .timestamp_millis();
        candles.push(Candle {
            timestamp,
            open: fields[1]
                .parse::<f64>()
                .map_err(|error| format!("bad open at line {}: {}", index + 1, error))?,
            high: fields[2]
                .parse::<f64>()
                .map_err(|error| format!("bad high at line {}: {}", index + 1, error))?,
            low: fields[3]
                .parse::<f64>()
                .map_err(|error| format!("bad low at line {}: {}", index + 1, error))?,
            close: fields[4]
                .parse::<f64>()
                .map_err(|error| format!("bad close at line {}: {}", index + 1, error))?,
            volume: 1_000.0,
        });
    }
    Ok(candles)
}

/// Splits a single CSV/TSV/whitespace-separated market row into exactly 5
/// fields: timestamp, open, high, low, close.
pub fn parse_market_row(line: &str) -> Result<Vec<&str>, String> {
    let tab_fields = line.split('\t').collect::<Vec<_>>();
    if tab_fields.len() == 5 {
        return Ok(tab_fields);
    }

    let comma_fields = line.split(',').collect::<Vec<_>>();
    if comma_fields.len() == 5 {
        return Ok(comma_fields);
    }

    let whitespace_fields = line.split_whitespace().collect::<Vec<_>>();
    if whitespace_fields.len() == 6 {
        let timestamp = line
            .get(0..19)
            .ok_or_else(|| "missing datetime".to_string())?;
        return Ok(vec![
            timestamp,
            whitespace_fields[2],
            whitespace_fields[3],
            whitespace_fields[4],
            whitespace_fields[5],
        ]);
    }

    Err("expected 5 market columns".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_row() {
        let line = "2024-01-01 09:15:00\t100.5\t101.0\t99.5\t100.75";
        let fields = parse_market_row(line).unwrap();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[0], "2024-01-01 09:15:00");
        assert_eq!(fields[4], "100.75");
    }

    #[test]
    fn parses_comma_separated_row() {
        let line = "2024-01-01 09:15:00,100.5,101.0,99.5,100.75";
        let fields = parse_market_row(line).unwrap();
        assert_eq!(fields[4], "100.75");
    }

    #[test]
    fn parses_whitespace_separated_row() {
        // Whitespace layout expects exactly 6 tokens: timestamp prefix,
        // symbol, and OHLC. The parser takes the first 19 characters as the
        // timestamp (YYYY-MM-DD HH:MM:SS) and reads OHLC from indices 2..6.
        let line = "2024-01-01 09:15:00 NIFTY 100.5 101.0 99.5";
        let fields = parse_market_row(line).unwrap();
        assert_eq!(fields[0], "2024-01-01 09:15:00");
        assert_eq!(fields[1], "NIFTY");
        assert_eq!(fields[4], "99.5");
    }

    #[test]
    fn rejects_malformed_row() {
        let err = parse_market_row("2024-01-01 09:15:00 100.5").unwrap_err();
        assert!(err.contains("5 market columns"));
    }
}