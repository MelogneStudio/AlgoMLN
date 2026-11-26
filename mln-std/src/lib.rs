#![forbid(unsafe_code)]

//! Standard operator registry (`OpSpec`) and operator definitions for LangMelogne.

pub mod eval;
pub mod spec;
pub mod tier1;

pub use eval::TriBool;
pub use spec::{
    CostClass, NanRule, OpRegistry, OpSpec, ParamDefault, ParamSpec, ParamTy, ReturnTy, WarmupRule,
};
pub use tier1::{tier1_registry, TIER1_OPS};
