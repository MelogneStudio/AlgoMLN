use mln_lang::{BinOp, CmpOp, UnOp};
use rust_decimal::Decimal;

use crate::types::ConstVal;

/// Folds a unary arithmetic operation on a constant value.
pub fn fold_unary(op: UnOp, val: &ConstVal) -> Option<ConstVal> {
    match (op, val) {
        (UnOp::Neg, ConstVal::Dec(d, pct)) => d
            .checked_mul(Decimal::NEGATIVE_ONE)
            .map(|v| ConstVal::Dec(v, *pct)),
        (UnOp::Pos, ConstVal::Dec(d, pct)) => Some(ConstVal::Dec(*d, *pct)),
        _ => None,
    }
}

/// Folds a binary arithmetic operation on two constant numeric values.
pub fn fold_binary(op: BinOp, lhs: &ConstVal, rhs: &ConstVal) -> Option<ConstVal> {
    let (ConstVal::Dec(a, pct_a), ConstVal::Dec(b, pct_b)) = (lhs, rhs) else {
        return None;
    };
    if *pct_a || *pct_b {
        return None;
    }
    let res = match op {
        BinOp::Add => a.checked_add(*b)?,
        BinOp::Sub => a.checked_sub(*b)?,
        BinOp::Mul => a.checked_mul(*b)?,
        BinOp::Div => {
            if b.is_zero() {
                return None;
            }
            a.checked_div(*b)?
        }
        BinOp::Pow => {
            let exp_norm = b.normalize();
            if exp_norm.scale() != 0 || exp_norm.is_sign_negative() {
                return None;
            }
            let exp = u32::try_from(exp_norm.mantissa()).ok()?;
            if exp > 64 {
                return None;
            }
            let mut acc = Decimal::ONE;
            for _ in 0..exp {
                acc = acc.checked_mul(*a)?;
            }
            acc
        }
    };
    Some(ConstVal::Dec(res, false))
}

/// Folds a comparison between two constant values.
pub fn fold_compare_pair(op: CmpOp, lhs: &ConstVal, rhs: &ConstVal) -> Option<bool> {
    match (lhs, rhs) {
        (ConstVal::Dec(a, pct_a), ConstVal::Dec(b, pct_b)) if pct_a == pct_b => Some(match op {
            CmpOp::Eq => a == b,
            CmpOp::NotEq => a != b,
            CmpOp::Lt => a < b,
            CmpOp::LtEq => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::GtEq => a >= b,
        }),
        (ConstVal::Bool(a), ConstVal::Bool(b)) => match op {
            CmpOp::Eq => Some(a == b),
            CmpOp::NotEq => Some(a != b),
            _ => None,
        },
        (ConstVal::Tf(a), ConstVal::Tf(b)) => {
            let am = a.minutes();
            let bm = b.minutes();
            Some(match op {
                CmpOp::Eq => am == bm,
                CmpOp::NotEq => am != bm,
                CmpOp::Lt => am < bm,
                CmpOp::LtEq => am <= bm,
                CmpOp::Gt => am > bm,
                CmpOp::GtEq => am >= bm,
            })
        }
        (ConstVal::Str(a), ConstVal::Str(b)) => match op {
            CmpOp::Eq => Some(a == b),
            CmpOp::NotEq => Some(a != b),
            _ => None,
        },
        _ => None,
    }
}

/// Folds `not` on a constant boolean.
pub fn fold_not(val: &ConstVal) -> Option<ConstVal> {
    match val {
        ConstVal::Bool(b) => Some(ConstVal::Bool(!b)),
        _ => None,
    }
}

/// Folds `and` on two constant booleans.
pub fn fold_and(lhs: &ConstVal, rhs: &ConstVal) -> Option<ConstVal> {
    match (lhs, rhs) {
        (ConstVal::Bool(a), ConstVal::Bool(b)) => Some(ConstVal::Bool(*a && *b)),
        _ => None,
    }
}

/// Folds `or` on two constant booleans.
pub fn fold_or(lhs: &ConstVal, rhs: &ConstVal) -> Option<ConstVal> {
    match (lhs, rhs) {
        (ConstVal::Bool(a), ConstVal::Bool(b)) => Some(ConstVal::Bool(*a || *b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_arithmetic_and_unary() {
        let ten = ConstVal::Dec(Decimal::from(10), false);
        let two = ConstVal::Dec(Decimal::from(2), false);
        let prod = fold_binary(BinOp::Mul, &ten, &two).unwrap();
        assert_eq!(prod.as_exact_int(), Some(20));

        let neg = fold_unary(UnOp::Neg, &ConstVal::Dec(Decimal::from(1), false)).unwrap();
        assert_eq!(neg.as_exact_int(), Some(-1));

        assert!(fold_binary(BinOp::Div, &ten, &ConstVal::Dec(Decimal::ZERO, false)).is_none());
    }

    #[test]
    fn fold_timeframe_comparisons_by_minutes() {
        use crate::types::Tf;
        use mln_lang::TfUnit;

        let m60 = ConstVal::Tf(Tf::new(60, TfUnit::Minute));
        let h1 = ConstVal::Tf(Tf::new(1, TfUnit::Hour));
        let d1 = ConstVal::Tf(Tf::new(1, TfUnit::Day));
        assert_eq!(fold_compare_pair(CmpOp::Eq, &m60, &h1), Some(true));
        assert_eq!(fold_compare_pair(CmpOp::NotEq, &m60, &h1), Some(false));
        assert_eq!(fold_compare_pair(CmpOp::Lt, &h1, &m60), Some(false));
        assert_eq!(fold_compare_pair(CmpOp::LtEq, &h1, &m60), Some(true));
        assert_eq!(fold_compare_pair(CmpOp::Gt, &d1, &m60), Some(true));
    }
}
