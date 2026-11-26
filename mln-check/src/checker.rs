use std::collections::{BTreeMap, HashMap};

use mln_lang::{Arg, CmpOp, Def, Diagnostic, Expr, Program, Severity, Span, Stmt};
use mln_std::{tier1_registry, OpRegistry, OpSpec, ParamTy, ReturnTy};

use crate::const_eval::{fold_and, fold_binary, fold_compare_pair, fold_not, fold_or, fold_unary};
use crate::types::{
    CheckConfig, CheckedExpr, CheckedProgram, CheckedStmt, Clock, ConstVal, DefInstance,
    OutputWarmup, Tf, Ty, WarmupReport,
};

const BUILTIN_SERIES: &[&str] = &["open", "high", "low", "close", "volume"];
const MAX_CALL_DEPTH: usize = 64;

/// Result of running [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutput {
    pub program: CheckedProgram,
    pub diags: Vec<Diagnostic>,
}

impl CheckOutput {
    /// Returns `true` when checking produced zero errors (warnings such as `W0201` are allowed).
    pub fn is_ok(&self) -> bool {
        !self.diags.iter().any(|d| d.severity == Severity::Error)
    }
}

/// Checks a parsed [`Program`] against `registry` and `config`.
pub fn check(program: &Program, registry: &OpRegistry, config: &CheckConfig) -> CheckOutput {
    let mut checker = Checker::new(program, registry, config);
    checker.run();
    checker.finish()
}

/// Convenience wrapper using the default Tier 1 registry and a `1m` base clock.
pub fn check_default(program: &Program) -> CheckOutput {
    let reg = tier1_registry();
    check(program, &reg, &CheckConfig::default())
}

/// Convenience wrapper using the default Tier 1 registry and a custom `base_clock`.
pub fn check_with_base(program: &Program, base_clock: Tf) -> CheckOutput {
    let reg = tier1_registry();
    check(program, &reg, &CheckConfig { base_clock })
}

#[derive(Debug, Clone)]
struct Binding {
    ty: Ty,
    clock: Clock,
    const_val: Option<ConstVal>,
    warmups: BTreeMap<Tf, u64>,
    span: Span,
    used: bool,
}

#[derive(Debug, Clone)]
struct DefEntry<'a> {
    def: &'a Def,
    used: bool,
    recursive: bool,
    poisoned_name: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

struct Checker<'a> {
    program: &'a Program,
    registry: &'a OpRegistry,
    config: CheckConfig,
    diags: Vec<Diagnostic>,
    defs: HashMap<String, DefEntry<'a>>,
    declared_top: HashMap<String, Span>,
    globals: HashMap<String, Binding>,
    scopes: Vec<HashMap<String, Binding>>,
    local_declared: Vec<HashMap<String, Span>>,
    checked_stmts: Vec<CheckedStmt>,
    def_instances: Vec<DefInstance>,
    call_depth: usize,
    has_parse_error: bool,
}

impl<'a> Checker<'a> {
    fn new(program: &'a Program, registry: &'a OpRegistry, config: &CheckConfig) -> Self {
        Self {
            program,
            registry,
            config: *config,
            diags: Vec::new(),
            defs: HashMap::new(),
            declared_top: HashMap::new(),
            globals: HashMap::new(),
            scopes: Vec::new(),
            local_declared: Vec::new(),
            checked_stmts: Vec::new(),
            def_instances: Vec::new(),
            call_depth: 0,
            has_parse_error: false,
        }
    }

    fn is_builtin(&self, name: &str) -> bool {
        BUILTIN_SERIES.contains(&name) || self.registry.get(name).is_some()
    }

    fn emit_error(&mut self, code: &'static str, msg: impl Into<String>, span: Span) {
        self.diags.push(Diagnostic::error(code, msg, span));
    }

    fn emit_warning(&mut self, code: &'static str, msg: impl Into<String>, span: Span) {
        self.diags.push(Diagnostic {
            code,
            severity: Severity::Warning,
            message: msg.into(),
            span,
            help: None,
        });
    }

    fn collect_top_declarations(&mut self) {
        for stmt in &self.program.stmts {
            match stmt {
                Stmt::Assign { name, .. } => {
                    self.declared_top
                        .entry(name.name.clone())
                        .or_insert(name.span);
                }
                Stmt::Def(d) => {
                    self.declared_top
                        .entry(d.name.name.clone())
                        .or_insert(d.name.span);
                    if self.is_builtin(&d.name.name) {
                        self.emit_error(
                            "E0203",
                            format!("cannot shadow built-in `{}`", d.name.name),
                            d.name.span,
                        );
                        continue;
                    }
                    if self.defs.contains_key(&d.name.name) {
                        self.emit_error(
                            "E0201",
                            format!("redefinition of `{}`", d.name.name),
                            d.name.span,
                        );
                        continue;
                    }
                    self.defs.insert(
                        d.name.name.clone(),
                        DefEntry {
                            def: d,
                            used: false,
                            recursive: false,
                            poisoned_name: false,
                        },
                    );
                }
                Stmt::Error(_) => {
                    self.has_parse_error = true;
                }
            }
        }
    }

    fn detect_def_recursion(&mut self) {
        let mut edges: HashMap<String, Vec<(String, Span)>> = HashMap::new();
        for (name, entry) in &self.defs {
            let mut callees = Vec::new();
            for p in &entry.def.params {
                if let Some(d) = &p.default {
                    collect_calls_in_expr(d, &mut callees);
                }
            }
            for s in &entry.def.body {
                if let Stmt::Assign { value, .. } = s {
                    collect_calls_in_expr(value, &mut callees);
                }
            }
            collect_calls_in_expr(&entry.def.ret, &mut callees);
            edges.insert(name.clone(), callees);
        }

        let names: Vec<String> = self.defs.keys().cloned().collect();
        let mut state: HashMap<String, VisitState> = HashMap::new();
        let mut stack: Vec<String> = Vec::new();

        for name in names {
            if !state.contains_key(&name) {
                self.dfs_recursion(&name, &edges, &mut state, &mut stack);
            }
        }
    }

