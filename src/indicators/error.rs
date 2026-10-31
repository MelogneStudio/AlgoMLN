use std::fmt;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndicatorError {
    InvalidCandle { reason: &'static str },
    InvalidSpec { reason: &'static str },
    RegisterAfterStart,
}
impl fmt::Display for IndicatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCandle { reason } => write!(f, "invalid candle: {reason}"),
            Self::InvalidSpec { reason } => write!(f, "invalid indicator spec: {reason}"),
            Self::RegisterAfterStart => f.write_str("cannot register indicators after advancing"),
        }
    }
}
impl std::error::Error for IndicatorError {}
