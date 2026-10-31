use serde::{Deserialize, Serialize};

/// UTF-8 byte offsets plus one-based line and Unicode-scalar column positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub column: u32,
}

/// A decimal's lexical spelling and exact finite base-10 value. The value is
/// kept as a normalized coefficient and scale, never converted through f64.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecimalValue {
    pub negative: bool,
    pub coefficient: String,
    pub scale: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NumericLiteral {
    pub spelling: String,
    pub value: DecimalValue,
    pub span: Span,
}

impl NumericLiteral {
    pub fn parse(spelling: String, span: Span) -> Option<Self> {
        let (negative, raw) = if let Some(s) = spelling.strip_prefix('-') {
            (true, s)
        } else {
            (false, spelling.strip_prefix('+').unwrap_or(&spelling))
        };
        let mut it = raw.split('.');
        let whole = it.next()?;
        let fraction = it.next().unwrap_or("");
        if it.next().is_some()
            || whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let digits = format!("{whole}{fraction}");
        let mut coefficient = digits.trim_start_matches('0').to_owned();
        let mut scale = fraction.len() as u32;
        while scale > 0 && coefficient.ends_with('0') {
            coefficient.pop();
            scale -= 1;
        }
        let coefficient = if coefficient.is_empty() {
            "0".to_owned()
        } else {
            coefficient.to_owned()
        };
        let negative = negative && coefficient.chars().any(|c| c != '0');
        let scale = if coefficient == "0" { 0 } else { scale };
        Some(Self {
            spelling,
            value: DecimalValue {
                negative,
                coefficient,
                scale,
            },
            span,
        })
    }
    pub fn cmp_integer(&self, integer: &str) -> Option<std::cmp::Ordering> {
        let rhs = Self::parse(integer.to_owned(), Span::default())?;
        if self.value.coefficient == "0" && rhs.value.coefficient == "0" {
            return Some(std::cmp::Ordering::Equal);
        }
        let scale = self.value.scale.max(rhs.value.scale);
        let mut a = self.value.coefficient.clone();
        let mut b = rhs.value.coefficient.clone();
        a.extend(std::iter::repeat('0').take((scale - self.value.scale) as usize));
        b.extend(std::iter::repeat('0').take((scale - rhs.value.scale) as usize));
        let ord = a.len().cmp(&b.len()).then_with(|| a.cmp(&b));
        Some(if self.value.negative != rhs.value.negative {
            if self.value.negative {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        } else if self.value.coefficient == "0" && rhs.value.coefficient == "0" {
            std::cmp::Ordering::Equal
        } else if self.value.negative {
            ord.reverse()
        } else {
            ord
        })
    }
    pub fn compare_value(&self, other: &Self) -> std::cmp::Ordering {
        if self.value.coefficient == "0" && other.value.coefficient == "0" {
            return std::cmp::Ordering::Equal;
        }
        if self.value.negative != other.value.negative {
            return if self.value.negative {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let scale = self.value.scale.max(other.value.scale);
        let mut a = self.value.coefficient.clone();
        let mut b = other.value.coefficient.clone();
        a.extend(std::iter::repeat('0').take((scale - self.value.scale) as usize));
        b.extend(std::iter::repeat('0').take((scale - other.value.scale) as usize));
        let ord = a.len().cmp(&b.len()).then_with(|| a.cmp(&b));
        if self.value.coefficient == "0" && other.value.coefficient == "0" {
            std::cmp::Ordering::Equal
        } else if self.value.negative {
            ord.reverse()
        } else {
            ord
        }
    }
    pub fn is_integer(&self) -> bool {
        self.value.scale == 0
    }
}

impl Span {
    pub const fn new(start: usize, end: usize, line: u32, column: u32) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }
}
