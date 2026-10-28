use algomln::indicators::{
    Candle, IncrementalProvider, IndicatorProvider, IndicatorSpec, IndicatorType,
};
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

criterion_group!(benches, full_indicator_set);
criterion_main!(benches);
