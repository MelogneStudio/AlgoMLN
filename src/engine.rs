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
    /// Largest indicator lookback used by the strategy, suitable as the
    /// default amount of history to feed in observe mode before a run.
    pub fn warmup_bars(&self) -> usize {
        fn expr(e: &NumericExpr, max: &mut usize) {
            match e {
                NumericExpr::Indicator(i) => {
                    let needed = match i.kind {
                        IndicatorKind::Rsi | IndicatorKind::RelVol | IndicatorKind::Atr => {
                            (i.period as usize).saturating_add(1)
                        }
                        _ => i.period as usize,
                    };
                    *max = (*max).max(needed);
                }
                NumericExpr::Price(p) => match p {
                    PriceField::PrevClose
                    | PriceField::PrevOpen
                    | PriceField::PrevHigh
                    | PriceField::PrevLow => {
                        *max = (*max).max(1);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        fn condition(c: &Condition, max: &mut usize) {
            match c {
                Condition::Compare { left, right, .. } | Condition::Cross { left, right, .. } => {
                    expr(left, max);
                    expr(right, max);
                }
                Condition::And(a, b) | Condition::Or(a, b) => {
                    condition(a, max);
                    condition(b, max);
                }
                Condition::Not(a) => condition(a, max),
                _ => {}
            }
        }
        let mut max = 0;
        for rule in &self.strategy.rules {
            condition(&rule.condition, &mut max);
        }
        max
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
#[derive(Clone, Copy, Debug)]
enum FastExpr {
    Literal(Option<f64>),
    Price(PField),
    Indicator(IndicatorHandle),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ActionKind {
    Entry,
    Exit,
}

#[derive(Clone, Debug)]
enum FastCondition {
    Compare {
        left: FastExpr,
        op: CompareOp,
        right: FastExpr,
    },
    Cross {
        cross_id: usize,
        direction: CrossDirection,
        left: FastExpr,
        right: FastExpr,
    },
    And(Box<FastCondition>, Box<FastCondition>),
    Or(Box<FastCondition>, Box<FastCondition>),
    Not(Box<FastCondition>),
    Constant(bool),
}

pub struct Engine {
    symbol: Symbol,
    provider: PrecomputedProvider,
    handles: Vec<IndicatorHandle>,
    previous_ts: Option<i64>,
    cross_pairs: Vec<(FastExpr, FastExpr)>,
    cross_prev: Vec<(Option<f64>, Option<f64>)>,
    compiled_rules: Vec<(RuleId, ActionKind, FastCondition)>,
}
impl Engine {
    pub fn warmup_bars(program: &Program) -> usize {
        program.warmup_bars()
    }
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

        fn compile_expr(
            e: &NumericExpr,
            handles: &HashMap<IndicatorSpec, IndicatorHandle>,
        ) -> Result<FastExpr, EngineError> {
            match e {
                NumericExpr::Literal(x) => Ok(FastExpr::Literal(x.spelling.parse().ok())),
                NumericExpr::Price(x) => {
                    let field = match x {
                        PriceField::Close => PField::Close,
                        PriceField::Open => PField::Open,
                        PriceField::High => PField::High,
                        PriceField::Low => PField::Low,
                        PriceField::Volume => PField::Volume,
                        PriceField::PrevClose => PField::PrevClose,
                        PriceField::PrevOpen => PField::PrevOpen,
                        PriceField::PrevHigh => PField::PrevHigh,
                        PriceField::PrevLow => PField::PrevLow,
                    };
                    Ok(FastExpr::Price(field))
                }
                NumericExpr::Indicator(i) => {
                    let spec = IndicatorSpec::from_call(i)?;
                    let handle = *handles
                        .get(&spec)
                        .ok_or(EngineError::InvalidProgram("unregistered indicator handle"))?;
                    Ok(FastExpr::Indicator(handle))
                }
            }
        }

        fn compile_condition(
            c: &Condition,
            handles: &HashMap<IndicatorSpec, IndicatorHandle>,
            cross_pairs: &mut Vec<(FastExpr, FastExpr)>,
            cross_map: &mut HashMap<(NumericExpr, NumericExpr), usize>,
        ) -> Result<FastCondition, EngineError> {
            match c {
                Condition::Compare { left, op, right } => {
                    let left = compile_expr(left, handles)?;
                    let right = compile_expr(right, handles)?;
                    Ok(FastCondition::Compare {
                        left,
                        op: *op,
                        right,
                    })
                }
                Condition::Cross {
                    direction,
                    left,
                    right,
                } => {
                    let left_fast = compile_expr(left, handles)?;
                    let right_fast = compile_expr(right, handles)?;
                    let key = (left.clone(), right.clone());
                    let cross_id = if let Some(&id) = cross_map.get(&key) {
                        id
                    } else {
                        let id = cross_pairs.len();
                        cross_pairs.push((left_fast, right_fast));
                        cross_map.insert(key, id);
                        id
                    };
                    Ok(FastCondition::Cross {
                        cross_id,
                        direction: *direction,
                        left: left_fast,
                        right: right_fast,
                    })
                }
                Condition::And(a, b) => {
                    let a = compile_condition(a, handles, cross_pairs, cross_map)?;
                    let b = compile_condition(b, handles, cross_pairs, cross_map)?;
                    Ok(FastCondition::And(Box::new(a), Box::new(b)))
                }
                Condition::Or(a, b) => {
                    let a = compile_condition(a, handles, cross_pairs, cross_map)?;
                    let b = compile_condition(b, handles, cross_pairs, cross_map)?;
                    Ok(FastCondition::Or(Box::new(a), Box::new(b)))
                }
                Condition::Not(a) => {
                    let a = compile_condition(a, handles, cross_pairs, cross_map)?;
                    Ok(FastCondition::Not(Box::new(a)))
                }
                _ => Ok(FastCondition::Constant(false)),
            }
        }

        let mut compiled_rules = Vec::new();
        let mut cross_pairs = Vec::new();
        let mut cross_map = HashMap::new();
        for r in &program.strategy.rules {
            let kind = match &r.action {
                Action::Buy(_) => ActionKind::Entry,
                Action::Sell(_) | Action::SellAll => ActionKind::Exit,
            };
            let cond = compile_condition(&r.condition, &handles, &mut cross_pairs, &mut cross_map)?;
            compiled_rules.push((r.id.clone(), kind, cond));
        }
        let cross_len = cross_pairs.len();

        Ok(Self {
            symbol,
            provider,
            handles: handles.into_values().collect(),
            previous_ts: None,
            cross_pairs,
            cross_prev: vec![(None, None); cross_len],
            compiled_rules,
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
        for (id, kind, cond) in &self.compiled_rules {
            let v = self.eval_fast(cond).unwrap_or(false);
            match kind {
                ActionKind::Entry => {
                    entry.insert(id.clone(), v);
                }
                ActionKind::Exit => {
                    exit.insert(id.clone(), v);
                }
            }
        }
        for (id, (a, b)) in self.cross_pairs.iter().enumerate() {
            let x = self.eval_fast_expr(a);
            let y = self.eval_fast_expr(b);
            self.cross_prev[id] = (x, y);
        }
        self.previous_ts = Some(c.timestamp);
        let warm = self
            .handles
            .iter()
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
        self.cross_prev.fill((None, None));
    }
    fn eval_fast_expr(&self, e: &FastExpr) -> Option<f64> {
        match e {
            FastExpr::Literal(x) => *x,
            FastExpr::Price(x) => self.provider.price(*x),
            FastExpr::Indicator(h) => self.provider.get(*h),
        }
    }
    fn eval_fast(&self, c: &FastCondition) -> Option<bool> {
        match c {
            FastCondition::Compare { left, op, right } => {
                self.eval_fast_expr(left)
                    .zip(self.eval_fast_expr(right))
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
            FastCondition::Cross {
                cross_id,
                direction,
                left,
                right,
            } => {
                let (a, b) = self.cross_prev[*cross_id];
                let (x, y) = (self.eval_fast_expr(left), self.eval_fast_expr(right));
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
            FastCondition::And(a, b) => match (self.eval_fast(a), self.eval_fast(b)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            FastCondition::Or(a, b) => match (self.eval_fast(a), self.eval_fast(b)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            FastCondition::Not(a) => Some(!self.eval_fast(a)?),
            FastCondition::Constant(b) => Some(*b),
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

    #[test]
    fn warmup_bars_accounts_for_delta_indicators_and_prev_price() {
        let p_rsi = program("TRADE_IN ABC\nWHEN rsi(14) < 30\nBUY 1\n");
        assert_eq!(p_rsi.warmup_bars(), 15);

        let p_atr = program("TRADE_IN ABC\nWHEN atr(14) > 1\nBUY 1\n");
        assert_eq!(p_atr.warmup_bars(), 15);

        let p_relvol = program("TRADE_IN ABC\nWHEN rel_vol(20) > 2\nBUY 1\n");
        assert_eq!(p_relvol.warmup_bars(), 21);

        let p_sma = program("TRADE_IN ABC\nWHEN sma(20) > 100\nBUY 1\n");
        assert_eq!(p_sma.warmup_bars(), 20);

        let p_prev = program("TRADE_IN ABC\nWHEN prev_close > 100\nBUY 1\n");
        assert_eq!(p_prev.warmup_bars(), 1);
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