    fn dfs_recursion(
        &mut self,
        cur: &str,
        edges: &HashMap<String, Vec<(String, Span)>>,
        state: &mut HashMap<String, VisitState>,
        stack: &mut Vec<String>,
    ) {
        state.insert(cur.to_string(), VisitState::Visiting);
        stack.push(cur.to_string());

        if let Some(callees) = edges.get(cur) {
            for (target, call_span) in callees {
                if !self.defs.contains_key(target) {
                    continue;
                }
                match state.get(target).copied() {
                    Some(VisitState::Visiting) => {
                        let start_idx = stack.iter().position(|n| n == target).unwrap_or(0);
                        for node in &stack[start_idx..] {
                            if let Some(entry) = self.defs.get_mut(node) {
                                entry.recursive = true;
                            }
                        }
                        self.emit_error(
                            "E0210",
                            format!("recursive call to `def {target}` is not allowed"),
                            *call_span,
                        );
                    }
                    None => {
                        self.dfs_recursion(target, edges, state, stack);
                    }
                    Some(VisitState::Done) => {}
                }
            }
        }

        stack.pop();
        state.insert(cur.to_string(), VisitState::Done);
    }

    fn run(&mut self) {
        self.collect_top_declarations();
        self.detect_def_recursion();

        let ambient = self.config.base_clock;
        let mut checked_stmts = Vec::with_capacity(self.program.stmts.len());

        for stmt in &self.program.stmts {
            match stmt {
                Stmt::Assign { name, value, span } => {
                    let expr = self.check_expr(value, ambient, Some(&name.name));
                    let used = matches!(name.name.as_str(), "entry" | "exit");
                    if self.is_builtin(&name.name) {
                        self.emit_error(
                            "E0203",
                            format!("cannot shadow built-in `{}`", name.name),
                            name.span,
                        );
                    } else if self.globals.contains_key(&name.name)
                        || self.defs.contains_key(&name.name)
                    {
                        self.emit_error(
                            "E0201",
                            format!("redefinition of `{}`", name.name),
                            name.span,
                        );
                    } else {
                        self.globals.insert(
                            name.name.clone(),
                            Binding {
                                ty: expr.ty,
                                clock: expr.clock,
                                const_val: expr.const_val.clone(),
                                warmups: expr.warmups.clone(),
                                span: name.span,
                                used,
                            },
                        );
                    }
                    checked_stmts.push(CheckedStmt::Assign {
                        name: name.name.clone(),
                        expr,
                        span: *span,
                    });
                }
                Stmt::Def(d) => {
                    if self.globals.contains_key(&d.name.name) {
                        self.emit_error(
                            "E0201",
                            format!("redefinition of `{}`", d.name.name),
                            d.name.span,
                        );
                        if let Some(entry) = self.defs.get_mut(&d.name.name) {
                            entry.poisoned_name = true;
                        }
                    }
                    for p in &d.params {
                        if self.is_builtin(&p.name.name) {
                            self.emit_error(
                                "E0203",
                                format!("parameter `{}` cannot shadow a built-in", p.name.name),
                                p.name.span,
                            );
                        }
                    }
                    checked_stmts.push(CheckedStmt::DefDecl {
                        name: d.name.name.clone(),
                        span: d.span,
                    });
                }
                Stmt::Error(span) => {
                    self.has_parse_error = true;
                    checked_stmts.push(CheckedStmt::Error(*span));
                }
            }
        }

        // Validate uncalled non-recursive `def`s once with symbolic `Num` parameters,
        // and emit `W0201` if they are clean and unused.
        let uncalled_defs: Vec<(String, &'a Def)> = self
            .defs
            .iter()
            .filter(|(_, e)| !e.used && !e.recursive && !e.poisoned_name)
            .map(|(k, e)| (k.clone(), e.def))
            .collect();
        for (def_name, def) in uncalled_defs {
            let mut param_bindings = Vec::with_capacity(def.params.len());
            for p in &def.params {
                let b = if let Some(d_expr) = &p.default {
                    let ce = self.check_expr(d_expr, ambient, None);
                    Binding {
                        ty: ce.ty,
                        clock: ce.clock,
                        const_val: ce.const_val,
                        warmups: ce.warmups,
                        span: p.name.span,
                        used: true,
                    }
                } else {
                    Binding {
                        ty: Ty::Num,
                        clock: Clock::Tf(ambient),
                        const_val: None,
                        warmups: BTreeMap::new(),
                        span: p.name.span,
                        used: true,
                    }
                };
                param_bindings.push((p.name.name.clone(), b));
            }
            let _ = self.check_def_body(def, &param_bindings, ambient);
            if !def_has_error(def) {
                self.emit_warning(
                    "W0201",
                    format!("unused function `def {def_name}`"),
                    def.name.span,
                );
            }
        }

        // Emit `W0201` for unused top-level variables (excluding `entry`, `exit`, and `Ty::Error`).
        let mut unused_vars: Vec<(Span, String)> = self
            .globals
            .iter()
            .filter(|(name, b)| !b.used && *name != "entry" && *name != "exit" && b.ty != Ty::Error)
            .map(|(name, b)| (b.span, name.clone()))
            .collect();
        unused_vars.sort_by_key(|(s, _)| s.start);
        for (span, name) in unused_vars {
            self.emit_warning("W0201", format!("unused variable `{name}`"), span);
        }

        self.check_outputs();
        self.checked_stmts = checked_stmts;
    }

    fn check_outputs(&mut self) {
        let entry = self.globals.get("entry").cloned();
        let exit = self.globals.get("exit").cloned();

        if !self.has_parse_error {
            if entry.is_none() {
                self.emit_error(
                    "E0220",
                    "program is missing required `entry` output",
                    Span::empty(0),
                );
            }
            if exit.is_none() {
                self.emit_error(
                    "E0220",
                    "program is missing required `exit` output",
                    Span::empty(0),
                );
            }
        }

        for (label, binding) in [("entry", entry), ("exit", exit)] {
            if let Some(b) = binding {
                if b.ty != Ty::Bool && b.ty != Ty::Error {
                    self.emit_error(
                        "E0300",
                        format!("`{label}` must have type Bool, found {}", b.ty),
                        b.span,
                    );
                }
            }
        }
    }

    fn finish(mut self) -> CheckOutput {
        self.diags.sort_by(|a, b| {
            a.span
                .start
                .cmp(&b.span.start)
                .then_with(|| a.code.cmp(b.code))
        });
        let warmup = self.build_warmup_report();
        CheckOutput {
            program: CheckedProgram {
                base_clock: self.config.base_clock,
                stmts: self.checked_stmts,
                def_instances: self.def_instances,
                warmup,
            },
            diags: self.diags,
        }
    }

    fn build_warmup_report(&self) -> WarmupReport {
        let entry = self
            .globals
            .get("entry")
            .map(|b| self.output_warmup("entry", b));
        let exit = self
            .globals
            .get("exit")
            .map(|b| self.output_warmup("exit", b));

        let max_base_bars_estimate = entry
            .as_ref()
            .map_or(0, |o| o.base_bars_estimate)
            .max(exit.as_ref().map_or(0, |o| o.base_bars_estimate));

        WarmupReport {
            base_clock: self.config.base_clock,
            entry,
            exit,
            max_base_bars_estimate,
        }
    }

    fn output_warmup(&self, name: &str, b: &Binding) -> OutputWarmup {
        let base_min = self.config.base_clock.minutes().max(1);
        let expr_clock = match b.clock {
            Clock::Constant => self.config.base_clock,
            Clock::Tf(tf) => tf,
        };
        let own_clock_bars = b.warmups.get(&expr_clock).copied().unwrap_or(0);

        let mut primary_clock = expr_clock;
        let mut primary_clock_bars = own_clock_bars;
        let mut max_est = 0u64;

        for (&tf, &bars) in &b.warmups {
            let ratio = tf.minutes().div_ceil(base_min).max(1);
            let est = bars.saturating_mul(ratio);
            if est > max_est {
                max_est = est;
                primary_clock = tf;
                primary_clock_bars = bars;
            }
        }

        OutputWarmup {
            name: name.to_string(),
            clock: expr_clock,
            own_clock_bars,
            primary_clock,
            primary_clock_bars,
            by_clock: b.warmups.clone(),
            base_bars_estimate: max_est,
        }
    }

    fn err_expr(&self, span: Span) -> CheckedExpr {
        CheckedExpr {
            ty: Ty::Error,
            clock: Clock::Constant,
            const_val: None,
            warmups: BTreeMap::new(),
            span,
        }
    }

    fn lookup_var(&mut self, name: &str) -> Option<Binding> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(b) = scope.get_mut(name) {
                b.used = true;
                return Some(b.clone());
            }
        }
        if let Some(b) = self.globals.get_mut(name) {
            b.used = true;
            return Some(b.clone());
        }
        None
    }

    fn is_declared_later_local(&self, name: &str) -> bool {
        self.local_declared
            .iter()
            .rev()
            .any(|m| m.contains_key(name))
    }

    fn check_expr(
        &mut self,
        expr: &'a Expr,
        ambient: Tf,
        self_assign_target: Option<&str>,
    ) -> CheckedExpr {
        match expr {
            Expr::Num { value, pct, span } => CheckedExpr {
                ty: Ty::Dec,
                clock: Clock::Constant,
                const_val: Some(ConstVal::Dec(*value, *pct)),
                warmups: BTreeMap::new(),
                span: *span,
            },
            Expr::Timeframe(tf) => CheckedExpr {
                ty: Ty::Tf,
                clock: Clock::Constant,
                const_val: Some(ConstVal::Tf((*tf).into())),
                warmups: BTreeMap::new(),
                span: tf.span,
            },
            Expr::Str(s, span) => CheckedExpr {
                ty: Ty::Str,
                clock: Clock::Constant,
                const_val: Some(ConstVal::Str(s.clone())),
                warmups: BTreeMap::new(),
                span: *span,
            },
            Expr::Bool(b, span) => CheckedExpr {
                ty: Ty::Bool,
                clock: Clock::Constant,
                const_val: Some(ConstVal::Bool(*b)),
                warmups: BTreeMap::new(),
                span: *span,
            },
            Expr::Error(span) => self.err_expr(*span),
            Expr::Name(id) => {
                if let Some(b) = self.lookup_var(&id.name) {
                    return CheckedExpr {
                        ty: b.ty,
                        clock: b.clock,
                        const_val: b.const_val,
                        warmups: b.warmups,
                        span: id.span,
                    };
                }
                if BUILTIN_SERIES.contains(&id.name.as_str()) {
                    return CheckedExpr {
                        ty: Ty::Num,
                        clock: Clock::Tf(ambient),
                        const_val: None,
                        warmups: BTreeMap::new(),
                        span: id.span,
                    };
                }
                if self.defs.contains_key(&id.name) || self.registry.get(&id.name).is_some() {
                    self.emit_error(
                        "E0300",
                        format!("function `{}` must be called with `(...)`", id.name),
                        id.span,
                    );
                    return self.err_expr(id.span);
                }
                if self_assign_target == Some(id.name.as_str())
                    || self.is_declared_later_local(&id.name)
                    || (self.scopes.is_empty() && self.declared_top.contains_key(&id.name))
                {
                    self.emit_error(
                        "E0202",
                        format!("use of `{}` before definition", id.name),
                        id.span,
                    );
                } else {
                    self.emit_error(
                        "E0200",
                        format!("unknown identifier `{}`", id.name),
                        id.span,
                    );
                }
                self.err_expr(id.span)
            }
            Expr::Unary { op, e, span } => {
                let inner = self.check_expr(e, ambient, self_assign_target);
                if inner.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                if !matches!(inner.ty, Ty::Num | Ty::Dec) {
                    self.emit_error(
                        "E0300",
                        format!(
                            "unary operator requires numeric operand, found {}",
                            inner.ty
                        ),
                        *span,
                    );
                    return self.err_expr(*span);
                }
                let const_val = inner.const_val.as_ref().and_then(|v| fold_unary(*op, v));
                CheckedExpr {
                    ty: inner.ty,
                    clock: inner.clock,
                    const_val,
                    warmups: inner.warmups,
                    span: *span,
                }
            }
            Expr::Binary { op, l, r, span } => {
                let cl = self.check_expr(l, ambient, self_assign_target);
                let cr = self.check_expr(r, ambient, self_assign_target);
                if cl.ty == Ty::Error || cr.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                if !matches!(cl.ty, Ty::Num | Ty::Dec) || !matches!(cr.ty, Ty::Num | Ty::Dec) {
                    self.emit_error(
                        "E0300",
                        format!(
                            "binary arithmetic requires numeric operands, found {} and {}",
                            cl.ty, cr.ty
                        ),
                        *span,
                    );
                    return self.err_expr(*span);
                }
                let ty = if cl.ty == Ty::Dec && cr.ty == Ty::Dec {
                    Ty::Dec
                } else {
                    Ty::Num
                };
                let const_val = if ty == Ty::Dec {
                    cl.const_val
                        .as_ref()
                        .zip(cr.const_val.as_ref())
                        .and_then(|(a, b)| fold_binary(*op, a, b))
                } else {
                    None
                };
                let clock = combine_clocks(&[cl.clock, cr.clock]);
                let warmups = merge_warmups(&[&cl.warmups, &cr.warmups]);
                CheckedExpr {
                    ty,
                    clock,
                    const_val,
                    warmups,
                    span: *span,
                }
            }
            Expr::Compare { first, rest, span } => {
                let c0 = self.check_expr(first, ambient, self_assign_target);
                let mut checked_rest = Vec::with_capacity(rest.len());
                let mut any_err = c0.ty == Ty::Error;
                for (op, rhs) in rest {
                    let cr = self.check_expr(rhs, ambient, self_assign_target);
                    if cr.ty == Ty::Error {
                        any_err = true;
                    }
                    checked_rest.push((*op, cr));
                }
                if any_err {
                    return self.err_expr(*span);
                }

                let mut prev = &c0;
                let mut all_const = c0.const_val.is_some();
                let mut const_acc = true;
                for (op, cr) in &checked_rest {
                    if !types_comparable(*op, prev.ty, cr.ty) {
                        self.emit_error(
                            "E0300",
                            format!("cannot compare {} and {}", prev.ty, cr.ty),
                            *span,
                        );
                        return self.err_expr(*span);
                    }
                    if all_const {
                        if let (Some(lv), Some(rv)) = (&prev.const_val, &cr.const_val) {
                            if let Some(b) = fold_compare_pair(*op, lv, rv) {
                                const_acc = const_acc && b;
                            } else {
                                all_const = false;
                            }
                        } else {
                            all_const = false;
                        }
                    }
                    prev = cr;
                }

                let mut clocks = Vec::with_capacity(1 + checked_rest.len());
                clocks.push(c0.clock);
                let mut warmup_refs: Vec<&BTreeMap<Tf, u64>> = Vec::with_capacity(1 + rest.len());
                warmup_refs.push(&c0.warmups);
                for (_, cr) in &checked_rest {
                    clocks.push(cr.clock);
                    warmup_refs.push(&cr.warmups);
                }
                CheckedExpr {
                    ty: Ty::Bool,
                    clock: combine_clocks(&clocks),
                    const_val: all_const.then_some(ConstVal::Bool(const_acc)),
                    warmups: merge_warmups(&warmup_refs),
                    span: *span,
                }
            }
            Expr::Not(inner, span) => {
                let ci = self.check_expr(inner, ambient, self_assign_target);
                if ci.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                if ci.ty != Ty::Bool {
                    self.emit_error(
                        "E0300",
                        format!("`not` requires Bool operand, found {}", ci.ty),
                        *span,
                    );
                    return self.err_expr(*span);
                }
                let const_val = ci.const_val.as_ref().and_then(fold_not);
                CheckedExpr {
                    ty: Ty::Bool,
                    clock: ci.clock,
                    const_val,
                    warmups: ci.warmups,
                    span: *span,
                }
            }
            Expr::And(l, r, span) | Expr::Or(l, r, span) => {
                let is_and = matches!(expr, Expr::And(_, _, _));
                let cl = self.check_expr(l, ambient, self_assign_target);
                let cr = self.check_expr(r, ambient, self_assign_target);
                if cl.ty == Ty::Error || cr.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                if cl.ty != Ty::Bool || cr.ty != Ty::Bool {
                    let op_name = if is_and { "and" } else { "or" };
                    self.emit_error(
                        "E0300",
                        format!(
                            "`{op_name}` requires Bool operands, found {} and {}",
                            cl.ty, cr.ty
                        ),
                        *span,
                    );
                    return self.err_expr(*span);
                }
                let const_val =
                    cl.const_val
                        .as_ref()
                        .zip(cr.const_val.as_ref())
                        .and_then(|(a, b)| {
                            if is_and {
                                fold_and(a, b)
                            } else {
                                fold_or(a, b)
                            }
                        });
                CheckedExpr {
                    ty: Ty::Bool,
                    clock: combine_clocks(&[cl.clock, cr.clock]),
                    const_val,
                    warmups: merge_warmups(&[&cl.warmups, &cr.warmups]),
                    span: *span,
                }
            }
            Expr::IfElse {
                cond,
                then,
                otherwise,
                span,
            } => {
                let cc = self.check_expr(cond, ambient, self_assign_target);
                let ct = self.check_expr(then, ambient, self_assign_target);
                let co = self.check_expr(otherwise, ambient, self_assign_target);
                if cc.ty == Ty::Error || ct.ty == Ty::Error || co.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                if cc.ty != Ty::Bool {
                    self.emit_error(
                        "E0300",
                        format!("`if` condition must be Bool, found {}", cc.ty),
                        cc.span,
                    );
                    return self.err_expr(*span);
                }
                let cond_is_const = cc.const_val.is_some();
                let ty = match (ct.ty, co.ty) {
                    (Ty::Num | Ty::Dec, Ty::Num | Ty::Dec) => {
                        if cond_is_const && ct.ty == Ty::Dec && co.ty == Ty::Dec {
                            Ty::Dec
                        } else {
                            Ty::Num
                        }
                    }
                    (Ty::Bool, Ty::Bool) => Ty::Bool,
                    (Ty::Tf, Ty::Tf) if cond_is_const => Ty::Tf,
                    (Ty::Str, Ty::Str) if cond_is_const => Ty::Str,
                    _ => {
                        self.emit_error(
                            "E0300",
                            format!(
                                "incompatible `if`/`else` branch types {} and {}",
                                ct.ty, co.ty
                            ),
                            *span,
                        );
                        return self.err_expr(*span);
                    }
                };
                let const_val = match &cc.const_val {
                    Some(ConstVal::Bool(true)) => ct.const_val.clone(),
                    Some(ConstVal::Bool(false)) => co.const_val.clone(),
                    _ => None,
                };
                CheckedExpr {
                    ty,
                    clock: combine_clocks(&[cc.clock, ct.clock, co.clock]),
                    const_val,
                    warmups: merge_warmups(&[&cc.warmups, &ct.warmups, &co.warmups]),
                    span: *span,
                }
            }
            Expr::Index { base, index, span } => {
                let cb = self.check_expr(base, ambient, self_assign_target);
                let ci = self.check_expr(index, ambient, self_assign_target);
                if cb.ty == Ty::Error || ci.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                if !matches!(cb.ty, Ty::Num | Ty::Bool | Ty::Dec) {
                    self.emit_error(
                        "E0300",
                        format!("cannot index expression of type {}", cb.ty),
                        *span,
                    );
                    return self.err_expr(*span);
                }
                let Some(cv) = &ci.const_val else {
                    self.emit_error("E0311", "index must be a constant integer", ci.span);
                    return self.err_expr(*span);
                };
                let Some(k) = cv.as_exact_int() else {
                    self.emit_error("E0311", "index must be a constant integer", ci.span);
                    return self.err_expr(*span);
                };
                if k < 0 {
                    self.emit_error(
                        "E0310",
                        format!("negative index `{k}` would read the future"),
                        ci.span,
                    );
                    return self.err_expr(*span);
                }
                let mut warmups = cb.warmups;
                if let Clock::Tf(tf) = cb.clock {
                    let entry = warmups.entry(tf).or_insert(0);
                    *entry = entry.saturating_add(k as u64);
                }
                CheckedExpr {
                    ty: cb.ty,
                    clock: cb.clock,
                    const_val: if cb.clock == Clock::Constant {
                        cb.const_val
                    } else {
                        None
                    },
                    warmups,
                    span: *span,
                }
            }
            Expr::Attr { base, name, span } => {
                let cb = self.check_expr(base, ambient, self_assign_target);
                if cb.ty != Ty::Error {
                    self.emit_error(
                        "E0300",
                        format!("attribute `.{}` is not supported on {}", name.name, cb.ty),
                        *span,
                    );
                }
                self.err_expr(*span)
            }
            Expr::At { base, tf, span } => {
                let target_tf = Tf::from(*tf);
                if target_tf < ambient {
                    self.emit_error(
                        "E0401",
                        format!(
                            "finer clock `{target_tf}` cannot be used inside coarser clock `{ambient}`"
                        ),
                        tf.span,
                    );
                    return self.err_expr(*span);
                }
                let cb = self.check_expr(base, target_tf, self_assign_target);
                if cb.ty == Ty::Error {
                    return self.err_expr(*span);
                }
                match cb.clock {
                    Clock::Constant => {
                        self.emit_error(
                            "E0402",
                            "cannot apply `@` to a constant expression",
                            *span,
                        );
                        self.err_expr(*span)
                    }
                    Clock::Tf(c) => {
                        if c < target_tf {
                            self.emit_error(
                                "E0401",
                                format!(
                                    "finer clock `{c}` cannot be aligned to coarser clock `{target_tf}`"
                                ),
                                *span,
                            );
                            return self.err_expr(*span);
                        }
                        CheckedExpr {
                            ty: cb.ty,
                            clock: Clock::Tf(target_tf),
                            const_val: None,
                            warmups: cb.warmups,
                            span: *span,
                        }
                    }
                }
            }
            Expr::Call { callee, args, span } => {
                self.check_call(callee, args, *span, ambient, self_assign_target)
            }
        }
    }

    fn check_call(
        &mut self,
        callee: &'a Expr,
        args: &'a [Arg],
        span: Span,
        ambient: Tf,
        self_assign_target: Option<&str>,
    ) -> CheckedExpr {
        let Expr::Name(id) = callee else {
            let c = self.check_expr(callee, ambient, self_assign_target);
            if c.ty != Ty::Error {
                self.emit_error("E0301", "expression is not callable", callee.span());
            }
            return self.err_expr(span);
        };

        if let Some(op) = self.registry.get(&id.name).copied() {
            return self.check_builtin_call(op, args, span, ambient, self_assign_target);
        }

        if let Some(def_entry) = self.defs.get(&id.name).cloned() {
            if let Some(entry) = self.defs.get_mut(&id.name) {
                entry.used = true;
            }
            if def_entry.recursive || def_entry.poisoned_name {
                return self.err_expr(span);
            }
            return self.check_def_call(def_entry.def, args, span, ambient, self_assign_target);
        }

        // Check arguments even if unknown, or check if `id` is a non-callable variable.
        if self.lookup_var(&id.name).is_some() || BUILTIN_SERIES.contains(&id.name.as_str()) {
            self.emit_error("E0301", format!("`{}` is not callable", id.name), id.span);
        } else if self_assign_target == Some(id.name.as_str())
            || self.is_declared_later_local(&id.name)
            || (self.scopes.is_empty() && self.declared_top.contains_key(&id.name))
        {
            self.emit_error(
                "E0202",
                format!("use of `{}` before definition", id.name),
                id.span,
            );
        } else {
            self.emit_error("E0200", format!("unknown function `{}`", id.name), id.span);
        }
        self.err_expr(span)
    }

    fn check_builtin_call(
        &mut self,
        op: OpSpec,
        args: &'a [Arg],
        span: Span,
        ambient: Tf,
        self_assign_target: Option<&str>,
    ) -> CheckedExpr {
        let param_count = op.params.len();
        let mut bound: Vec<Option<&'a Expr>> = vec![None; param_count];
        let mut sig_error = false;

        for (i, arg) in args.iter().enumerate() {
            if let Some(kw) = &arg.name {
                if let Some(idx) = op.params.iter().position(|p| p.name == kw.name) {
                    if bound[idx].is_some() {
                        self.emit_error(
                            "E0301",
                            format!("argument `{}` provided more than once", kw.name),
                            arg.span,
                        );
                        sig_error = true;
                    } else {
                        bound[idx] = Some(&arg.value);
                    }
                } else {
                    self.emit_error(
                        "E0302",
                        format!("unknown keyword argument `{}` for `{}`", kw.name, op.name),
                        kw.span,
                    );
                    sig_error = true;
                }
            } else if i < param_count {
                if bound[i].is_some() {
                    self.emit_error("E0301", "duplicate argument", arg.span);
                    sig_error = true;
                } else {
                    bound[i] = Some(&arg.value);
                }
            } else {
                self.emit_error(
                    "E0301",
                    format!(
                        "`{}` takes at most {param_count} argument(s), got {}",
                        op.name,
                        args.len()
                    ),
                    span,
                );
                sig_error = true;
                break;
            }
        }

        for (idx, param) in op.params.iter().enumerate() {
            if bound[idx].is_none() && param.default.is_none() {
                self.emit_error(
                    "E0301",
                    format!("`{}` missing required argument `{}`", op.name, param.name),
                    span,
                );
                sig_error = true;
            }
        }

        if sig_error {
            // Still check provided arg expressions for inner undefined names.
            for arg in args {
                let _ = self.check_expr(&arg.value, ambient, self_assign_target);
            }
            return self.err_expr(span);
        }

        let mut int_args: Vec<Option<u64>> = vec![None; param_count];
        let mut series_clocks: Vec<Clock> = Vec::new();
        let mut series_warmups: Vec<BTreeMap<Tf, u64>> = Vec::new();
        let mut any_err = false;

        for (idx, param) in op.params.iter().enumerate() {
            if let Some(arg_expr) = bound[idx] {
                let ce = self.check_expr(arg_expr, ambient, self_assign_target);
                if ce.ty == Ty::Error {
                    any_err = true;
                    continue;
                }
                match param.ty {
                    ParamTy::Num => {
                        if !matches!(ce.ty, Ty::Num | Ty::Dec) {
                            self.emit_error(
                                "E0303",
                                format!(
                                    "argument `{}` to `{}` expects Num, found {}",
                                    param.name, op.name, ce.ty
                                ),
                                ce.span,
                            );
                            any_err = true;
                        } else {
                            series_clocks.push(ce.clock);
                            series_warmups.push(ce.warmups);
                        }
                    }
                    ParamTy::Bool => {
                        if ce.ty != Ty::Bool {
                            self.emit_error(
                                "E0303",
                                format!(
                                    "argument `{}` to `{}` expects Bool, found {}",
                                    param.name, op.name, ce.ty
                                ),
                                ce.span,
                            );
                            any_err = true;
                        } else {
                            series_clocks.push(ce.clock);
                            series_warmups.push(ce.warmups);
                        }
                    }
                    ParamTy::ConstPosInt => {
                        let k = ce.const_val.as_ref().and_then(ConstVal::as_exact_int);
                        if let Some(k) = k.filter(|&v| v >= 1) {
                            int_args[idx] = Some(k as u64);
                        } else {
                            self.emit_error(
                                "E0303",
                                format!(
                                    "argument `{}` to `{}` must be a constant positive integer",
                                    param.name, op.name
                                ),
                                ce.span,
                            );
                            any_err = true;
                        }
                    }
                    ParamTy::ConstNonNegInt => {
                        let k = ce.const_val.as_ref().and_then(ConstVal::as_exact_int);
                        if let Some(k) = k.filter(|&v| v >= 0) {
                            int_args[idx] = Some(k as u64);
                        } else {
                            self.emit_error(
                                "E0303",
                                format!(
                                    "argument `{}` to `{}` must be a constant non-negative integer",
                                    param.name, op.name
                                ),
                                ce.span,
                            );
                            any_err = true;
                        }
                    }
                    ParamTy::ConstDec => {
                        if ce.ty != Ty::Dec || ce.const_val.is_none() {
                            self.emit_error(
                                "E0303",
                                format!(
                                    "argument `{}` to `{}` must be a constant decimal",
                                    param.name, op.name
                                ),
                                ce.span,
                            );
                            any_err = true;
                        }
                    }
                    ParamTy::ConstStr => {
                        if ce.ty != Ty::Str || ce.const_val.is_none() {
                            self.emit_error(
                                "E0303",
                                format!(
                                    "argument `{}` to `{}` must be a constant string",
                                    param.name, op.name
                                ),
                                ce.span,
                            );
                            any_err = true;
                        }
                    }
                }
            } else if let Some(def_val) = param.default {
                if let Some(d) = def_val.as_decimal() {
                    if let Ok(k) = u64::try_from(d.normalize().mantissa()) {
                        if d.normalize().scale() == 0 {
                            int_args[idx] = Some(k);
                        }
                    }
                }
            }
        }

        if any_err {
            return self.err_expr(span);
        }

        let call_clock = combine_clocks(&series_clocks);
        let warmup_refs: Vec<&BTreeMap<Tf, u64>> = series_warmups.iter().collect();
        let mut warmups = merge_warmups(&warmup_refs);
        let op_warmup = op.warmup.compute(&int_args);
        if op_warmup > 0 {
            let target_clock = match call_clock {
                Clock::Tf(tf) => tf,
                Clock::Constant => ambient,
            };
            let entry = warmups.entry(target_clock).or_insert(0);
            *entry = entry.saturating_add(op_warmup);
        }

        let ret_ty = match op.ret {
            ReturnTy::Num => Ty::Num,
            ReturnTy::Bool => Ty::Bool,
        };
        CheckedExpr {
            ty: ret_ty,
            clock: call_clock,
            const_val: None,
            warmups,
            span,
        }
    }

    fn check_def_call(
        &mut self,
        def: &'a Def,
        args: &'a [Arg],
        span: Span,
        ambient: Tf,
        self_assign_target: Option<&str>,
    ) -> CheckedExpr {
        if self.call_depth >= MAX_CALL_DEPTH {
            self.emit_error("E0210", "def call nesting exceeds limit", span);
            return self.err_expr(span);
        }

        let param_count = def.params.len();
        let mut bound: Vec<Option<&'a Expr>> = vec![None; param_count];
        let mut sig_error = false;

        for (i, arg) in args.iter().enumerate() {
            if let Some(kw) = &arg.name {
                if let Some(idx) = def.params.iter().position(|p| p.name.name == kw.name) {
                    if bound[idx].is_some() {
                        self.emit_error(
                            "E0301",
                            format!("argument `{}` provided more than once", kw.name),
                            arg.span,
                        );
                        sig_error = true;
                    } else {
                        bound[idx] = Some(&arg.value);
                    }
                } else {
                    self.emit_error(
                        "E0302",
                        format!(
                            "unknown keyword argument `{}` for `{}`",
                            kw.name, def.name.name
                        ),
                        kw.span,
                    );
                    sig_error = true;
                }
            } else if i < param_count {
                if bound[i].is_some() {
                    self.emit_error("E0301", "duplicate argument", arg.span);
                    sig_error = true;
                } else {
                    bound[i] = Some(&arg.value);
                }
            } else {
                self.emit_error(
                    "E0301",
                    format!(
                        "`{}` takes at most {param_count} argument(s), got {}",
                        def.name.name,
                        args.len()
                    ),
                    span,
                );
                sig_error = true;
                break;
            }
        }

        for (idx, param) in def.params.iter().enumerate() {
            if bound[idx].is_none() && param.default.is_none() {
                self.emit_error(
                    "E0301",
                    format!(
                        "`{}` missing required argument `{}`",
                        def.name.name, param.name.name
                    ),
                    span,
                );
                sig_error = true;
            }
        }

        if sig_error {
            return self.err_expr(span);
        }

        let mut param_bindings = Vec::with_capacity(param_count);
        let mut arg_types = Vec::with_capacity(param_count);
        for (idx, param) in def.params.iter().enumerate() {
            let ce = if let Some(arg_expr) = bound[idx] {
                self.check_expr(arg_expr, ambient, self_assign_target)
            } else {
                let default_expr = param.default.as_ref().expect("checked above");
                self.check_expr(default_expr, ambient, None)
            };
            arg_types.push(ce.ty);
            param_bindings.push((
                param.name.name.clone(),
                Binding {
                    ty: ce.ty,
                    clock: ce.clock,
                    const_val: ce.const_val,
                    warmups: ce.warmups,
                    span: param.name.span,
                    used: true,
                },
            ));
        }

        self.call_depth += 1;
        let ret = self.check_def_body(def, &param_bindings, ambient);
        self.call_depth -= 1;

        let instance_id = self.def_instances.len();
        self.def_instances.push(DefInstance {
            id: instance_id,
            def_name: def.name.name.clone(),
            call_span: span,
            clock: ret.clock,
            arg_types,
            ret_ty: ret.ty,
            warmups: ret.warmups.clone(),
        });

        CheckedExpr {
            ty: ret.ty,
            clock: ret.clock,
            const_val: ret.const_val,
            warmups: ret.warmups,
            span,
        }
    }

    fn check_def_body(
        &mut self,
        def: &'a Def,
        param_bindings: &[(String, Binding)],
        ambient: Tf,
    ) -> CheckedExpr {
        let mut scope = HashMap::new();
        let mut declared = HashMap::new();

        for (pname, binding) in param_bindings {
            declared.insert(pname.clone(), binding.span);
            scope.insert(pname.clone(), binding.clone());
        }
        for stmt in &def.body {
            if let Stmt::Assign { name, .. } = stmt {
                declared.entry(name.name.clone()).or_insert(name.span);
            }
        }

        self.scopes.push(scope);
        self.local_declared.push(declared);

        for stmt in &def.body {
            match stmt {
                Stmt::Assign { name, value, .. } => {
                    let expr = self.check_expr(value, ambient, Some(&name.name));
                    let is_builtin = self.is_builtin(&name.name);
                    let already_defined = self
                        .scopes
                        .last()
                        .expect("active scope")
                        .contains_key(&name.name);
                    if is_builtin {
                        self.emit_error(
                            "E0203",
                            format!("cannot shadow built-in `{}`", name.name),
                            name.span,
                        );
                    } else if already_defined {
                        self.emit_error(
                            "E0201",
                            format!("redefinition of `{}`", name.name),
                            name.span,
                        );
                    } else {
                        self.scopes.last_mut().expect("active scope").insert(
                            name.name.clone(),
                            Binding {
                                ty: expr.ty,
                                clock: expr.clock,
                                const_val: expr.const_val,
                                warmups: expr.warmups,
                                span: name.span,
                                used: false,
                            },
                        );
                    }
                }
                Stmt::Def(_) | Stmt::Error(_) => {}
            }
        }

        let ret = self.check_expr(&def.ret, ambient, None);

        let finished_scope = self.scopes.pop().expect("active scope");
        self.local_declared.pop();

        // Warn on unused locals inside the def body (not params, and only once on first check if desired;
        // skipping duplicate W0201 across call sites keeps diagnostics clean).
        let _ = finished_scope;

        ret
    }
}

