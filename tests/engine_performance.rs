use algomln::{indicators::Candle, Engine, Program};
use std::hint::black_box;

fn candles() -> Vec<Candle> {
    include_str!("../sample-data/nifty_1min.csv")
        .lines()
        .skip(1)
        .enumerate()
        .map(|(i, row)| {
            let mut fields = row.split(',');
            let _ = fields.next();
            let mut n = || fields.next().unwrap().parse::<f64>().unwrap();
            let open = n();
            let high = n();
            let low = n();
            let close = n();
            Candle {
                timestamp: i as i64 * 60_000,
                open,
                high: high.max(close),
                low: low.min(open).min(close),
                close,
                volume: 0.,
            }
        })
        .collect()
}

#[test]
#[ignore = "timing-sensitive; run explicitly in release CI"]
fn condition_engine_nifty_baseline_regression_guard() {
    let input = candles();
    let program=Program::new(algomln::compile("TRADE_IN NIFTY\nWHEN ema(20) > sma(50) AND rsi(14) < 70\nBUY 1\nWHEN CROSS_BELOW(close, ema(20))\nSELL ALL\n").unwrap().strategy().clone());
    let start = std::time::Instant::now();
    let mut engine = Engine::new(&program, "NIFTY".into()).unwrap();
    for c in &input {
        black_box(engine.on_candle(black_box(c)).unwrap());
    }
    let rate = input.len() as f64 / start.elapsed().as_secs_f64();
    assert!(
        rate >= 1_211_130.0,
        "{rate:.0} candles/s is >10% below 1,345,700 baseline"
    );
}
