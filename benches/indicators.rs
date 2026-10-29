use algomln::indicators::{
    Candle, IncrementalProvider, IndicatorProvider, IndicatorSpec, IndicatorType,
};
use algomln::{Engine, Program};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

fn candles() -> Vec<Candle> {
    include_str!("../sample-data/nifty_1min.csv")
        .lines()
        .skip(1)
        .enumerate()
        .map(|(i, row)| {
            let mut fields = row.split(',');
            let _ = fields.next();
            let mut n = || fields.next().unwrap().parse().unwrap();
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
                volume: 0.0,
            }
        })
        .collect()
}

fn full_indicator_set(c: &mut Criterion) {
    let input = candles();
    let mut group = c.benchmark_group("incremental_full_set");
    group.throughput(Throughput::Elements(input.len() as u64));
    group.bench_function("nifty_1min_184863", |b| {
        b.iter(|| {
            let mut provider = IncrementalProvider::new();
            for (kind, period) in [
                (IndicatorType::Sma, 20),
                (IndicatorType::Ema, 20),
                (IndicatorType::Rsi, 14),
                (IndicatorType::Atr, 14),
                (IndicatorType::BbUpper, 20),
                (IndicatorType::BbMid, 20),
                (IndicatorType::BbLower, 20),
                (IndicatorType::RelVol, 20),
                (IndicatorType::Vwap, 0),
            ] {
                provider
                    .register(IndicatorSpec::new(kind, period, None).unwrap())
                    .unwrap();
            }
            for candle in &input {
                provider.advance(black_box(candle)).unwrap();
            }
            black_box(provider)
        })
    });
    group.finish();
}

fn condition_engine(c: &mut Criterion) {
    let input = candles();
    let program=Program::new(algomln::compile("TRADE_IN NIFTY\nWHEN ema(20) > sma(50) AND rsi(14) < 70\nBUY 1\nWHEN CROSS_BELOW(close, ema(20))\nSELL ALL\n").unwrap().strategy().clone());
    let mut group = c.benchmark_group("condition_engine_nifty");
    group.throughput(Throughput::Elements(input.len() as u64));
    group.bench_function("nifty_1min_184863", |b| {
        b.iter(|| {
            let mut engine = Engine::new(&program, "NIFTY".into()).unwrap();
            for candle in &input {
                black_box(engine.on_candle(black_box(candle)).unwrap());
            }
        })
    });
    group.finish();
}

criterion_group!(benches, full_indicator_set, condition_engine);
criterion_main!(benches);
