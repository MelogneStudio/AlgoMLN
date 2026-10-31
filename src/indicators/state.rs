use super::{
    atr::Atr,
    bollinger::Bollinger,
    candle::Candle,
    ema::Ema,
    rel_vol::RelVol,
    rsi::Rsi,
    sma::Sma,
    spec::{IndicatorSpec, IndicatorType},
    vwap::Vwap,
};
pub(super) trait Indicator {
    fn update(&mut self, c: &Candle) -> Option<f64>;
}
pub(super) enum IndicatorState {
    Sma(Sma),
    Ema(Ema),
    Rsi(Rsi),
    Atr(Atr),
    Bollinger(Bollinger),
    RelVol(RelVol),
    Vwap(Vwap),
}
impl IndicatorState {
    pub fn new(s: IndicatorSpec) -> Self {
        match s.kind {
            IndicatorType::Sma => Self::Sma(Sma::new(s.period as usize)),
            IndicatorType::Ema => Self::Ema(Ema::new(s.period as usize)),
            IndicatorType::Rsi => Self::Rsi(Rsi::new(s.period as usize)),
            IndicatorType::Atr => Self::Atr(Atr::new(s.period as usize)),
            IndicatorType::BbUpper | IndicatorType::BbMid | IndicatorType::BbLower => {
                Self::Bollinger(Bollinger::new(
                    s.period as usize,
                    s.kind,
                    s.bollinger_multiplier(),
                ))
            }
            IndicatorType::RelVol => Self::RelVol(RelVol::new(s.period as usize)),
            IndicatorType::Vwap => Self::Vwap(Vwap::new()),
        }
    }
    pub fn update(&mut self, c: &Candle) -> Option<f64> {
        match self {
            Self::Sma(x) => x.update(c),
            Self::Ema(x) => x.update(c),
            Self::Rsi(x) => x.update(c),
            Self::Atr(x) => x.update(c),
            Self::Bollinger(x) => x.update(c),
            Self::RelVol(x) => x.update(c),
            Self::Vwap(x) => x.update(c),
        }
    }
}
