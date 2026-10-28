use super::{
    Candle, IncrementalProvider, IndicatorProvider, IndicatorSpec, IndicatorType, PriceField,
};

fn candles() -> &'static [Candle] {
    static INPUT: std::sync::OnceLock<Vec<Candle>> = std::sync::OnceLock::new();
    INPUT.get_or_init(|| {
        include_str!("../../sample-data/nifty_1min.csv")
            .lines()
            .skip(1)
            .enumerate()
            .map(|(i, row)| {
                let mut columns = row.split(',');
                let _timestamp = columns.next().unwrap();
                let mut number = || columns.next().unwrap().parse::<f64>().unwrap();
                let open = number();
                let high = number();
                let low = number();
                let close = number();
                // The source has five rounding discrepancies of less than 0.001 where
                // close falls just outside the quoted low/high bounds.
                let high = high.max(close);
                let low = low.min(open).min(close);
                Candle {
                    timestamp: i as i64 * 60_000,
                    open,
                    high,
                    low,
                    close,
                    // The source CSV has no volume column. Zero is explicit so volume-based
                    // outputs are undefined instead of silently inventing market data.
                    volume: 0.0,
                }
            })
            .collect()
    })
}

fn check_fixture(kind: IndicatorType, period: u32, name: &str, fixture: &str) {
    let mut provider = IncrementalProvider::new();
    let handle = provider
        .register(IndicatorSpec::new(kind, period, None).unwrap())
        .unwrap();
    let rows: Vec<_> = fixture.lines().skip(1).collect();
    let input = candles();
    assert_eq!(
        rows.len(),
        input.len(),
        "fixture length for {name}_{period}"
    );
    for (i, (candle, row)) in input.iter().zip(rows).enumerate() {
        provider.advance(candle).unwrap();
        let expected = if row == "None" {
            None
        } else {
            Some(row.parse::<f64>().unwrap())
        };
        let actual = provider.get(handle);
        match (actual, expected) {
            (None, None) => {}
            (Some(a), Some(e)) => {
                let tolerance = 1e-9 * e.abs().max(1.0);
                assert!(
                    (a - e).abs() <= tolerance,
                    "{name}({period}) at row {i}: {a} != {e}"
                );
            }
            pair => panic!("{name}({period}) at row {i}: got {pair:?}"),
        }
    }
}

macro_rules! golden_periods {
    ($kind:ident, $name:literal) => {
        check_fixture(
            IndicatorType::$kind,
            1,
            $name,
            include_str!(concat!("tests/fixtures/", $name, "_1.csv")),
        );
        check_fixture(
            IndicatorType::$kind,
            2,
            $name,
            include_str!(concat!("tests/fixtures/", $name, "_2.csv")),
        );
        check_fixture(
            IndicatorType::$kind,
            5,
            $name,
            include_str!(concat!("tests/fixtures/", $name, "_5.csv")),
        );
        check_fixture(
            IndicatorType::$kind,
            14,
            $name,
            include_str!(concat!("tests/fixtures/", $name, "_14.csv")),
        );
        check_fixture(
            IndicatorType::$kind,
            20,
            $name,
            include_str!(concat!("tests/fixtures/", $name, "_20.csv")),
        );
        check_fixture(
            IndicatorType::$kind,
            50,
            $name,
            include_str!(concat!("tests/fixtures/", $name, "_50.csv")),
        );
    };
}

#[test]
fn matches_pandas_golden_fixtures() {
    golden_periods!(Sma, "sma");
    golden_periods!(Ema, "ema");
    golden_periods!(Rsi, "rsi");
    golden_periods!(Atr, "atr");
    golden_periods!(BbUpper, "bb_upper");
    golden_periods!(BbMid, "bb_mid");
    golden_periods!(BbLower, "bb_lower");
}

#[test]
fn provider_price_and_volume_fields_follow_csv_assumptions() {
    let mut provider = IncrementalProvider::new();
    let input = candles();
    provider.advance(&input[0]).unwrap();
    assert_eq!(provider.price(PriceField::Volume), Some(0.0));
    assert_eq!(provider.price(PriceField::PrevClose), None);
    provider.advance(&input[1]).unwrap();
    assert_eq!(provider.price(PriceField::PrevClose), Some(input[0].close));
}

fn sample(timestamp: i64, close: f64, volume: f64) -> Candle {
    Candle {
        timestamp,
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume,
    }
}

