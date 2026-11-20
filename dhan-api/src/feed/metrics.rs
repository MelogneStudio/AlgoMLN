use crate::feed::normalize::LttConvention;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct ConnectionMetrics {
    pub packets_by_code: HashMap<u8, u64>,
    pub decode_errors: u64,
    pub unknown_codes: u64,
    pub deduped_ticks: u64,
    pub late_ticks: u64,
    pub dropped_ticks: u64,
    pub stale_ticks: u64,
    pub zero_ltt_ticks: u64,
    pub reconnects: u64,
    pub detected_convention: Option<LttConvention>,
    pub last_packet_recv_time: Option<f64>,
    pub last_ltt: Option<i64>,
    pub last_recv_minus_ltt: Option<f64>,
    pub latencies_sec: Vec<f64>, // receive_time - corrected_ltt
}

impl ConnectionMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_packet(&mut self, code: u8, recv_t: f64) {
        *self.packets_by_code.entry(code).or_insert(0) += 1;
        self.last_packet_recv_time = Some(recv_t);
    }

    pub fn record_latency(&mut self, recv_t: f64, time_utc: i64) {
        let lat = recv_t - time_utc as f64;
        if self.latencies_sec.len() < 10_000 {
            self.latencies_sec.push(lat);
        }
    }

    pub fn latency_stats(&self) -> Option<(f64, f64)> {
        // Returns (p50, max)
        if self.latencies_sec.is_empty() {
            return None;
        }
        let mut sorted = self.latencies_sec.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p50 = sorted[sorted.len() / 2];
        let max = *sorted.last().unwrap();
        Some((p50, max))
    }

    pub fn status_line(&self) -> String {
        let mut codes: Vec<_> = self.packets_by_code.iter().collect();
        codes.sort_by_key(|(k, _)| *k);
        let codes_str: Vec<String> = codes
            .into_iter()
            .map(|(k, v)| format!("code_{k}={v}"))
            .collect();
        let ltt_str = match self.last_ltt {
            Some(ltt) => format!("{ltt}"),
            None => "none".to_string(),
        };
        let recv_minus_ltt_str = match self.last_recv_minus_ltt {
            Some(diff) => format!("{diff:.3}s"),
            None => "none".to_string(),
        };
        let conv_str = match self.detected_convention {
            Some(c) => format!("{c:?}"),
            None => "unknown".to_string(),
        };
        format!(
            "FEED STATUS | packets: [{}] | last_ltt: {} | recv-ltt: {} | convention: {} | stale: {} | zero_ltt: {} | drops: {} | reconnects: {}",
            codes_str.join(", "),
            ltt_str,
            recv_minus_ltt_str,
            conv_str,
            self.stale_ticks,
            self.zero_ltt_ticks,
            self.dropped_ticks,
            self.reconnects,
        )
    }
}
