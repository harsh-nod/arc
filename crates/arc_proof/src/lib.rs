//! AIR proof system: constraint solving and proof obligation discharge.
//!
//! Supports arithmetic bounds checking, type refinement proofs,
//! and authority/capability proof obligations.

use std::collections::HashMap;

/// A proof obligation that must be discharged.
#[derive(Debug, Clone)]
pub struct ProofObligation {
    pub kind: ObligationKind,
    pub description: String,
    pub context: ProofContext,
}

/// The kind of proof obligation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObligationKind {
    /// Prove that an index is within bounds: 0 <= index < bound.
    BoundsCheck { index: Expr, bound: Expr },
    /// Prove that a value satisfies a predicate.
    Predicate { expr: Expr },
    /// Prove that a type refinement holds.
    Refinement { from_type: String, to_type: String },
    /// Prove that authority has been obtained for a capability.
    Authority { capability: String },
    /// A general-purpose constraint.
    Custom { constraint: String },
}

/// Simple symbolic expressions for constraint reasoning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// An integer literal.
    Lit(i64),
    /// A symbolic variable.
    Var(String),
    /// Addition.
    Add(Box<Expr>, Box<Expr>),
    /// Subtraction.
    Sub(Box<Expr>, Box<Expr>),
    /// Multiplication.
    Mul(Box<Expr>, Box<Expr>),
    /// Comparison: lhs < rhs.
    Lt(Box<Expr>, Box<Expr>),
    /// Comparison: lhs <= rhs.
    Le(Box<Expr>, Box<Expr>),
    /// Comparison: lhs == rhs.
    Eq(Box<Expr>, Box<Expr>),
    /// Logical AND.
    And(Box<Expr>, Box<Expr>),
    /// Logical NOT.
    Not(Box<Expr>),
}

impl Expr {
    pub fn lit(v: i64) -> Self {
        Expr::Lit(v)
    }
    pub fn var(name: &str) -> Self {
        Expr::Var(name.to_string())
    }

    /// Try to evaluate the expression with concrete variable bindings.
    pub fn eval(&self, env: &HashMap<String, i64>) -> Option<i64> {
        match self {
            Expr::Lit(v) => Some(*v),
            Expr::Var(name) => env.get(name).copied(),
            Expr::Add(l, r) => Some(l.eval(env)? + r.eval(env)?),
            Expr::Sub(l, r) => Some(l.eval(env)? - r.eval(env)?),
            Expr::Mul(l, r) => Some(l.eval(env)? * r.eval(env)?),
            Expr::Lt(l, r) => Some((l.eval(env)? < r.eval(env)?) as i64),
            Expr::Le(l, r) => Some((l.eval(env)? <= r.eval(env)?) as i64),
            Expr::Eq(l, r) => Some((l.eval(env)? == r.eval(env)?) as i64),
            Expr::And(l, r) => Some(((l.eval(env)? != 0) && (r.eval(env)? != 0)) as i64),
            Expr::Not(e) => Some((e.eval(env)? == 0) as i64),
        }
    }