#[test]
fn volume_indicators_use_prior_window_and_reset_at_utc_day_boundary() {
    let mut provider = IncrementalProvider::new();
    let rel = provider
        .register(IndicatorSpec::new(IndicatorType::RelVol, 2, None).unwrap())
        .unwrap();
    let vwap = provider
        .register(IndicatorSpec::new(IndicatorType::Vwap, 20, None).unwrap())
        .unwrap();
    let day = 86_400_000;
    provider.advance(&sample(day - 120_000, 10.0, 2.0)).unwrap();
    assert_eq!(provider.get(rel), None);
    assert_eq!(provider.get(vwap), Some(10.0));
    provider.advance(&sample(day - 60_000, 12.0, 4.0)).unwrap();
    assert_eq!(provider.get(rel), None);
    assert_eq!(provider.get(vwap), Some((10.0 * 2.0 + 12.0 * 4.0) / 6.0));
    provider.advance(&sample(day, 20.0, 9.0)).unwrap();
    assert_eq!(provider.get(rel), Some(3.0));
    assert_eq!(provider.get(vwap), Some(20.0));
}

#[test]
fn invalid_candles_do_not_mutate_provider_and_specs_dedupe() {
    let mut provider = IncrementalProvider::new();
    let sma_spec = IndicatorSpec::new(IndicatorType::Sma, 1, None).unwrap();
    let h = provider.register(sma_spec).unwrap();
    assert_eq!(provider.register(sma_spec).unwrap(), h);
    let bb_default = IndicatorSpec::new(IndicatorType::BbUpper, 20, None).unwrap();
    let bb_explicit = IndicatorSpec::new(IndicatorType::BbUpper, 20, Some(2.0)).unwrap();
    assert_eq!(bb_default, bb_explicit);
    assert_eq!(
        provider.register(bb_default).unwrap(),
        provider.register(bb_explicit).unwrap()
    );
    provider.advance(&sample(1, 100.0, 1.0)).unwrap();
    assert_eq!(provider.get(h), Some(100.0));
    let invalid = Candle {
        high: 1.0,
        low: 2.0,
        ..sample(2, 100.0, 1.0)
    };
    assert!(provider.advance(&invalid).is_err());
    assert_eq!(provider.get(h), Some(100.0));
    assert_eq!(provider.price(PriceField::Close), Some(100.0));
}

#[test]
fn period_one_and_short_large_periods_follow_warmup_rules() {
    let mut provider = IncrementalProvider::new();
    let sma = provider
        .register(IndicatorSpec::new(IndicatorType::Sma, 1, None).unwrap())
        .unwrap();
    let ema = provider
        .register(IndicatorSpec::new(IndicatorType::Ema, 1, None).unwrap())
        .unwrap();
    let rsi = provider
        .register(IndicatorSpec::new(IndicatorType::Rsi, 1, None).unwrap())
        .unwrap();
    let atr = provider
        .register(IndicatorSpec::new(IndicatorType::Atr, 1, None).unwrap())
        .unwrap();
    let bb = provider
        .register(IndicatorSpec::new(IndicatorType::BbUpper, 1, None).unwrap())
        .unwrap();
    let rel = provider
        .register(IndicatorSpec::new(IndicatorType::RelVol, 1, None).unwrap())
        .unwrap();
    let large = provider
        .register(IndicatorSpec::new(IndicatorType::Sma, 10_000, None).unwrap())
        .unwrap();
    provider.advance(&sample(1, 10.0, 2.0)).unwrap();
    assert_eq!(provider.get(sma), Some(10.0));
    assert_eq!(provider.get(ema), Some(10.0));
    assert_eq!(provider.get(bb), Some(10.0));
    assert_eq!(provider.get(rsi), None);
    assert_eq!(provider.get(atr), None);
    assert_eq!(provider.get(rel), None);
    assert_eq!(provider.get(large), None);
    provider.advance(&sample(2, 10.0, 4.0)).unwrap();
    assert_eq!(provider.get(rsi), Some(50.0));
    assert_eq!(provider.get(atr), Some(2.0));
    assert_eq!(provider.get(rel), Some(2.0));
}

#[test]
fn reset_replay_reproduces_values_and_preserves_handles() {
    let mut provider = IncrementalProvider::new();
    let handle = provider
        .register(IndicatorSpec::new(IndicatorType::Ema, 3, None).unwrap())
        .unwrap();
    let input = [
        sample(1, 10.0, 1.0),
        sample(2, 11.0, 1.0),
        sample(3, 9.0, 1.0),
    ];
    let mut first = Vec::new();
    for candle in &input {
        provider.advance(candle).unwrap();
        first.push(provider.get(handle));
    }
    provider.reset();
    let mut replay = Vec::new();
    for candle in &input {
        provider.advance(candle).unwrap();
        replay.push(provider.get(handle));
    }
    assert_eq!(first, replay);
}
