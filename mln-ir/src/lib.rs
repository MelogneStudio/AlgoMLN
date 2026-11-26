#![forbid(unsafe_code)]

//! Stream DAG intermediate representation (`IrGraph`) and CSE lowering for LangMelogne.

pub mod dump;
pub mod ir;
pub mod lower;

pub use dump::dump_ir;
pub use ir::{IrClock, IrConst, IrGraph, IrNode, IrOp, IrTy, NodeId, SeriesField};
pub use lower::{lower, lower_default, IrError};