    /// Simplify the expression algebraically.
    pub fn simplify(&self) -> Expr {
        match self {
            Expr::Add(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(a), Expr::Lit(b)) => Expr::Lit(a + b),
                    (_, Expr::Lit(0)) => l,
                    (Expr::Lit(0), _) => r,
                    _ => Expr::Add(Box::new(l), Box::new(r)),
                }
            }
            Expr::Sub(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(a), Expr::Lit(b)) => Expr::Lit(a - b),
                    (_, Expr::Lit(0)) => l,
                    _ if l == r => Expr::Lit(0),
                    _ => Expr::Sub(Box::new(l), Box::new(r)),
                }
            }
            Expr::Mul(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(a), Expr::Lit(b)) => Expr::Lit(a * b),
                    (_, Expr::Lit(1)) => l,
                    (Expr::Lit(1), _) => r,
                    (_, Expr::Lit(0)) | (Expr::Lit(0), _) => Expr::Lit(0),
                    _ => Expr::Mul(Box::new(l), Box::new(r)),
                }
            }
            Expr::Lt(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(a), Expr::Lit(b)) => Expr::Lit((*a < *b) as i64),
                    _ => Expr::Lt(Box::new(l), Box::new(r)),
                }
            }
            Expr::Le(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(a), Expr::Lit(b)) => Expr::Lit((*a <= *b) as i64),
                    _ => Expr::Le(Box::new(l), Box::new(r)),
                }
            }
            Expr::Eq(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(a), Expr::Lit(b)) => Expr::Lit((*a == *b) as i64),
                    _ if l == r => Expr::Lit(1), // x == x is always true
                    _ => Expr::Eq(Box::new(l), Box::new(r)),
                }
            }
            Expr::And(l, r) => {
                let l = l.simplify();
                let r = r.simplify();
                match (&l, &r) {
                    (Expr::Lit(0), _) | (_, Expr::Lit(0)) => Expr::Lit(0),
                    (Expr::Lit(_), _) => r, // nonzero AND r = r
                    (_, Expr::Lit(_)) => l,
                    _ => Expr::And(Box::new(l), Box::new(r)),
                }
            }
            Expr::Not(e) => {
                let e = e.simplify();
                match &e {
                    Expr::Lit(0) => Expr::Lit(1),
                    Expr::Lit(_) => Expr::Lit(0),
                    Expr::Not(inner) => inner.simplify(),
                    _ => Expr::Not(Box::new(e)),
                }
            }
            other => other.clone(),
        }
    }
}

/// Known facts available at a proof point.
#[derive(Debug, Clone, Default)]
pub struct ProofContext {
    /// Known facts (expressions known to be true/nonzero).
    pub facts: Vec<Expr>,
    /// Known variable bounds: variable name → (lower, upper) inclusive.
    pub bounds: HashMap<String, (i64, i64)>,
    /// Known authority tokens held.
    pub authorities: Vec<String>,
}

impl ProofContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_fact(&mut self, expr: Expr) {
        self.facts.push(expr);
    }

    pub fn add_bound(&mut self, var: &str, lower: i64, upper: i64) {
        self.bounds.insert(var.to_string(), (lower, upper));
    }

    pub fn add_authority(&mut self, capability: &str) {
        self.authorities.push(capability.to_string());
    }
}

/// Result of attempting to discharge a proof obligation.
#[derive(Debug, Clone)]
pub enum ProofResult {
    /// The obligation is provably satisfied.
    Proved { reason: String },
    /// The obligation cannot be proved with available information.
    Unproved { reason: String },
    /// The obligation is provably false.
    Disproved { reason: String },
}

impl ProofResult {
    pub fn is_proved(&self) -> bool {
        matches!(self, ProofResult::Proved { .. })
    }

    pub fn is_disproved(&self) -> bool {
        matches!(self, ProofResult::Disproved { .. })
    }
}

/// Attempt to discharge a proof obligation.
pub fn discharge(obligation: &ProofObligation) -> ProofResult {
    match &obligation.kind {
        ObligationKind::BoundsCheck { index, bound } => {
            discharge_bounds_check(index, bound, &obligation.context)
        }
        ObligationKind::Predicate { expr } => discharge_predicate(expr, &obligation.context),
        ObligationKind::Refinement { from_type, to_type } => {
            discharge_refinement(from_type, to_type, &obligation.context)
        }
        ObligationKind::Authority { capability } => {
            discharge_authority(capability, &obligation.context)
        }
        ObligationKind::Custom { constraint } => {
            // Custom constraints are not automatically solvable
            ProofResult::Unproved {
                reason: format!("custom constraint '{}' requires manual proof", constraint),
            }
        }
    }
}

