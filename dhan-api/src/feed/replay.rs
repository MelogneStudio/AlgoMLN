use crate::feed::decoder::{decode, Packet};
use crate::feed::normalize::{ltp_to_paise, ltt_to_utc_seconds, Deduper, Tick};
use crate::feed::source::{FeedEvent, TickSource};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Deserialize)]
struct FrameLine {
    t: f64,
    hex: String,
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

pub struct ReplaySource {
    reader: Box<dyn BufRead>,
    deduper: Deduper,
    current_time_t: Option<f64>,
    emitted_connected: bool,
    pub total_frames: usize,
    pub dropped_duplicates: usize,
    pub decode_errors: usize,
}

impl ReplaySource {
    pub fn new(reader: Box<dyn BufRead>) -> Self {
        Self {
            reader,
            deduper: Deduper::new(),
            current_time_t: None,
            emitted_connected: false,
            total_frames: 0,
            dropped_duplicates: 0,
            decode_errors: 0,
        }
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self::new(Box::new(BufReader::new(file))))
    }

    pub fn from_reader<R: BufRead + 'static>(reader: R) -> Self {
        Self::new(Box::new(reader))
    }

    pub fn current_time_seconds(&self) -> Option<f64> {
        self.current_time_t
    }
}

impl TickSource for ReplaySource {
    fn current_time_utc(&self) -> Option<i64> {
        self.current_time_t.map(|t| t as i64)
    }

    fn next(&mut self) -> Option<FeedEvent> {
        if !self.emitted_connected {
            self.emitted_connected = true;
            return Some(FeedEvent::Connected);
        }

        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None, // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    let frame: FrameLine = match serde_json::from_str(trimmed) {
                        Ok(f) => f,
                        Err(_) => {
                            self.decode_errors += 1;
                            continue;
                        }
                    };

                    self.current_time_t = Some(frame.t);
                    let recv_ns = (frame.t * 1_000_000_000.0).round() as u64;

                    let bytes = match hex_to_bytes(&frame.hex) {
                        Some(b) => b,
                        None => {
                            self.decode_errors += 1;
                            continue;
                        }
                    };

                    self.total_frames += 1;
                    let (header, packet) = match decode(&bytes) {
                        Ok(res) => res,
                        Err(_) => {
                            self.decode_errors += 1;
                            continue;
                        }
                    };

                    match packet {
                        Packet::Ticker { ltp, ltt } => {
                            let time_utc = ltt_to_utc_seconds(ltt);
                            let price_paise = ltp_to_paise(ltp);
                            if !self.deduper.accept(header.security_id, time_utc, price_paise) {
                                self.dropped_duplicates += 1;
                                continue;
                            }
                            return Some(FeedEvent::Tick(Tick {
                                security_id: header.security_id,
                                price_paise,
                                time_utc,
                                volume: None,
                                recv_ns,
                            }));
                        }
                        Packet::Quote {
                            ltp,
                            ltt,
                            volume,
                            ..
                        } => {
                            let time_utc = ltt_to_utc_seconds(ltt);
                            let price_paise = ltp_to_paise(ltp);
                            if !self.deduper.accept(header.security_id, time_utc, price_paise) {
                                self.dropped_duplicates += 1;
                                continue;
                            }
                            return Some(FeedEvent::Tick(Tick {
                                security_id: header.security_id,
                                price_paise,
                                time_utc,
                                volume: Some(volume as i64),
                                recv_ns,
                            }));
                        }
                        Packet::Full {
                            ltp,
                            ltt,
                            volume,
                            ..
                        } => {
                            let time_utc = ltt_to_utc_seconds(ltt);
                            let price_paise = ltp_to_paise(ltp);
                            if !self.deduper.accept(header.security_id, time_utc, price_paise) {
                                self.dropped_duplicates += 1;
                                continue;
                            }
                            return Some(FeedEvent::Tick(Tick {
                                security_id: header.security_id,
                                price_paise,
                                time_utc,
                                volume: Some(volume as i64),
                                recv_ns,
                            }));
                        }
                        Packet::PrevClose { close, oi } => {
                            return Some(FeedEvent::PrevClose {
                                security_id: header.security_id,
                                price_paise: ltp_to_paise(close),
                                oi,
                            });
                        }
                        Packet::Disconnect { reason } => {
                            return Some(FeedEvent::Disconnected {
                                reason: Some(reason),
                            });
                        }
                        Packet::Oi { .. } | Packet::Unknown { .. } => {
                            // Ignored / passed over in tick stream
                            continue;
                        }
                    }
                }
                Err(_) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_replay_source_mock() {
        let jsonl = r#"{"t": 1700000000.5, "hex": "02100001450b00009a99a24400f15365"}
{"t": 1700000001.0, "hex": "02100001450b00009a99a24400f15365"}
{"t": 1700000001.5, "hex": "06100001450b00009a19a84400000000"}
{"t": 1700000002.0, "hex": "02100001450b00000000aa4401f15365"}
"#;
        let mut source = ReplaySource::from_reader(Cursor::new(jsonl.as_bytes()));

        // First event is Connected
        assert_eq!(source.next(), Some(FeedEvent::Connected));

        // First tick
        let ev1 = source.next().unwrap();
        match ev1 {
            FeedEvent::Tick(t) => {
                assert_eq!(t.security_id, 2885);
                assert_eq!(t.price_paise, 130080);
                assert_eq!(t.time_utc, 1700000000 - 19800);
                assert_eq!(t.recv_ns, 1700000000500000000);
            }
            _ => panic!("expected tick"),
        }

        // Second tick in jsonl is identical (same time & price) -> dropped by deduper
        // Next event is PrevClose
        let ev2 = source.next().unwrap();
        match ev2 {
            FeedEvent::PrevClose {
                security_id,
                price_paise,
                ..
            } => {
                assert_eq!(security_id, 2885);
                assert_eq!(price_paise, 134480);
            }
            _ => panic!("expected PrevClose"),
        }

        // Next is third tick (new timestamp)
        let ev3 = source.next().unwrap();
        match ev3 {
            FeedEvent::Tick(t) => {
                assert_eq!(t.security_id, 2885);
                assert_eq!(t.time_utc, 1700000001 - 19800);
            }
            _ => panic!("expected tick"),
        }

        // EOF
        assert_eq!(source.next(), None);
        assert_eq!(source.dropped_duplicates, 1);
        assert_eq!(source.total_frames, 4);
    }
}
