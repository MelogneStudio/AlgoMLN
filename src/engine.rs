use crate::{
    ast::*,
    indicators::{
        Candle, IndicatorError, IndicatorHandle, IndicatorProvider, IndicatorSpec,
        PrecomputedProvider, PriceField as PField,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

pub type RuleId = String;
pub type Symbol = String;
#[derive(Clone, Debug)]
pub struct Program {
    strategy: Strategy,
}
impl Program {
    pub fn new(strategy: Strategy) -> Self {
        Self { strategy }
    }
    pub fn rules(&self) -> &[Rule] {
        &self.strategy.rules
    }
    pub fn stop_loss(&self) -> Option<&Decimal> {
        self.strategy.stop_loss.as_ref()
    }
    pub fn take_profit(&self) -> Option<&Decimal> {
        self.strategy.take_profit.as_ref()
    }
    pub fn risk(&self) -> &RiskConfig {
        &self.strategy.risk
    }
    pub fn symbols(&self) -> Vec<&str> {
        match &self.strategy.trade_in {
            TradeIn::Symbols(s) => s.iter().map(String::as_str).collect(),
            TradeIn::Index(s) => vec![s],
        }
    }
    pub fn strategy(&self) -> &Strategy {
        &self.strategy
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct LevelFrame {
    pub symbol: Symbol,
    pub candle_ts: i64,
    pub warm: bool,
    pub entry: BTreeMap<RuleId, bool>,
    pub exit: BTreeMap<RuleId, bool>,
}
#[derive(Debug)]
pub enum EngineError {
    Indicator(IndicatorError),
    NonMonotonicTs { previous: i64, current: i64 },
    InvalidProgram(&'static str),
}
impl From<IndicatorError> for EngineError {
    fn from(e: IndicatorError) -> Self {
        Self::Indicator(e)
    }
}
impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EngineError {}
pub struct Engine {
    symbol: Symbol,
    provider: PrecomputedProvider,
    handles: HashMap<IndicatorSpec, IndicatorHandle>,
    previous_ts: Option<i64>,
    cross_prev: HashMap<(NumericExpr, NumericExpr), (Option<f64>, Option<f64>)>,
    program: Program,
}
impl Engine {
    pub fn new(program: &Program, symbol: Symbol) -> Result<Self, EngineError> {
        let mut provider = PrecomputedProvider::new();
        let mut handles = HashMap::new();
        fn reg(
            c: &Condition,
            p: &mut PrecomputedProvider,
            h: &mut HashMap<IndicatorSpec, IndicatorHandle>,
        ) -> Result<(), EngineError> {
            match c {
                Condition::Compare { left, right, .. } | Condition::Cross { left, right, .. } => {
                    for e in [left, right] {
                        if let NumericExpr::Indicator(i) = e {
                            let s = IndicatorSpec::from_call(i)?;
                            if let std::collections::hash_map::Entry::Vacant(entry) = h.entry(s) {
                                let n = p.register(s)?;
                                entry.insert(n);
                            }
                        }
                    }
                    Ok(())
                }
                Condition::And(a, b) | Condition::Or(a, b) => {
                    reg(a, p, h)?;
                    reg(b, p, h)
                }
                Condition::Not(a) => reg(a, p, h),
                _ => Ok(()),
            }
        }
        for r in &program.strategy.rules {
            reg(&r.condition, &mut provider, &mut handles)?;
        }
        Ok(Self {
            symbol,
            provider,
            handles,
            previous_ts: None,
            cross_prev: HashMap::new(),
            program: program.clone(),
        })
    }
    pub fn on_candle(&mut self, c: &Candle) -> Result<LevelFrame, EngineError> {
        if let Some(t) = self.previous_ts {
            if c.timestamp <= t {
                return Err(EngineError::NonMonotonicTs {
                    previous: t,
                    current: c.timestamp,
                });
            }
        }
        c.is_valid(None)
            .map_err(|reason| EngineError::Indicator(IndicatorError::InvalidCandle { reason }))?;
        self.provider.advance_unchecked(c);
        let mut entry = BTreeMap::new();
        let mut exit = BTreeMap::new();
        for r in &self.program.strategy.rules {
            let v = self.eval(&r.condition).unwrap_or(false);
            match &r.action {
                Action::Buy(_) => {
                    entry.insert(r.id.clone(), v);
                }
                Action::Sell(_) | Action::SellAll => {
                    exit.insert(r.id.clone(), v);
                }
            }
        }
        let mut keys = Vec::new();
        for r in &self.program.strategy.rules {
            Self::cross_keys(&r.condition, &mut keys)
        }
        for (a, b) in keys {
            let x = self.value(&a);
            let y = self.value(&b);
            self.cross_prev.insert((a, b), (x, y));
        }
        self.previous_ts = Some(c.timestamp);
        let warm = self
            .handles
            .values()
            .all(|h| self.provider.get(*h).is_some());
        Ok(LevelFrame {
            symbol: self.symbol.clone(),
            candle_ts: c.timestamp,
            warm,
            entry,
            exit,
        })
    }
    pub fn reset(&mut self) {
        self.provider.reset();
        self.previous_ts = None;
        self.cross_prev.clear()
    }
    fn cross_keys(c: &Condition, out: &mut Vec<(NumericExpr, NumericExpr)>) {
        match c {
            Condition::Cross { left, right, .. } => out.push((left.clone(), right.clone())),
            Condition::And(a, b) | Condition::Or(a, b) => {
                Self::cross_keys(a, out);
                Self::cross_keys(b, out)
            }
            Condition::Not(a) => Self::cross_keys(a, out),
            _ => {}
        }
    }
    fn value(&self, e: &NumericExpr) -> Option<f64> {
        match e {
            NumericExpr::Literal(x) => x.spelling.parse().ok(),
            NumericExpr::Price(x) => self.provider.price(match x {
                PriceField::Close => PField::Close,
                PriceField::Open => PField::Open,
                PriceField::High => PField::High,
                PriceField::Low => PField::Low,
                PriceField::Volume => PField::Volume,
                PriceField::PrevClose => PField::PrevClose,
                PriceField::PrevOpen => PField::PrevOpen,
                PriceField::PrevHigh => PField::PrevHigh,
                PriceField::PrevLow => PField::PrevLow,
            }),
            NumericExpr::Indicator(i) => IndicatorSpec::from_call(i)
                .ok()
                .and_then(|s| self.handles.get(&s))
                .and_then(|h| self.provider.get(*h)),
        }
    }
    fn eval(&self, c: &Condition) -> Option<bool> {
        match c {
            Condition::Compare { left, op, right } => {
                self.value(left)
                    .zip(self.value(right))
                    .map(|(a, b)| match cmp3(a, b) {
                        std::cmp::Ordering::Less => {
                            matches!(op, CompareOp::Lt | CompareOp::Le | CompareOp::Ne)
                        }
                        std::cmp::Ordering::Equal => {
                            matches!(op, CompareOp::Le | CompareOp::Ge | CompareOp::Eq)
                        }
                        std::cmp::Ordering::Greater => {
                            matches!(op, CompareOp::Gt | CompareOp::Ge | CompareOp::Ne)
                        }
                    })
            }
            Condition::Cross {
                direction,
                left,
                right,
            } => {
                let (a, b) = self
                    .cross_prev
                    .get(&(left.clone(), right.clone()))
                    .copied()
                    .unwrap_or((None, None));
                let (x, y) = (self.value(left), self.value(right));
                a.zip(b)
                    .zip(x.zip(y))
                    .map(|((a, b), (x, y))| match direction {
                        CrossDirection::Above => {
                            cmp3(a, b) != std::cmp::Ordering::Greater
                                && cmp3(x, y) == std::cmp::Ordering::Greater
                        }
                        CrossDirection::Below => {
                            cmp3(a, b) != std::cmp::Ordering::Less
                                && cmp3(x, y) == std::cmp::Ordering::Less
                        }
                    })
            }
            Condition::And(a, b) => match (self.eval(a), self.eval(b)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Condition::Or(a, b) => match (self.eval(a), self.eval(b)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Condition::Not(a) => Some(!self.eval(a)?),
            _ => Some(false),
        }
    }
}
/// Evaluate a candle series through the batch provider API. The provider contract
/// is identical to streaming, so this helper preserves frame ordering and state.
pub fn evaluate_series(
    program: &Program,
    symbol: Symbol,
    candles: &[Candle],
) -> Result<Vec<LevelFrame>, EngineError> {
    let mut engine = Engine::new(program, symbol)?;
    candles.iter().map(|c| engine.on_candle(c)).collect()
}
const ABS_FLOOR: f64 = 1e-12;
fn cmp3(a: f64, b: f64) -> std::cmp::Ordering {
    if (a - b).abs() <= ABS_FLOOR.max(1e-9 * a.abs().max(b.abs())) {
        std::cmp::Ordering::Equal
    } else {
        a.total_cmp(&b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candle(ts: i64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.,
        }
    }
    fn program(src: &str) -> Program {
        crate::compile(src).unwrap()
    }
    #[test]
    fn crossover_warmup_and_non_monotonic_rejection() {
        let p=program("TRADE_IN ABC\nWHEN CROSS_ABOVE(close, ema(2))\nBUY 1\nWHEN close <= ema(2)\nSELL ALL\n");
        let mut e = Engine::new(&p, "ABC".into()).unwrap();
        let first = e.on_candle(&candle(1, 10.)).unwrap();
        assert!(!first.warm);
        assert!(!first.entry["rule_1"]);
        assert!(matches!(
            e.on_candle(&candle(1, 11.)),
            Err(EngineError::NonMonotonicTs { .. })
        ));
        let second = e.on_candle(&candle(2, 12.)).unwrap();
        assert!(second.warm);
    }
    #[test]
    fn batch_matches_stream_and_reset_replays() {
        let p = program(
            "TRADE_IN ABC\nWHEN close > 10\nBUY 1\nWHEN CROSS_BELOW(close, ema(2))\nSELL ALL\n",
        );
        let data = [candle(1, 10.), candle(2, 12.), candle(3, 8.)];
        let batch = evaluate_series(&p, "ABC".into(), &data).unwrap();
        let mut e = Engine::new(&p, "ABC".into()).unwrap();
        let stream: Vec<_> = data.iter().map(|c| e.on_candle(c).unwrap()).collect();
        assert_eq!(batch, stream);
        e.reset();
        let replay: Vec<_> = data.iter().map(|c| e.on_candle(c).unwrap()).collect();
        assert_eq!(batch, replay);
    }
    #[test]
    fn invalid_candle_does_not_advance_state() {
        let p = program("TRADE_IN ABC\nWHEN close > 1\nBUY 1\n");
        let mut e = Engine::new(&p, "ABC".into()).unwrap();
        let bad = Candle {
            timestamp: 1,
            open: 1.,
            high: 1.,
            low: 2.,
            close: 1.,
            volume: 0.,
        };
        assert!(e.on_candle(&bad).is_err());
        assert!(e.on_candle(&candle(1, 2.)).is_ok());
    }

    #[test]
    fn missing_indicators_remain_false_under_not_and_boolean_nesting() {
        let p = program(
            "TRADE_IN ABC\nWHEN NOT (sma(5) > 1)\nBUY 1\nWHEN sma(5) > 1 OR close > 1\nBUY 1\n",
        );
        let mut e = Engine::new(&p, "ABC".into()).unwrap();
        let f = e.on_candle(&candle(1, 2.)).unwrap();
        assert!(!f.entry["rule_1"]);
        assert!(f.entry["rule_2"]);
    }

    #[test]
    fn tolerant_comparison_operators_are_complementary_at_boundary() {
        let p=program("TRADE_IN ABC\nWHEN close < 100\nBUY 1\nWHEN close <= 100\nBUY 1\nWHEN close == 100\nBUY 1\nWHEN close != 100\nBUY 1\nWHEN close >= 100\nBUY 1\nWHEN close > 100\nBUY 1\n");
        for (close, expected) in [
            (100.0, [false, true, true, false, true, false]),
            (100.00000005, [false, true, true, false, true, false]),
            (100.0000002, [false, false, false, true, true, true]),
        ] {
            let frames = evaluate_series(&p, "ABC".into(), &[candle(1, close)]).unwrap();
            let actual: Vec<_> = (1..=6)
                .map(|i| frames[0].entry[&format!("rule_{i}")])
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn every_crossover_rule_reads_the_same_previous_values() {
        let p=program("TRADE_IN ABC\nWHEN CROSS_ABOVE(close, sma(2))\nBUY 1\nWHEN CROSS_ABOVE(close, sma(2))\nBUY 1\n");
        let frames = evaluate_series(
            &p,
            "ABC".into(),
            &[candle(1, 10.), candle(2, 10.), candle(3, 12.)],
        )
        .unwrap();
        assert_eq!(frames[2].entry["rule_1"], frames[2].entry["rule_2"]);
    }

    #[test]
    fn no_indicator_rules_are_warm_from_first_valid_candle() {
        let p = program("TRADE_IN ABC\nWHEN close > 1\nBUY 1\n");
        assert!(evaluate_series(&p, "ABC".into(), &[candle(1, 2.)]).unwrap()[0].warm);
    }

    proptest::proptest! {
        #[test]
        fn generated_sequences_match_batch_and_stream(closes in proptest::collection::vec(-1000.0f64..1000.0, 1..300)) {
            let p=program("TRADE_IN ABC\nWHEN ema(7) >= sma(3) AND close > 10\nBUY 1\nWHEN CROSS_BELOW(close, ema(7)) OR rsi(5) < 40\nSELL ALL\n");
            let data:Vec<_>=closes.iter().enumerate().map(|(i,&x)|candle(i as i64+1,x)).collect();
            let batch=evaluate_series(&p,"ABC".into(),&data).unwrap();
            let mut stream=Engine::new(&p,"ABC".into()).unwrap();
            let actual:Vec<_>=data.iter().map(|c|stream.on_candle(c).unwrap()).collect();
            proptest::prop_assert_eq!(batch,actual);
        }
    }
}