fn discharge_bounds_check(index: &Expr, bound: &Expr, ctx: &ProofContext) -> ProofResult {
    let index_simplified = index.simplify();
    let bound_simplified = bound.simplify();

    // Try direct evaluation if all variables are known
    let mut env = HashMap::new();
    for (var, (lo, _hi)) in &ctx.bounds {
        env.insert(var.clone(), *lo);
    }

    // Check lower bound: 0 <= index
    match &index_simplified {
        Expr::Lit(v) if *v < 0 => {
            return ProofResult::Disproved {
                reason: format!("index {} is negative", v),
            };
        }
        Expr::Var(name) => {
            if let Some((lo, _)) = ctx.bounds.get(name) {
                if *lo < 0 {
                    return ProofResult::Unproved {
                        reason: format!(
                            "variable {} has lower bound {} which may be negative",
                            name, lo
                        ),
                    };
                }
            }
        }
        _ => {}
    }

    // Check upper bound: index < bound
    match (&index_simplified, &bound_simplified) {
        (Expr::Lit(idx), Expr::Lit(bnd)) => {
            if *idx >= 0 && *idx < *bnd {
                ProofResult::Proved {
                    reason: format!("0 <= {} < {} holds", idx, bnd),
                }
            } else {
                ProofResult::Disproved {
                    reason: format!("bounds check failed: {} not in [0, {})", idx, bnd),
                }
            }
        }
        (Expr::Var(name), Expr::Lit(bnd)) => {
            if let Some((lo, hi)) = ctx.bounds.get(name) {
                if *lo >= 0 && *hi < *bnd {
                    ProofResult::Proved {
                        reason: format!(
                            "{} in [{}, {}] which is within [0, {})",
                            name, lo, hi, bnd
                        ),
                    }
                } else {
                    ProofResult::Unproved {
                        reason: format!(
                            "{} in [{}, {}] — cannot prove within [0, {})",
                            name, lo, hi, bnd
                        ),
                    }
                }
            } else {
                ProofResult::Unproved {
                    reason: format!("no bounds known for {}", name),
                }
            }
        }
        _ => ProofResult::Unproved {
            reason: "cannot solve symbolic bounds check".to_string(),
        },
    }
}

fn discharge_predicate(expr: &Expr, ctx: &ProofContext) -> ProofResult {
    let simplified = expr.simplify();

    // Check if the simplified expression is a literal truth
    match &simplified {
        Expr::Lit(v) if *v != 0 => {
            return ProofResult::Proved {
                reason: "expression simplifies to true".to_string(),
            };
        }
        Expr::Lit(0) => {
            return ProofResult::Disproved {
                reason: "expression simplifies to false".to_string(),
            };
        }
        _ => {}
    }

    // Check if any known fact implies this predicate
    for fact in &ctx.facts {
        let fact_simplified = fact.simplify();
        if fact_simplified == simplified {
            return ProofResult::Proved {
                reason: "matches known fact".to_string(),
            };
        }
    }

    ProofResult::Unproved {
        reason: "cannot prove predicate with available facts".to_string(),
    }
}

fn discharge_refinement(from: &str, to: &str, _ctx: &ProofContext) -> ProofResult {
    // Simple subtyping: same type is always valid
    if from == to {
        return ProofResult::Proved {
            reason: "identity refinement".to_string(),
        };
    }

    // Integer widening: i32 -> i64
    if from == "i32" && to == "i64" {
        return ProofResult::Proved {
            reason: "integer widening i32 -> i64".to_string(),
        };
    }

    // Refinement types with matching constructors
    if let (Some(from_base), Some(to_base)) = (
        from.strip_prefix("!arc.refined<")
            .and_then(|s| s.strip_suffix('>')),
        to.strip_prefix("!arc.refined<")
            .and_then(|s| s.strip_suffix('>')),
    ) {
        if from_base == to_base {
            return ProofResult::Proved {
                reason: "matching refinement constructor".to_string(),
            };
        }
    }

    ProofResult::Unproved {
        reason: format!("cannot prove refinement {} -> {}", from, to),
    }
}