fn collect_calls_in_expr(expr: &Expr, out: &mut Vec<(String, Span)>) {
    match expr {
        Expr::Unary { e, .. } | Expr::Not(e, _) | Expr::At { base: e, .. } => {
            collect_calls_in_expr(e, out)
        }
        Expr::Attr { base, .. } => collect_calls_in_expr(base, out),
        Expr::Binary { l, r, .. } | Expr::And(l, r, _) | Expr::Or(l, r, _) => {
            collect_calls_in_expr(l, out);
            collect_calls_in_expr(r, out);
        }
        Expr::Compare { first, rest, .. } => {
            collect_calls_in_expr(first, out);
            for (_, rhs) in rest {
                collect_calls_in_expr(rhs, out);
            }
        }
        Expr::IfElse {
            cond,
            then,
            otherwise,
            ..
        } => {
            collect_calls_in_expr(cond, out);
            collect_calls_in_expr(then, out);
            collect_calls_in_expr(otherwise, out);
        }
        Expr::Call { callee, args, span } => {
            if let Expr::Name(id) = callee.as_ref() {
                out.push((id.name.clone(), *span));
            } else {
                collect_calls_in_expr(callee, out);
            }
            for arg in args {
                collect_calls_in_expr(&arg.value, out);
            }
        }
        Expr::Index { base, index, .. } => {
            collect_calls_in_expr(base, out);
            collect_calls_in_expr(index, out);
        }
        Expr::Num { .. }
        | Expr::Timeframe(_)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Name(_)
        | Expr::Error(_) => {}
    }
}

