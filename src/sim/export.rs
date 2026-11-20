use std::{
    collections::BTreeMap,
    fs::File,
    path::PathBuf,
};

use crate::trader::{OrderReason, TraderEvent};
use super::types::{round_half_even_i128, RunResult, SymbolEquityPoint, Trade};

/// Write deterministic, machine-readable run artifacts into an output folder.
pub fn write_results(dir: impl Into<PathBuf>, result: &RunResult) -> Result<(), std::io::Error> {
    let dir = dir.into();
    std::fs::create_dir_all(&dir)?;
    let mut f = csv::Writer::from_writer(File::create(dir.join("fills.csv"))?);
    csv_record(
        &mut f,
        [
            "ts",
            "symbol",
            "order_id",
            "side",
            "qty",
            "price",
            "price_paise",
            "fees",
            "fees_paise",
            "reason",
        ]
        .map(str::to_owned)
        .to_vec(),
    )?;
    for x in &result.fills {
        csv_record(
            &mut f,
            vec![
                x.ts.to_string(),
                x.symbol.clone(),
                x.order_id.clone(),
                format!("{:?}", x.side),
                x.qty.to_string(),
                format!("{:.10}", x.price),
                x.price_paise.to_string(),
                format!("{:.10}", x.fees),
                x.fees_paise.to_string(),
                format!("{:?}", x.reason),
            ],
        )?;
    }
    f.flush()?;
    let mut e = csv::Writer::from_writer(File::create(dir.join("equity.csv"))?);
    csv_record(
        &mut e,
        [
            "ts",
            "cash",
            "cash_paise",
            "positions_value",
            "positions_value_paise",
            "equity",
            "equity_paise",
        ]
        .map(str::to_owned)
        .to_vec(),
    )?;
    for x in &result.equity {
        csv_record(
            &mut e,
            vec![
                x.ts.to_string(),
                format!("{:.10}", x.cash),
                x.cash_paise.to_string(),
                format!("{:.10}", x.positions_value),
                x.positions_value_paise.to_string(),
                format!("{:.10}", x.equity),
                x.equity_paise.to_string(),
            ],
        )?;
    }
    e.flush()?;
    std::fs::write(
        dir.join("open_positions.json"),
        to_json_bytes(&result.open_positions)?,
    )?;
    std::fs::write(
        dir.join("events.json"),
        to_json_bytes(&result.events)?,
    )?;
    std::fs::write(
        dir.join("manifest.json"),
        to_json_bytes(&result.manifest)?,
    )?;
    let start = result
        .manifest
        .pointer("/config/initial_cash")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| result.equity.first().map(|x| x.equity))
        .unwrap_or(0.0);
    let start_paise = crate::trader::rupees_to_paise(start);
    let end_paise = result.equity.last().map_or(start_paise, |x| x.equity_paise);
    let mut peak_paise = start_paise;
    let mut max_dd = 0.0f64;
    for p in &result.equity {
        peak_paise = peak_paise.max(p.equity_paise);
        if peak_paise > 0 {
            max_dd = max_dd.max((peak_paise - p.equity_paise) as f64 / peak_paise as f64 * 100.0);
        }
    }
    let metrics = build_metrics(result, start_paise, end_paise, max_dd);
    std::fs::write(
        dir.join("metrics.json"),
        to_json_bytes(&metrics)?,
    )?;
    let mut t = csv::Writer::from_writer(File::create(dir.join("trades.csv"))?);
    csv_record(
        &mut t,
        [
            "symbol",
            "entry_ts",
            "exit_ts",
            "qty",
            "avg_entry",
            "avg_entry_paise",
            "avg_exit",
            "avg_exit_paise",
            "fees",
            "fees_paise",
            "net_pnl",
            "net_pnl_paise",
            "exit_reason",
        ]
        .map(str::to_owned)
        .to_vec(),
    )?;
    for tr in &result.trades {
        csv_record(
            &mut t,
            vec![
                tr.symbol.clone(),
                tr.entry_ts.to_string(),
                tr.exit_ts.to_string(),
                tr.qty.to_string(),
                format!("{:.10}", tr.avg_entry),
                tr.avg_entry_paise.to_string(),
                format!("{:.10}", tr.avg_exit),
                tr.avg_exit_paise.to_string(),
                format!("{:.10}", tr.fees),
                tr.fees_paise.to_string(),
                format!("{:.10}", tr.net_pnl),
                tr.net_pnl_paise.to_string(),
                format!("{:?}", tr.exit_reason),
            ],
        )?;
    }
    t.flush()?;
    Ok(())
}

