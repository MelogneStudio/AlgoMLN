use std::collections::HashMap;
use std::fmt;

use mln_check::{check, CheckConfig, CheckedProgram, Tf};
use mln_lang::{Arg, BinOp, CmpOp, Def, Diagnostic, Expr, Program, Stmt, UnOp};
use mln_std::{tier1_registry, OpRegistry, OpSpec, ParamTy};
use rust_decimal::Decimal;

use crate::ir::{IrClock, IrConst, IrGraph, IrOp, IrTy, NodeId, SeriesField};

/// Error returned if IR lowering is given an invalid program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrError {
    CheckFailed(Vec<Diagnostic>),
    MissingOutputs,
    Unsupported(&'static str),
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrError::CheckFailed(diags) => {
                write!(
                    f,
                    "program failed static checks ({} diagnostics)",
                    diags.len()
                )
            }
            IrError::MissingOutputs => f.write_str("program is missing entry or exit"),
            IrError::Unsupported(msg) => write!(f, "unsupported construct in IR: {msg}"),
        }
    }
}

impl std::error::Error for IrError {}

/// Convenience helper: checks `program` with the Tier 1 registry and `1m` base clock, then lowers to [`IrGraph`].
pub fn lower_default(program: &Program) -> Result<IrGraph, IrError> {
    let reg = tier1_registry();
    let checked = check(program, &reg, &CheckConfig::default());
    if !checked.is_ok() {
        return Err(IrError::CheckFailed(checked.diags));
    }
    lower(program, &checked.program, &reg)
}

/// Lowers a checked [`Program`] into a deduplicated [`IrGraph`].
pub fn lower(
    program: &Program,
    checked: &CheckedProgram,
    registry: &OpRegistry,
) -> Result<IrGraph, IrError> {
    let mut lowerer = Lowerer::new(checked.base_clock, registry);
    lowerer.lower_program(program)?;
    let entry = lowerer
        .globals
        .get("entry")
        .map(|v| v.node)
        .ok_or(IrError::MissingOutputs)?;
    let exit = lowerer
        .globals
        .get("exit")
        .map(|v| v.node)
        .ok_or(IrError::MissingOutputs)?;
    lowerer.graph.entry = entry;
    lowerer.graph.exit = exit;
    Ok(lowerer.graph)
}

#[derive(Debug, Clone)]
struct IrValue {
    node: NodeId,
    ty: IrTy,
    clock: IrClock,
    const_val: Option<IrConst>,
}

struct Lowerer<'a> {
    base_clock: Tf,
    registry: &'a OpRegistry,
    graph: IrGraph,
    defs: HashMap<String, &'a Def>,
    globals: HashMap<String, IrValue>,
    locals: Vec<HashMap<String, IrValue>>,
}

impl<'a> Lowerer<'a> {
    fn new(base_clock: Tf, registry: &'a OpRegistry) -> Self {
        Self {
            base_clock,
            registry,
            graph: IrGraph::new(base_clock),
            defs: HashMap::new(),
            globals: HashMap::new(),
            locals: Vec::new(),
        }
    }

    fn clock_from_tf(&self, tf: Tf) -> IrClock {
        if tf == self.base_clock {
            IrClock::Base
        } else {
            IrClock::Tf(tf)
        }
    }

