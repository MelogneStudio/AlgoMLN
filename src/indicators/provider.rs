use super::{
    candle::Candle,
    error::IndicatorError,
    spec::{IndicatorSpec, PriceField},
    state::IndicatorState,
};
use std::collections::HashMap;
pub type IndicatorHandle = usize;
pub trait IndicatorProvider {
    fn register(&mut self, spec: IndicatorSpec) -> Result<IndicatorHandle, IndicatorError>;
    fn advance(&mut self, candle: &Candle) -> Result<(), IndicatorError>;
    fn get(&self, handle: IndicatorHandle) -> Option<f64>;
    fn price(&self, field: PriceField) -> Option<f64>;
    fn reset(&mut self);
}
pub struct IncrementalProvider {
    states: Vec<IndicatorState>,
    specs: Vec<IndicatorSpec>,
    values: Vec<Option<f64>>,
    index: HashMap<IndicatorSpec, IndicatorHandle>,
    previous: Option<Candle>,
    prior: Option<Candle>,
    started: bool,
}
impl Default for IncrementalProvider {
    fn default() -> Self {
        Self::new()
    }
}
impl IncrementalProvider {
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            specs: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            previous: None,
            prior: None,
            started: false,
        }
    }
}
impl IndicatorProvider for IncrementalProvider {
    fn register(&mut self, spec: IndicatorSpec) -> Result<IndicatorHandle, IndicatorError> {
        if self.started {
            return Err(IndicatorError::RegisterAfterStart);
        }
        if let Some(&h) = self.index.get(&spec) {
            return Ok(h);
        }
        let h = self.states.len();
        self.states.push(IndicatorState::new(spec));
        self.specs.push(spec);
        self.values.push(None);
        self.index.insert(spec, h);
        Ok(h)
    }
    fn advance(&mut self, c: &Candle) -> Result<(), IndicatorError> {
        c.is_valid(self.previous.as_ref())
            .map_err(|reason| IndicatorError::InvalidCandle { reason })?;
        self.started = true;
        for (state, value) in self.states.iter_mut().zip(self.values.iter_mut()) {
            *value = state.update(c).filter(|x| x.is_finite());
            debug_assert!(value.map_or(true, |x| x.is_finite()));
        }
        self.prior = self.previous;
        self.previous = Some(*c);
        Ok(())
    }
    fn get(&self, h: IndicatorHandle) -> Option<f64> {
        self.values.get(h).copied().flatten()
    }
    fn price(&self, f: PriceField) -> Option<f64> {
        let c = self.previous?;
        Some(match f {
            PriceField::Close => c.close,
            PriceField::Open => c.open,
            PriceField::High => c.high,
            PriceField::Low => c.low,
            PriceField::Volume => c.volume,
            PriceField::PrevClose => return self.previous_prev_field(f),
            PriceField::PrevOpen | PriceField::PrevHigh | PriceField::PrevLow => {
                return self.previous_prev_field(f)
            }
        })
    }
    fn reset(&mut self) {
        self.states = self
            .specs
            .iter()
            .copied()
            .map(IndicatorState::new)
            .collect();
        self.values.fill(None);
        self.previous = None;
        self.prior = None;
        self.started = false;
    }
}
impl IncrementalProvider {
    fn previous_prev_field(&self, f: PriceField) -> Option<f64> {
        let c = self.prior?;
        Some(match f {
            PriceField::PrevClose => c.close,
            PriceField::PrevOpen => c.open,
            PriceField::PrevHigh => c.high,
            PriceField::PrevLow => c.low,
            _ => return None,
        })
    }
}