fn csv_record(writer: &mut csv::Writer<File>, fields: Vec<String>) -> Result<(), std::io::Error> {
    writer.write_record(fields).map_err(csv_io_error)
}

fn csv_io_error(error: csv::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

fn to_json_bytes<T: serde::Serialize>(val: &T) -> Result<Vec<u8>, std::io::Error> {
    serde_json::to_vec_pretty(val).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn build_metrics(
    result: &RunResult,
    start_paise: i64,
    end_paise: i64,
    portfolio_dd: f64,
) -> serde_json::Value {
    let exposure = |series: &[SymbolEquityPoint]| {
        let total = series
            .last()
            .zip(series.first())
            .map_or(0.0, |(b, a)| (b.ts - a.ts).max(0) as f64);
        let held: f64 = series
            .windows(2)
            .filter(|w| w[0].position_qty > 0)
            .map(|w| (w[1].ts - w[0].ts).max(0) as f64)
            .sum();
        if total == 0.0 {
            0.0
        } else {
            held / total * 100.0
        }
    };
    let stats = |trades: Vec<&Trade>| {
        let wins: Vec<_> = trades.iter().filter(|t| t.net_pnl_paise > 0).collect();
        let losses: Vec<_> = trades.iter().filter(|t| t.net_pnl_paise < 0).collect();
        let gross_win: i128 = wins.iter().map(|t| t.net_pnl_paise as i128).sum();
        let gross_loss: i128 = losses.iter().map(|t| -(t.net_pnl_paise as i128)).sum();
        serde_json::json!({"trades":trades.len(),"win_rate_pct":if trades.is_empty(){0.0}else{wins.len() as f64/trades.len() as f64*100.0},"profit_factor":if gross_loss==0{serde_json::Value::Null}else if gross_win==0{serde_json::json!(0.0)}else{serde_json::json!(gross_win as f64/gross_loss as f64)},"avg_win":if wins.is_empty(){0.0}else{crate::trader::paise_to_rupees(round_half_even_i128(gross_win, wins.len() as i128) as i64)},"avg_loss":if losses.is_empty(){0.0}else{crate::trader::paise_to_rupees(round_half_even_i128(gross_loss, losses.len() as i128) as i64)}})
    };
    let mut symbol_metrics = serde_json::Map::new();
    let n = result.symbol_equity.len().max(1) as i64;
    let allocation_base = start_paise / n;
    let allocation_remainder = start_paise.rem_euclid(n);
    let mut trades_by_symbol = BTreeMap::<&str, Vec<&Trade>>::new();
    for t in &result.trades {
        trades_by_symbol.entry(t.symbol.as_str()).or_default().push(t);
    }
    for (index, (symbol, series)) in result.symbol_equity.iter().enumerate() {
        let allocation_paise = allocation_base + i64::from((index as i64) < allocation_remainder);
        let final_eq_paise = series.last().map_or(allocation_paise, |p| p.equity_paise);
        let mut peak_paise = allocation_paise;
        let mut dd = 0.0f64;
        for p in series {
            peak_paise = peak_paise.max(p.equity_paise);
            if peak_paise > 0 {
                dd = dd.max((peak_paise - p.equity_paise) as f64 / peak_paise as f64 * 100.0)
            }
        }
        let ts = trades_by_symbol.remove(symbol.as_str()).unwrap_or_default();
        let mut item = stats(ts);
        if let Some(obj) = item.as_object_mut() {
            let net_pnl_paise = final_eq_paise - allocation_paise;
            obj.insert(
                "net_pnl".into(),
                serde_json::json!(crate::trader::paise_to_rupees(net_pnl_paise)),
            );
            obj.insert("net_pnl_paise".into(), serde_json::json!(net_pnl_paise));
            obj.insert(
                "return_pct".into(),
                serde_json::json!(if allocation_paise == 0 {
                    0.0
                } else {
                    net_pnl_paise as f64 / allocation_paise as f64 * 100.0
                }),
            );
            obj.insert("max_drawdown_pct".into(), serde_json::json!(dd));
            obj.insert("exposure_pct".into(), serde_json::json!(exposure(series)));
        }
        symbol_metrics.insert(symbol.clone(), item);
    }
    let mut blocked = BTreeMap::<String, u64>::new();
    let mut skipped = 0;
    let mut breach = 0;
    let mut rejects = 0;
    for ev in &result.events {
        match ev {
            TraderEvent::RuleSkipped { .. } => skipped += 1,
            TraderEvent::RiskBlocked { limit, .. } => {
                *blocked.entry((*limit).to_owned()).or_default() += 1
            }
            TraderEvent::DailyLossBreached => breach += 1,
            TraderEvent::OrderRejected { .. } => rejects += 1,
            _ => {}
        }
    }
    let flatten_orders = result
        .fills
        .iter()
        .filter(|f| f.reason == OrderReason::DailyLossFlatten)
        .map(|f| f.order_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let portfolio_trades: Vec<_> = result.trades.iter().collect();
    let mut p = stats(portfolio_trades);
    let mut held_seconds = 0.0;
    let mut total_seconds = 0.0;
    if let Some(first) = result.equity.first() {
        if let Some(last) = result.equity.last() {
            total_seconds = (last.ts - first.ts).max(0) as f64;
            let series_list: Vec<&[SymbolEquityPoint]> =
                result.symbol_equity.values().map(Vec::as_slice).collect();
            let mut cursors = vec![0usize; series_list.len()];
            for w in result.equity.windows(2) {
                let ts = w[0].ts;
                let mut any = false;
                for (idx, series) in series_list.iter().enumerate() {
                    while cursors[idx] < series.len() && series[cursors[idx]].ts < ts {
                        cursors[idx] += 1;
                    }
                    if cursors[idx] < series.len()
                        && series[cursors[idx]].ts == ts
                        && series[cursors[idx]].position_qty > 0
                    {
                        any = true;
                    }
                }
                if any {
                    held_seconds += (w[1].ts - w[0].ts).max(0) as f64;
                }
            }
        }
    }
    if let Some(obj) = p.as_object_mut() {
        let net_pnl_paise = end_paise - start_paise;
        obj.insert(
            "net_pnl".into(),
            serde_json::json!(crate::trader::paise_to_rupees(net_pnl_paise)),
        );
        obj.insert("net_pnl_paise".into(), serde_json::json!(net_pnl_paise));
        obj.insert("start_equity_paise".into(), serde_json::json!(start_paise));
        obj.insert("end_equity_paise".into(), serde_json::json!(end_paise));
        obj.insert(
            "return_pct".into(),
            serde_json::json!(if start_paise == 0 {
                0.0
            } else {
                net_pnl_paise as f64 / start_paise as f64 * 100.0
            }),
        );
        obj.insert("max_drawdown_pct".into(), serde_json::json!(portfolio_dd));
        obj.insert(
            "exposure_pct".into(),
            serde_json::json!(if total_seconds == 0.0 {
                0.0
            } else {
                held_seconds / total_seconds * 100.0
            }),
        );
        obj.insert(
            "open_positions".into(),
            serde_json::json!(result.open_positions.len()),
        );
        obj.insert("rules_skipped".into(), serde_json::json!(skipped));
        obj.insert("entries_blocked_by_gate".into(), serde_json::json!(blocked));
        obj.insert("daily_loss_breaches".into(), serde_json::json!(breach));
        obj.insert("flattens".into(), serde_json::json!(flatten_orders));
        obj.insert("rejects".into(), serde_json::json!(rejects));
        obj.insert(
            "missing_candles".into(),
            serde_json::to_value(&result.source_stats.missing_candles).unwrap_or_default(),
        );
        obj.insert(
            "skipped_bad".into(),
            serde_json::to_value(&result.source_stats.skipped_bad).unwrap_or_default(),
        );
    }
    serde_json::json!({"portfolio": p, "symbols": symbol_metrics})
}
