#![forbid(unsafe_code)]

//! Stream DAG intermediate representation (`IrGraph`) and CSE lowering for LangMelogne.

pub mod dump;
pub mod ir;
pub mod lower;

pub use dump::dump_ir;
pub use ir::{
    IrClock, IrConst, IrExecStep, IrGraph, IrNode, IrOp, IrReset, IrStateBlock, IrStateVar,
    IrTrigger, IrTy, NodeId, PositionField, SeriesField, StateId, SymbolField,
};
pub use lower::{lower, lower_default, IrError};