fn discharge_authority(capability: &str, ctx: &ProofContext) -> ProofResult {
    if ctx.authorities.contains(&capability.to_string()) {
        ProofResult::Proved {
            reason: format!("authority for {} is in context", capability),
        }
    } else {
        ProofResult::Unproved {
            reason: format!("no authority token for {}", capability),
        }
    }
}

// ---------------------------------------------------------------------------
// Solver trait and built-in linear arithmetic solver
// ---------------------------------------------------------------------------

/// A constraint solver backend. Implementations can wrap SMT solvers,
/// decision procedures, or custom logic.
pub trait Solver {
    /// Check if an expression is satisfiable given known facts.
    /// Returns `Some(true)` if provably satisfiable, `Some(false)` if
    /// provably unsatisfiable, `None` if unknown.
    fn check_sat(&self, expr: &Expr, ctx: &ProofContext) -> Option<bool>;

    /// Check if an expression is valid (true for all assignments) given
    /// known facts. Default: check that negation is unsatisfiable.
    fn check_valid(&self, expr: &Expr, ctx: &ProofContext) -> Option<bool> {
        match self.check_sat(&Expr::Not(Box::new(expr.clone())), ctx) {
            Some(false) => Some(true), // negation unsat → valid
            Some(true) => Some(false), // negation sat → not valid
            None => None,
        }
    }

    fn name(&self) -> &str;
}

/// A simple linear arithmetic solver that can handle:
/// - Constant expressions
/// - Variable bounds from context
/// - Simple linear comparisons (x < c, x <= c, x == c)
/// - Conjunctions of the above
pub struct LinearArithmeticSolver;

impl LinearArithmeticSolver {
    /// Try to determine the possible range of an expression given context bounds.
    fn expr_range(&self, expr: &Expr, ctx: &ProofContext) -> Option<(i64, i64)> {
        match expr {
            Expr::Lit(v) => Some((*v, *v)),
            Expr::Var(name) => ctx.bounds.get(name).copied(),
            Expr::Add(l, r) => {
                let (l_lo, l_hi) = self.expr_range(l, ctx)?;
                let (r_lo, r_hi) = self.expr_range(r, ctx)?;
                Some((l_lo.saturating_add(r_lo), l_hi.saturating_add(r_hi)))
            }
            Expr::Sub(l, r) => {
                let (l_lo, l_hi) = self.expr_range(l, ctx)?;
                let (r_lo, r_hi) = self.expr_range(r, ctx)?;
                Some((l_lo.saturating_sub(r_hi), l_hi.saturating_sub(r_lo)))
            }
            Expr::Mul(l, r) => {
                let (l_lo, l_hi) = self.expr_range(l, ctx)?;
                let (r_lo, r_hi) = self.expr_range(r, ctx)?;
                let products = [
                    l_lo.saturating_mul(r_lo),
                    l_lo.saturating_mul(r_hi),
                    l_hi.saturating_mul(r_lo),
                    l_hi.saturating_mul(r_hi),
                ];
                Some((*products.iter().min()?, *products.iter().max()?))
            }
            _ => None,
        }
    }
}

impl Solver for LinearArithmeticSolver {
    fn name(&self) -> &str {
        "linear_arithmetic"
    }