fn def_has_error(def: &Def) -> bool {
    matches!(def.ret, Expr::Error(_)) || def.body.iter().any(|s| matches!(s, Stmt::Error(_)))
}

fn types_comparable(op: CmpOp, a: Ty, b: Ty) -> bool {
    match (a, b) {
        (Ty::Num | Ty::Dec, Ty::Num | Ty::Dec) => true,
        (Ty::Bool, Ty::Bool) | (Ty::Str, Ty::Str) => matches!(op, CmpOp::Eq | CmpOp::NotEq),
        (Ty::Tf, Ty::Tf) => true,
        _ => false,
    }
}

fn combine_clocks(clocks: &[Clock]) -> Clock {
    let mut finest: Option<Tf> = None;
    for &c in clocks {
        if let Clock::Tf(tf) = c {
            finest = Some(match finest {
                Some(prev) => prev.min(tf),
                None => tf,
            });
        }
    }
    match finest {
        Some(tf) => Clock::Tf(tf),
        None => Clock::Constant,
    }
}

fn merge_warmups(maps: &[&BTreeMap<Tf, u64>]) -> BTreeMap<Tf, u64> {
    let mut out = BTreeMap::new();
    for map in maps {
        for (&tf, &bars) in *map {
            let entry = out.entry(tf).or_insert(0);
            *entry = (*entry).max(bars);
        }
    }
    out
}
