use crate::{
    ast::{IndicatorCall, IndicatorKind},
    source::NumericLiteral,
};
use std::hash::{Hash, Hasher};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PriceField {
    Close,
    Open,
    High,
    Low,
    Volume,
    PrevClose,
    PrevOpen,
    PrevHigh,
    PrevLow,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndicatorType {
    Sma,
    Ema,
    Rsi,
    Atr,
    BbUpper,
    BbMid,
    BbLower,
    RelVol,
    Vwap,
}
#[derive(Clone, Copy, Debug)]
pub struct IndicatorSpec {
    pub kind: IndicatorType,
    pub period: u32,
    multiplier: Option<u64>,
}
impl IndicatorSpec {
    pub fn new(
        kind: IndicatorType,
        period: u32,
        multiplier: Option<f64>,
    ) -> Result<Self, crate::indicators::IndicatorError> {
        if kind != IndicatorType::Vwap && period == 0 {
            return Err(crate::indicators::IndicatorError::InvalidSpec {
                reason: "period must be positive",
            });
        }
        if multiplier.is_some_and(|m| !m.is_finite() || m <= 0.0) {
            return Err(crate::indicators::IndicatorError::InvalidSpec {
                reason: "multiplier must be finite and positive",
            });
        }
        if multiplier.is_some()
            && !matches!(
                kind,
                IndicatorType::BbUpper | IndicatorType::BbMid | IndicatorType::BbLower
            )
        {
            return Err(crate::indicators::IndicatorError::InvalidSpec {
                reason: "only Bollinger indicators accept a multiplier",
            });
        }
        Ok(Self {
            kind,
            period: if kind == IndicatorType::Vwap {
                0
            } else {
                period
            },
            multiplier: if matches!(
                kind,
                IndicatorType::BbUpper | IndicatorType::BbMid | IndicatorType::BbLower
            ) {
                Some(multiplier.unwrap_or(2.0).to_bits())
            } else {
                None
            },
        })
    }
    pub fn bollinger_multiplier(self) -> f64 {
        self.multiplier.map(f64::from_bits).unwrap_or(2.0)
    }
    pub fn from_call(call: &IndicatorCall) -> Result<Self, crate::indicators::IndicatorError> {
        let kind = match call.kind {
            IndicatorKind::Sma => IndicatorType::Sma,
            IndicatorKind::Ema => IndicatorType::Ema,
            IndicatorKind::Rsi => IndicatorType::Rsi,
            IndicatorKind::Atr => IndicatorType::Atr,
            IndicatorKind::BbUpper => IndicatorType::BbUpper,
            IndicatorKind::BbMid => IndicatorType::BbMid,
            IndicatorKind::BbLower => IndicatorType::BbLower,
            IndicatorKind::RelVol => IndicatorType::RelVol,
            IndicatorKind::Vwap => IndicatorType::Vwap,
        };
        Self::new(
            kind,
            call.period,
            call.multiplier
                .as_ref()
                .map(decimal_to_f64)
                .transpose()?
                .or_else(|| {
                    matches!(
                        kind,
                        IndicatorType::BbUpper | IndicatorType::BbMid | IndicatorType::BbLower
                    )
                    .then_some(2.0)
                }),
        )
    }
}
fn decimal_to_f64(d: &NumericLiteral) -> Result<f64, crate::indicators::IndicatorError> {
    let value =
        d.spelling
            .parse::<f64>()
            .map_err(|_| crate::indicators::IndicatorError::InvalidSpec {
                reason: "decimal multiplier cannot be represented as f64",
            })?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(crate::indicators::IndicatorError::InvalidSpec {
            reason: "decimal multiplier cannot be represented as finite f64",
        })
    }
}
impl PartialEq for IndicatorSpec {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.period == other.period
            && self.multiplier == other.multiplier
    }
}
impl Eq for IndicatorSpec {}
impl Hash for IndicatorSpec {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.period.hash(state);
        self.multiplier.hash(state)
    }
}