    fn check_sat(&self, expr: &Expr, ctx: &ProofContext) -> Option<bool> {
        let simplified = expr.simplify();

        // Constant: 0 = unsat, nonzero = sat
        match &simplified {
            Expr::Lit(0) => return Some(false),
            Expr::Lit(_) => return Some(true),
            _ => {}
        }

        // Check comparisons using range analysis
        match &simplified {
            Expr::Lt(l, r) => {
                let l_range = self.expr_range(l, ctx)?;
                let r_range = self.expr_range(r, ctx)?;
                // Always true: max(l) < min(r)
                if l_range.1 < r_range.0 {
                    return Some(true);
                }
                // Never true: min(l) >= max(r)
                if l_range.0 >= r_range.1 {
                    return Some(false);
                }
            }
            Expr::Le(l, r) => {
                let l_range = self.expr_range(l, ctx)?;
                let r_range = self.expr_range(r, ctx)?;
                if l_range.1 <= r_range.0 {
                    return Some(true);
                }
                if l_range.0 > r_range.1 {
                    return Some(false);
                }
            }
            Expr::Eq(l, r) => {
                let l_range = self.expr_range(l, ctx)?;
                let r_range = self.expr_range(r, ctx)?;
                // If ranges don't overlap, never equal
                if l_range.1 < r_range.0 || r_range.1 < l_range.0 {
                    return Some(false);
                }
                // If both are singletons and equal, always true
                if l_range.0 == l_range.1 && r_range.0 == r_range.1 && l_range.0 == r_range.0 {
                    return Some(true);
                }
            }
            Expr::And(l, r) => {
                // Both must be sat
                let l_sat = self.check_sat(l, ctx)?;
                if !l_sat {
                    return Some(false);
                }
                let r_sat = self.check_sat(r, ctx)?;
                if !r_sat {
                    return Some(false);
                }
                return Some(true);
            }
            Expr::Not(inner) => {
                // Avoid recursion through check_valid → check_sat(Not(...)).
                // Instead, directly compute based on range/comparison analysis.
                match &**inner {
                    Expr::Lit(0) => return Some(true),
                    Expr::Lit(_) => return Some(false),
                    Expr::Lt(l, r) => {
                        // not(l < r)  ↔  l >= r
                        if let (Some(l_range), Some(r_range)) =
                            (self.expr_range(l, ctx), self.expr_range(r, ctx))
                        {
                            // Always true (l >= r): min(l) >= max(r)
                            if l_range.0 >= r_range.1 {
                                return Some(true);
                            }
                            // Never true (l < r always): max(l) < min(r)
                            if l_range.1 < r_range.0 {
                                return Some(false);
                            }
                            // Satisfiable if max(l) >= min(r) (some l >= r exists)
                            if l_range.1 >= r_range.0 {
                                return Some(true);
                            }
                        }
                    }
                    Expr::Le(l, r) => {
                        // not(l <= r) ↔ l > r
                        if let (Some(l_range), Some(r_range)) =
                            (self.expr_range(l, ctx), self.expr_range(r, ctx))
                        {
                            if l_range.0 > r_range.1 {
                                return Some(true);
                            }
                            if l_range.1 <= r_range.0 {
                                return Some(false);
                            }
                            // Satisfiable if max(l) > min(r)
                            if l_range.1 > r_range.0 {
                                return Some(true);
                            }
                        }
                    }
                    Expr::Eq(l, r) => {
                        // not(l == r) ↔ l != r
                        if let (Some(l_range), Some(r_range)) =
                            (self.expr_range(l, ctx), self.expr_range(r, ctx))
                        {
                            // Ranges don't overlap → always not-equal → sat
                            if l_range.1 < r_range.0 || r_range.1 < l_range.0 {
                                return Some(true);
                            }
                            // Both singletons and equal → never not-equal → unsat
                            if l_range.0 == l_range.1
                                && r_range.0 == r_range.1
                                && l_range.0 == r_range.0
                            {
                                return Some(false);
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Check against known facts
        for fact in &ctx.facts {
            if fact.simplify() == simplified {
                return Some(true);
            }
        }

        None
    }
}

/// Attempt to discharge a proof obligation using a solver backend.
pub fn discharge_with_solver(obligation: &ProofObligation, solver: &dyn Solver) -> ProofResult {
    // First try the built-in discharge
    let result = discharge(obligation);
    if result.is_proved() || result.is_disproved() {
        return result;
    }

    // Fall back to the solver for predicates and bounds checks
    match &obligation.kind {
        ObligationKind::Predicate { expr } => match solver.check_valid(expr, &obligation.context) {
            Some(true) => ProofResult::Proved {
                reason: format!("proved by {} solver", solver.name()),
            },
            Some(false) => ProofResult::Disproved {
                reason: format!("disproved by {} solver", solver.name()),
            },
            None => ProofResult::Unproved {
                reason: format!("{} solver returned unknown", solver.name()),
            },
        },
        ObligationKind::BoundsCheck { index, bound } => {
            // Check: 0 <= index AND index < bound
            let check = Expr::And(
                Box::new(Expr::Le(Box::new(Expr::Lit(0)), Box::new(index.clone()))),
                Box::new(Expr::Lt(Box::new(index.clone()), Box::new(bound.clone()))),
            );
            match solver.check_valid(&check, &obligation.context) {
                Some(true) => ProofResult::Proved {
                    reason: format!("bounds proved by {} solver", solver.name()),
                },
                Some(false) => ProofResult::Disproved {
                    reason: format!("bounds disproved by {} solver", solver.name()),
                },
                None => ProofResult::Unproved {
                    reason: format!("{} solver returned unknown for bounds", solver.name()),
                },
            }
        }
        _ => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expr_eval_arithmetic() {
        let expr = Expr::Add(
            Box::new(Expr::Lit(3)),
            Box::new(Expr::Mul(Box::new(Expr::Lit(4)), Box::new(Expr::Lit(5)))),
        );
        let result = expr.eval(&HashMap::new()).unwrap();
        assert_eq!(result, 23);
    }

    #[test]
    fn expr_eval_with_vars() {
        let expr = Expr::Add(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(1)));
        let mut env = HashMap::new();
        env.insert("x".to_string(), 10);
        assert_eq!(expr.eval(&env), Some(11));
    }

    #[test]
    fn expr_simplify_identity() {
        let expr = Expr::Add(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(0)));
        let simplified = expr.simplify();
        assert_eq!(simplified, Expr::Var("x".into()));
    }

    #[test]
    fn expr_simplify_constant_fold() {
        let expr = Expr::Mul(Box::new(Expr::Lit(6)), Box::new(Expr::Lit(7)));
        assert_eq!(expr.simplify(), Expr::Lit(42));
    }

    #[test]
    fn expr_simplify_self_sub() {
        let expr = Expr::Sub(
            Box::new(Expr::Var("x".into())),
            Box::new(Expr::Var("x".into())),
        );
        assert_eq!(expr.simplify(), Expr::Lit(0));
    }

    #[test]
    fn expr_simplify_self_eq() {
        let expr = Expr::Eq(
            Box::new(Expr::Var("x".into())),
            Box::new(Expr::Var("x".into())),
        );
        assert_eq!(expr.simplify(), Expr::Lit(1));
    }

    #[test]
    fn bounds_check_literal_pass() {
        let obligation = ProofObligation {
            kind: ObligationKind::BoundsCheck {
                index: Expr::Lit(2),
                bound: Expr::Lit(10),
            },
            description: "array access".to_string(),
            context: ProofContext::new(),
        };
        assert!(discharge(&obligation).is_proved());
    }

    #[test]
    fn bounds_check_literal_fail() {
        let obligation = ProofObligation {
            kind: ObligationKind::BoundsCheck {
                index: Expr::Lit(10),
                bound: Expr::Lit(10),
            },
            description: "array access".to_string(),
            context: ProofContext::new(),
        };
        assert!(discharge(&obligation).is_disproved());
    }

    #[test]
    fn bounds_check_negative_index() {
        let obligation = ProofObligation {
            kind: ObligationKind::BoundsCheck {
                index: Expr::Lit(-1),
                bound: Expr::Lit(10),
            },
            description: "array access".to_string(),
            context: ProofContext::new(),
        };
        assert!(discharge(&obligation).is_disproved());
    }

    #[test]
    fn bounds_check_with_known_bounds() {
        let mut ctx = ProofContext::new();
        ctx.add_bound("i", 0, 7);
        let obligation = ProofObligation {
            kind: ObligationKind::BoundsCheck {
                index: Expr::Var("i".into()),
                bound: Expr::Lit(10),
            },
            description: "array access".to_string(),
            context: ctx,
        };
        assert!(discharge(&obligation).is_proved());
    }

    #[test]
    fn predicate_tautology() {
        let obligation = ProofObligation {
            kind: ObligationKind::Predicate {
                expr: Expr::Eq(
                    Box::new(Expr::Var("x".into())),
                    Box::new(Expr::Var("x".into())),
                ),
            },
            description: "reflexivity".to_string(),
            context: ProofContext::new(),
        };
        assert!(discharge(&obligation).is_proved());
    }

    #[test]
    fn predicate_from_context() {
        let mut ctx = ProofContext::new();
        ctx.add_fact(Expr::Lt(
            Box::new(Expr::Var("x".into())),
            Box::new(Expr::Lit(10)),
        ));
        let obligation = ProofObligation {
            kind: ObligationKind::Predicate {
                expr: Expr::Lt(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(10))),
            },
            description: "from context".to_string(),
            context: ctx,
        };
        assert!(discharge(&obligation).is_proved());
    }

    #[test]
    fn authority_with_token() {
        let mut ctx = ProofContext::new();
        ctx.add_authority("email.send");
        let obligation = ProofObligation {
            kind: ObligationKind::Authority {
                capability: "email.send".to_string(),
            },
            description: "email auth".to_string(),
            context: ctx,
        };
        assert!(discharge(&obligation).is_proved());
    }

    #[test]
    fn authority_without_token() {
        let obligation = ProofObligation {
            kind: ObligationKind::Authority {
                capability: "file.delete".to_string(),
            },
            description: "file auth".to_string(),
            context: ProofContext::new(),
        };
        let result = discharge(&obligation);
        assert!(!result.is_proved());
    }

    #[test]
    fn refinement_identity() {
        let obligation = ProofObligation {
            kind: ObligationKind::Refinement {
                from_type: "i64".to_string(),
                to_type: "i64".to_string(),
            },
            description: "identity".to_string(),
            context: ProofContext::new(),
        };
        assert!(discharge(&obligation).is_proved());
    }

    #[test]
    fn refinement_widening() {
        let obligation = ProofObligation {
            kind: ObligationKind::Refinement {
                from_type: "i32".to_string(),
                to_type: "i64".to_string(),
            },
            description: "widen".to_string(),
            context: ProofContext::new(),
        };
        assert!(discharge(&obligation).is_proved());
    }

    // --- Solver / LinearArithmeticSolver tests ---

    #[test]
    fn solver_range_literal() {
        let solver = LinearArithmeticSolver;
        let ctx = ProofContext::new();
        assert_eq!(solver.expr_range(&Expr::Lit(5), &ctx), Some((5, 5)));
    }

    #[test]
    fn solver_range_var_with_bounds() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 10);
        assert_eq!(
            solver.expr_range(&Expr::Var("x".into()), &ctx),
            Some((0, 10))
        );
    }

    #[test]
    fn solver_range_add() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 1, 5);
        let expr = Expr::Add(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(10)));
        assert_eq!(solver.expr_range(&expr, &ctx), Some((11, 15)));
    }

    #[test]
    fn solver_range_sub() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 10, 20);
        let expr = Expr::Sub(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(5)));
        assert_eq!(solver.expr_range(&expr, &ctx), Some((5, 15)));
    }

    #[test]
    fn solver_range_mul() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 2, 3);
        let expr = Expr::Mul(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(4)));
        assert_eq!(solver.expr_range(&expr, &ctx), Some((8, 12)));
    }

    #[test]
    fn solver_check_sat_constant() {
        let solver = LinearArithmeticSolver;
        let ctx = ProofContext::new();
        assert_eq!(solver.check_sat(&Expr::Lit(1), &ctx), Some(true));
        assert_eq!(solver.check_sat(&Expr::Lit(0), &ctx), Some(false));
    }

    #[test]
    fn solver_check_sat_lt_always_true() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 5);
        // x < 10 is always true when x in [0,5]
        let expr = Expr::Lt(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(10)));
        assert_eq!(solver.check_sat(&expr, &ctx), Some(true));
    }

    #[test]
    fn solver_check_sat_lt_always_false() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 10, 20);
        // x < 5 is never true when x in [10,20]
        let expr = Expr::Lt(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(5)));
        assert_eq!(solver.check_sat(&expr, &ctx), Some(false));
    }

    #[test]
    fn solver_check_sat_eq_disjoint() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 3);
        // x == 10: ranges [0,3] and [10,10] don't overlap → unsat
        let expr = Expr::Eq(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(10)));
        assert_eq!(solver.check_sat(&expr, &ctx), Some(false));
    }

    #[test]
    fn solver_check_valid_le() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 5);
        // x <= 5 is valid when x in [0,5]
        let expr = Expr::Le(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(5)));
        assert_eq!(solver.check_valid(&expr, &ctx), Some(true));
    }

    #[test]
    fn solver_check_valid_lt_not_valid() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 10);
        // x < 5 is not valid when x can be up to 10
        let expr = Expr::Lt(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(5)));
        assert_eq!(solver.check_valid(&expr, &ctx), Some(false));
    }

    #[test]
    fn discharge_with_solver_bounds_proved() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("i", 0, 7);
        let obligation = ProofObligation {
            kind: ObligationKind::BoundsCheck {
                index: Expr::Var("i".into()),
                bound: Expr::Lit(10),
            },
            description: "array access".to_string(),
            context: ctx,
        };
        // Built-in discharge handles this, so solver fallback should still succeed
        let result = discharge_with_solver(&obligation, &solver);
        assert!(result.is_proved());
    }

    #[test]
    fn discharge_with_solver_predicate_proved() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 5);
        // Predicate: x < 10 — always true when x in [0,5]
        let obligation = ProofObligation {
            kind: ObligationKind::Predicate {
                expr: Expr::Lt(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(10))),
            },
            description: "range check".to_string(),
            context: ctx,
        };
        let result = discharge_with_solver(&obligation, &solver);
        assert!(result.is_proved());
    }

    #[test]
    fn discharge_with_solver_predicate_disproved() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 10, 20);
        // Predicate: x < 5 — never true when x in [10,20]
        let obligation = ProofObligation {
            kind: ObligationKind::Predicate {
                expr: Expr::Lt(Box::new(Expr::Var("x".into())), Box::new(Expr::Lit(5))),
            },
            description: "range check".to_string(),
            context: ctx,
        };
        let result = discharge_with_solver(&obligation, &solver);
        assert!(result.is_disproved());
    }

    #[test]
    fn solver_and_conjunction() {
        let solver = LinearArithmeticSolver;
        let mut ctx = ProofContext::new();
        ctx.add_bound("x", 0, 5);
        // x < 10 AND x >= 0  — both sat
        let expr = Expr::And(
            Box::new(Expr::Lt(
                Box::new(Expr::Var("x".into())),
                Box::new(Expr::Lit(10)),
            )),
            Box::new(Expr::Le(
                Box::new(Expr::Lit(0)),
                Box::new(Expr::Var("x".into())),
            )),
        );
        assert_eq!(solver.check_sat(&expr, &ctx), Some(true));
    }
}