    fn lower_program(&mut self, program: &'a Program) -> Result<(), IrError> {
        for stmt in &program.stmts {
            match stmt {
                Stmt::Def(d) => {
                    self.defs.insert(d.name.name.clone(), d);
                }
                Stmt::Assign { name, value, .. } => {
                    let val = self.lower_expr(value, self.base_clock)?;
                    self.graph.bindings.push((name.name.clone(), val.node));
                    self.globals.insert(name.name.clone(), val);
                }
                Stmt::Error(_) => return Err(IrError::Unsupported("parse error node")),
            }
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<&IrValue> {
        if let Some(scope) = self.locals.last() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        self.globals.get(name)
    }

    fn intern_const(&mut self, c: IrConst, ty: IrTy) -> IrValue {
        let node = self.graph.intern(
            IrOp::Const(c.clone()),
            Vec::new(),
            Vec::new(),
            IrClock::Constant,
            ty,
        );
        IrValue {
            node,
            ty,
            clock: IrClock::Constant,
            const_val: Some(c),
        }
    }

    fn lower_expr(&mut self, expr: &'a Expr, ambient: Tf) -> Result<IrValue, IrError> {
        match expr {
            Expr::Num { value, pct, .. } => {
                let c = IrConst::Dec {
                    value: value.normalize(),
                    pct: *pct,
                };
                Ok(self.intern_const(c, IrTy::Dec))
            }
            Expr::Timeframe(tf) => Ok(self.intern_const(IrConst::Tf((*tf).into()), IrTy::Tf)),
            Expr::Str(s, _) => Ok(self.intern_const(IrConst::Str(s.clone()), IrTy::Str)),
            Expr::Bool(b, _) => Ok(self.intern_const(IrConst::Bool(*b), IrTy::Bool)),
            Expr::Name(id) => {
                if let Some(v) = self.lookup(&id.name) {
                    return Ok(v.clone());
                }
                if let Some(field) = SeriesField::from_name(&id.name) {
                    let clock = self.clock_from_tf(ambient);
                    let node = self.graph.intern(
                        IrOp::Series(field),
                        Vec::new(),
                        Vec::new(),
                        clock,
                        IrTy::Num,
                    );
                    return Ok(IrValue {
                        node,
                        ty: IrTy::Num,
                        clock,
                        const_val: None,
                    });
                }
                Err(IrError::Unsupported("unknown identifier"))
            }
            Expr::Unary { op, e, .. } => {
                let inner = self.lower_expr(e, ambient)?;
                if let Some(folded) = inner.const_val.as_ref().and_then(|c| fold_unary(*op, c)) {
                    return Ok(self.intern_const(folded, IrTy::Dec));
                }
                let node = self.graph.intern(
                    IrOp::Unary(*op),
                    vec![inner.node],
                    Vec::new(),
                    inner.clock,
                    inner.ty,
                );
                Ok(IrValue {
                    node,
                    ty: inner.ty,
                    clock: inner.clock,
                    const_val: None,
                })
            }
            Expr::Binary { op, l, r, .. } => {
                let vl = self.lower_expr(l, ambient)?;
                let vr = self.lower_expr(r, ambient)?;
                if let (Some(cl), Some(cr)) = (&vl.const_val, &vr.const_val) {
                    if let Some(folded) = fold_binary(*op, cl, cr) {
                        return Ok(self.intern_const(folded, IrTy::Dec));
                    }
                }
                let ty = if vl.ty == IrTy::Dec && vr.ty == IrTy::Dec {
                    IrTy::Dec
                } else {
                    IrTy::Num
                };
                let clock = combine_ir_clocks(&[vl.clock, vr.clock], self.base_clock);
                let node = self.graph.intern(
                    IrOp::Binary(*op),
                    vec![vl.node, vr.node],
                    Vec::new(),
                    clock,
                    ty,
                );
                Ok(IrValue {
                    node,
                    ty,
                    clock,
                    const_val: None,
                })
            }
            Expr::Compare { first, rest, .. } => {
                let mut prev = self.lower_expr(first, ambient)?;
                let mut cmp_nodes = Vec::with_capacity(rest.len());
                let mut all_const = prev.const_val.is_some();
                let mut const_acc = true;
                for (op, rhs) in rest {
                    let cur = self.lower_expr(rhs, ambient)?;
                    if all_const {
                        if let (Some(a), Some(b)) = (&prev.const_val, &cur.const_val) {
                            if let Some(res) = fold_cmp(*op, a, b) {
                                const_acc = const_acc && res;
                            } else {
                                all_const = false;
                            }
                        } else {
                            all_const = false;
                        }
                    }
                    let cclock = combine_ir_clocks(&[prev.clock, cur.clock], self.base_clock);
                    let cmp_id = self.graph.intern(
                        IrOp::Compare(*op),
                        vec![prev.node, cur.node],
                        Vec::new(),
                        cclock,
                        IrTy::Bool,
                    );
                    cmp_nodes.push((cmp_id, cclock));
                    prev = cur;
                }
                if all_const {
                    return Ok(self.intern_const(IrConst::Bool(const_acc), IrTy::Bool));
                }
                let (mut acc_id, mut acc_clock) = cmp_nodes[0];
                for &(rhs_id, rhs_clock) in &cmp_nodes[1..] {
                    acc_clock = combine_ir_clocks(&[acc_clock, rhs_clock], self.base_clock);
                    acc_id = self.graph.intern(
                        IrOp::And,
                        vec![acc_id, rhs_id],
                        Vec::new(),
                        acc_clock,
                        IrTy::Bool,
                    );
                }
                Ok(IrValue {
                    node: acc_id,
                    ty: IrTy::Bool,
                    clock: acc_clock,
                    const_val: None,
                })
            }
            Expr::Not(inner, _) => {
                let v = self.lower_expr(inner, ambient)?;
                if let Some(IrConst::Bool(b)) = v.const_val {
                    return Ok(self.intern_const(IrConst::Bool(!b), IrTy::Bool));
                }
                let node =
                    self.graph
                        .intern(IrOp::Not, vec![v.node], Vec::new(), v.clock, IrTy::Bool);
                Ok(IrValue {
                    node,
                    ty: IrTy::Bool,
                    clock: v.clock,
                    const_val: None,
                })
            }
            Expr::And(l, r, _) | Expr::Or(l, r, _) => {
                let is_and = matches!(expr, Expr::And(_, _, _));
                let vl = self.lower_expr(l, ambient)?;
                let vr = self.lower_expr(r, ambient)?;
                if let (Some(IrConst::Bool(a)), Some(IrConst::Bool(b))) =
                    (&vl.const_val, &vr.const_val)
                {
                    let res = if is_and { *a && *b } else { *a || *b };
                    return Ok(self.intern_const(IrConst::Bool(res), IrTy::Bool));
                }
                let clock = combine_ir_clocks(&[vl.clock, vr.clock], self.base_clock);
                let op = if is_and { IrOp::And } else { IrOp::Or };
                let node =
                    self.graph
                        .intern(op, vec![vl.node, vr.node], Vec::new(), clock, IrTy::Bool);
                Ok(IrValue {
                    node,
                    ty: IrTy::Bool,
                    clock,
                    const_val: None,
                })
            }
            Expr::IfElse {
                cond,
                then,
                otherwise,
                ..
            } => {
                let vc = self.lower_expr(cond, ambient)?;
                let vt = self.lower_expr(then, ambient)?;
                let vo = self.lower_expr(otherwise, ambient)?;
                if let Some(IrConst::Bool(b)) = vc.const_val {
                    if vc.clock == IrClock::Constant
                        && vt.clock == IrClock::Constant
                        && vo.clock == IrClock::Constant
                    {
                        return Ok(if b { vt } else { vo });
                    }
                }
                let ty = match (vt.ty, vo.ty) {
                    (IrTy::Bool, IrTy::Bool) => IrTy::Bool,
                    (IrTy::Dec, IrTy::Dec) if vc.const_val.is_some() => IrTy::Dec,
                    (IrTy::Tf, IrTy::Tf) => IrTy::Tf,
                    (IrTy::Str, IrTy::Str) => IrTy::Str,
                    _ => IrTy::Num,
                };
                let clock = combine_ir_clocks(&[vc.clock, vt.clock, vo.clock], self.base_clock);
                let node = self.graph.intern(
                    IrOp::IfElse,
                    vec![vc.node, vt.node, vo.node],
                    Vec::new(),
                    clock,
                    ty,
                );
                Ok(IrValue {
                    node,
                    ty,
                    clock,
                    const_val: None,
                })
            }
            Expr::Index { base, index, .. } => {
                let vb = self.lower_expr(base, ambient)?;
                let k = match self.eval_const_only(index)? {
                    Some(IrConst::Dec { value, pct: false }) => {
                        let norm = value.normalize();
                        u64::try_from(norm.mantissa())
                            .map_err(|_| IrError::Unsupported("negative index"))?
                    }
                    _ => return Err(IrError::Unsupported("non-constant index")),
                };
                if k == 0 {
                    return Ok(vb);
                }
                let node =
                    self.graph
                        .intern(IrOp::Lag(k), vec![vb.node], Vec::new(), vb.clock, vb.ty);
                Ok(IrValue {
                    node,
                    ty: vb.ty,
                    clock: vb.clock,
                    const_val: None,
                })
            }
            Expr::At { base, tf, .. } => {
                let target_tf = Tf::from(*tf);
                let vb = self.lower_expr(base, target_tf)?;
                Ok(IrValue {
                    node: vb.node,
                    ty: vb.ty,
                    clock: self.clock_from_tf(target_tf),
                    const_val: None,
                })
            }
            Expr::Call { callee, args, .. } => self.lower_call(callee, args, ambient),
            Expr::Attr { .. } | Expr::Error(_) => Err(IrError::Unsupported("attr or error node")),
        }
    }

    fn lower_call(
        &mut self,
        callee: &'a Expr,
        args: &'a [Arg],
        ambient: Tf,
    ) -> Result<IrValue, IrError> {
        let Expr::Name(id) = callee else {
            return Err(IrError::Unsupported("non-name callee"));
        };

        if let Some(op) = self.registry.get(&id.name).copied() {
            return self.lower_builtin_call(op, args, ambient);
        }

        if let Some(def) = self.defs.get(&id.name).copied() {
            return self.lower_def_call(def, args, ambient);
        }

        Err(IrError::Unsupported("unknown callee"))
    }

    fn lower_builtin_call(
        &mut self,
        op: OpSpec,
        args: &'a [Arg],
        ambient: Tf,
    ) -> Result<IrValue, IrError> {
        let mut bound: Vec<Option<&'a Expr>> = vec![None; op.params.len()];
        for (i, arg) in args.iter().enumerate() {
            if let Some(kw) = &arg.name {
                let idx = op
                    .params
                    .iter()
                    .position(|p| p.name == kw.name)
                    .ok_or(IrError::Unsupported("unknown kwarg"))?;
                bound[idx] = Some(&arg.value);
            } else {
                bound[i] = Some(&arg.value);
            }
        }

        let mut inputs = Vec::new();
        let mut const_args = Vec::new();
        let mut input_clocks = Vec::new();
        let mut int_args: Vec<Option<u64>> = vec![None; op.params.len()];

        for (idx, param) in op.params.iter().enumerate() {
            match param.ty {
                ParamTy::Num | ParamTy::Bool => {
                    let arg_expr = bound[idx].ok_or(IrError::Unsupported("missing arg"))?;
                    let val = self.lower_expr(arg_expr, ambient)?;
                    inputs.push(val.node);
                    input_clocks.push(val.clock);
                }
                ParamTy::ConstPosInt | ParamTy::ConstNonNegInt => {
                    let int_val = if let Some(arg_expr) = bound[idx] {
                        match self.eval_const_only(arg_expr)? {
                            Some(IrConst::Dec { value, pct: false }) => {
                                let norm = value.normalize();
                                i64::try_from(norm.mantissa())
                                    .map_err(|_| IrError::Unsupported("int overflow"))?
                            }
                            _ => return Err(IrError::Unsupported("non-const int param")),
                        }
                    } else {
                        match param.default {
                            Some(mln_std::ParamDefault::Int(v)) => v,
                            _ => return Err(IrError::Unsupported("missing default")),
                        }
                    };
                    if let Ok(u) = u64::try_from(int_val) {
                        int_args[idx] = Some(u);
                    }
                    const_args.push(IrConst::Int(int_val));
                }
                ParamTy::ConstDec => {
                    let dec_val = if let Some(arg_expr) = bound[idx] {
                        match self.eval_const_only(arg_expr)? {
                            Some(IrConst::Dec { value, pct }) => IrConst::Dec {
                                value: value.normalize(),
                                pct,
                            },
                            _ => return Err(IrError::Unsupported("non-const dec param")),
                        }
                    } else {
                        let d = param
                            .default
                            .and_then(|d| d.as_decimal())
                            .ok_or(IrError::Unsupported("missing dec default"))?;
                        IrConst::Dec {
                            value: d.normalize(),
                            pct: false,
                        }
                    };
                    const_args.push(dec_val);
                }
                ParamTy::ConstStr => {
                    let str_val = if let Some(arg_expr) = bound[idx] {
                        match self.eval_const_only(arg_expr)? {
                            Some(IrConst::Str(s)) => IrConst::Str(s),
                            _ => return Err(IrError::Unsupported("non-const str param")),
                        }
                    } else {
                        match param.default {
                            Some(mln_std::ParamDefault::Str(s)) => IrConst::Str(s.to_string()),
                            _ => return Err(IrError::Unsupported("missing str default")),
                        }
                    };
                    const_args.push(str_val);
                }
            }
        }

        let mut clock = combine_ir_clocks(&input_clocks, self.base_clock);
        let op_warmup = op.warmup.compute(&int_args);
        if clock == IrClock::Constant && (op_warmup > 0 || op.state_bytes > 0) {
            clock = self.clock_from_tf(ambient);
        }
        let ty = match op.ret {
            mln_std::ReturnTy::Num => IrTy::Num,
            mln_std::ReturnTy::Bool => IrTy::Bool,
        };
        let node = self
            .graph
            .intern(IrOp::Builtin(op.name), inputs, const_args, clock, ty);
        Ok(IrValue {
            node,
            ty,
            clock,
            const_val: None,
        })
    }

    fn lower_def_call(
        &mut self,
        def: &'a Def,
        args: &'a [Arg],
        ambient: Tf,
    ) -> Result<IrValue, IrError> {
        let mut bound: Vec<Option<&'a Expr>> = vec![None; def.params.len()];
        for (i, arg) in args.iter().enumerate() {
            if let Some(kw) = &arg.name {
                let idx = def
                    .params
                    .iter()
                    .position(|p| p.name.name == kw.name)
                    .ok_or(IrError::Unsupported("unknown kwarg"))?;
                bound[idx] = Some(&arg.value);
            } else {
                bound[i] = Some(&arg.value);
            }
        }

        let mut new_scope = HashMap::new();
        for (idx, param) in def.params.iter().enumerate() {
            let val = if let Some(arg_expr) = bound[idx] {
                self.lower_expr(arg_expr, ambient)?
            } else {
                let default_expr = param
                    .default
                    .as_ref()
                    .ok_or(IrError::Unsupported("missing arg"))?;
                // Evaluate default in global scope (lexical scoping).
                let saved = std::mem::take(&mut self.locals);
                let res = self.lower_expr(default_expr, ambient);
                self.locals = saved;
                res?
            };
            new_scope.insert(param.name.name.clone(), val);
        }

        let saved_locals = std::mem::take(&mut self.locals);
        self.locals.push(new_scope);

        for stmt in &def.body {
            if let Stmt::Assign { name, value, .. } = stmt {
                let val = self.lower_expr(value, ambient)?;
                self.locals
                    .last_mut()
                    .expect("active local scope")
                    .insert(name.name.clone(), val);
            }
        }
        let ret = self.lower_expr(&def.ret, ambient);
        self.locals = saved_locals;
        ret
    }

    fn eval_const_only(&self, expr: &'a Expr) -> Result<Option<IrConst>, IrError> {
        match expr {
            Expr::Num { value, pct, .. } => Ok(Some(IrConst::Dec {
                value: value.normalize(),
                pct: *pct,
            })),
            Expr::Timeframe(tf) => Ok(Some(IrConst::Tf((*tf).into()))),
            Expr::Str(s, _) => Ok(Some(IrConst::Str(s.clone()))),
            Expr::Bool(b, _) => Ok(Some(IrConst::Bool(*b))),
            Expr::Name(id) => Ok(self.lookup(&id.name).and_then(|v| v.const_val.clone())),
            Expr::Unary { op, e, .. } => {
                Ok(self.eval_const_only(e)?.and_then(|c| fold_unary(*op, &c)))
            }
            Expr::Binary { op, l, r, .. } => {
                let Some(cl) = self.eval_const_only(l)? else {
                    return Ok(None);
                };
                let Some(cr) = self.eval_const_only(r)? else {
                    return Ok(None);
                };
                Ok(fold_binary(*op, &cl, &cr))
            }
            Expr::Not(e, _) => Ok(match self.eval_const_only(e)? {
                Some(IrConst::Bool(b)) => Some(IrConst::Bool(!b)),
                _ => None,
            }),
            Expr::And(l, r, _) => Ok(match (self.eval_const_only(l)?, self.eval_const_only(r)?) {
                (Some(IrConst::Bool(a)), Some(IrConst::Bool(b))) => Some(IrConst::Bool(a && b)),
                _ => None,
            }),
            Expr::Or(l, r, _) => Ok(match (self.eval_const_only(l)?, self.eval_const_only(r)?) {
                (Some(IrConst::Bool(a)), Some(IrConst::Bool(b))) => Some(IrConst::Bool(a || b)),
                _ => None,
            }),
            Expr::IfElse {
                cond,
                then,
                otherwise,
                ..
            } => match self.eval_const_only(cond)? {
                Some(IrConst::Bool(true)) => self.eval_const_only(then),
                Some(IrConst::Bool(false)) => self.eval_const_only(otherwise),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }
}

fn combine_ir_clocks(clocks: &[IrClock], base_clock: Tf) -> IrClock {
    let mut finest: Option<Tf> = None;
    for &c in clocks {
        if let Some(tf) = c.resolve_tf(base_clock) {
            finest = Some(match finest {
                Some(prev) if prev.minutes() <= tf.minutes() => prev,
                _ => tf,
            });
        }
    }
    match finest {
        Some(tf) if tf == base_clock => IrClock::Base,
        Some(tf) => IrClock::Tf(tf),
        None => IrClock::Constant,
    }
}

fn fold_unary(op: UnOp, c: &IrConst) -> Option<IrConst> {
    match (op, c) {
        (UnOp::Neg, IrConst::Dec { value, pct }) => {
            value
                .checked_mul(Decimal::NEGATIVE_ONE)
                .map(|v| IrConst::Dec {
                    value: v.normalize(),
                    pct: *pct,
                })
        }
        (UnOp::Pos, IrConst::Dec { value, pct }) => Some(IrConst::Dec {
            value: value.normalize(),
            pct: *pct,
        }),
        _ => None,
    }
}

fn fold_binary(op: BinOp, lhs: &IrConst, rhs: &IrConst) -> Option<IrConst> {
    let (
        IrConst::Dec {
            value: a,
            pct: pct_a,
        },
        IrConst::Dec {
            value: b,
            pct: pct_b,
        },
    ) = (lhs, rhs)
    else {
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
            let norm = b.normalize();
            if norm.scale() != 0 || norm.is_sign_negative() {
                return None;
            }
            let exp = u32::try_from(norm.mantissa()).ok()?;
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
    Some(IrConst::Dec {
        value: res.normalize(),
        pct: false,
    })
}

fn fold_cmp(op: CmpOp, lhs: &IrConst, rhs: &IrConst) -> Option<bool> {
    match (lhs, rhs) {
        (
            IrConst::Dec {
                value: a,
                pct: pct_a,
            },
            IrConst::Dec {
                value: b,
                pct: pct_b,
            },
        ) if pct_a == pct_b => Some(match op {
            CmpOp::Eq => a == b,
            CmpOp::NotEq => a != b,
            CmpOp::Lt => a < b,
            CmpOp::LtEq => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::GtEq => a >= b,
        }),
        (IrConst::Bool(a), IrConst::Bool(b)) => match op {
            CmpOp::Eq => Some(a == b),
            CmpOp::NotEq => Some(a != b),
            _ => None,
        },
        (IrConst::Tf(a), IrConst::Tf(b)) => Some(match op {
            CmpOp::Eq => a.minutes() == b.minutes(),
            CmpOp::NotEq => a.minutes() != b.minutes(),
            CmpOp::Lt => a.minutes() < b.minutes(),
            CmpOp::LtEq => a.minutes() <= b.minutes(),
            CmpOp::Gt => a.minutes() > b.minutes(),
            CmpOp::GtEq => a.minutes() >= b.minutes(),
        }),
        (IrConst::Str(a), IrConst::Str(b)) => match op {
            CmpOp::Eq => Some(a == b),
            CmpOp::NotEq => Some(a != b),
            _ => None,
        },
        _ => None,
    }
}
