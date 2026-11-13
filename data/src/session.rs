use crate::catalog::Catalog;
use crate::error::DataError;
use crate::types::{Bar, Tf};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use chrono_tz::Asia::Kolkata;
use std::collections::{BTreeMap, HashMap};

pub fn missing_date_ranges(
    cat: &Catalog,
    id: &str,
    tf: Tf,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<(NaiveDate, NaiveDate)>, DataError> {
    let missing = cat.missing_dates(id, tf, from, to)?;
    Ok(match (missing.first(), missing.last()) {
        (Some(a), Some(b)) => vec![(*a, *b)],
        _ => Vec::new(),
    })
}

pub fn ts_to_ist_date(ts: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .with_timezone(&Kolkata)
        .date_naive()
}

pub fn local_midnight(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp()
}

pub fn chunk_dates(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    chunk_dates_limited(from, to, 89)
}

pub fn chunk_dates_limited(
    from: NaiveDate,
    to: NaiveDate,
    days: u64,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut s = from;
    while s <= to {
        let e = (s + chrono::Days::new(days.saturating_sub(1))).min(to);
        out.push((s, e));
        s = e + chrono::Days::new(1);
    }
    out
}

pub fn last_complete_bar(now: i64, tf: Tf, _session_end: i64, margin: i64) -> i64 {
    let step = tf.seconds();
    let local = DateTime::<Utc>::from_timestamp(now, 0)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .with_timezone(&Kolkata);
    if tf == Tf::D1 {
        let cutoff = local
            .date_naive()
            .and_hms_opt(16, 0, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let target_date = if now >= cutoff {
            local.date_naive()
        } else {
            local.date_naive() - chrono::Days::new(1)
        };
        return target_date
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
    }
    let last = now - margin - step;
    last - last.rem_euclid(step)
}

pub fn complete_intraday(
    bars: Vec<Bar>,
    now: i64,
    tf: Tf,
    session_ends: &HashMap<String, i64>,
    margin: i64,
) -> Vec<Bar> {
    bars.into_iter()
        .filter(|b| {
            let local = DateTime::<Utc>::from_timestamp(b.ts, 0)
                .unwrap_or(DateTime::UNIX_EPOCH)
                .with_timezone(&Kolkata);
            let end = session_ends
                .get(&local.date_naive().to_string())
                .copied()
                .unwrap_or_else(|| {
                    local
                        .date_naive()
                        .and_hms_opt(15, 30, 0)
                        .unwrap()
                        .and_local_timezone(Kolkata)
                        .single()
                        .unwrap()
                        .timestamp()
                });
            now >= ((b.ts + tf.seconds()).min(end) + margin)
        })
        .collect()
}

pub fn session_shapes(bars: &[Bar]) -> BTreeMap<NaiveDate, (i64, i64, usize)> {
    let mut m: BTreeMap<NaiveDate, Vec<i64>> = BTreeMap::new();
    for b in bars {
        let d = DateTime::<Utc>::from_timestamp(b.ts, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata)
            .date_naive();
        m.entry(d).or_default().push(b.ts);
    }
    m.into_iter()
        .map(|(d, mut xs)| {
            xs.sort();
            (d, (xs[0], *xs.last().unwrap(), xs.len()))
        })
        .collect()
}

pub fn session_regime_changes(shapes: &BTreeMap<NaiveDate, (i64, i64, usize)>) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut history = Vec::<i64>::new();
    let mut prior = None;
    for (d, (_, last, _)) in shapes {
        let local = DateTime::<Utc>::from_timestamp(*last, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata);
        let value = local.hour() as i64 * 3600 + local.minute() as i64 * 60 + local.second() as i64;
        history.push(value);
        if history.len() > 20 {
            history.remove(0);
        }
        let mut counts = HashMap::<i64, usize>::new();
        for x in &history {
            *counts.entry(*x).or_default() += 1;
        }
        let mode = counts
            .into_iter()
            .max_by_key(|(v, n)| (*n, std::cmp::Reverse(*v)))
            .map(|(v, _)| v);
        if let (Some(old), Some(new)) = (prior, mode) {
            if old != new {
                out.push(*d)
            }
        }
        prior = mode;
    }
    out
}

pub fn check_overlap(old: &[Bar], new: &[Bar]) -> (&'static str, Vec<i64>) {
    let oldm: HashMap<_, _> = old.iter().map(|b| (b.ts, b)).collect();
    let mut diffs = Vec::new();
    let mut ratios = Vec::new();
    let mut overlap = 0usize;
    for n in new {
        if let Some(o) = oldm.get(&n.ts) {
            overlap += 1;
            if o != &n {
                diffs.push(n.ts);
            }
            if n.close != 0.0 {
                ratios.push(o.close / n.close);
            }
        }
    }
    if diffs.is_empty() {
        return ("identical", diffs);
    }
    let avg = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
    if ratios.len() == overlap
        && ratios.iter().all(|x| (x - avg).abs() < 1e-6)
        && (avg - 1.0).abs() > 1e-4
    {
        return ("corporate_action", diffs);
    }
    ("revised", diffs)
}
