use std::fmt;

/// Small explicit kind language for Milestone 3.
///
/// The current implementation supports only the RFC's v1 kind set:
/// `Type` and right-associative arrows between `Type`-kinded arguments.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Kind {
    Type,
    Arrow(Box<Kind>, Box<Kind>),
}

impl Kind {
    pub fn arrow(parameter: Kind, result: Kind) -> Self {
        Self::Arrow(Box::new(parameter), Box::new(result))
    }

    pub fn constructor(arity: usize) -> Self {
        let mut kind = Self::Type;
        for _ in 0..arity {
            kind = Self::arrow(Self::Type, kind);
        }
        kind
    }

    /// Number of type parameters this kind accepts (e.g. `* -> * -> *` has arity 2).
    pub fn arity(&self) -> usize {
        match self {
            Self::Type => 0,
            Self::Arrow(_, result) => 1 + result.arity(),
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Type => write!(f, "Type"),
            Self::Arrow(parameter, result) => {
                if matches!(parameter.as_ref(), Kind::Arrow(..)) {
                    write!(f, "({parameter}) -> {result}")
                } else {
                    write!(f, "{parameter} -> {result}")
                }
            }
        }
    }
}

use crate::eq::define_typing_id;

define_typing_id!(pub TypeConstructorId, "type constructor table overflow");
define_typing_id!(pub KindParameterId, "kind parameter table overflow");
define_typing_id!(pub KindExprId, "kind expression arena overflow");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeConstructor {
    name: Box<str>,
    kind: Kind,
}

impl TypeConstructor {
    pub fn new(name: impl Into<String>, kind: Kind) -> Self {
        Self {
            name: name.into().into_boxed_str(),
            kind,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindParameter {
    name: Box<str>,
}

impl KindParameter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into().into_boxed_str(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Focused type-expression surface for kind checking.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KindExpr {
    Parameter(KindParameterId),
    Constructor(TypeConstructorId),
    Apply {
        callee: KindExprId,
        argument: KindExprId,
    },
    Tuple(Vec<KindExprId>),
    Record(Vec<KindRecordField>),
    Arrow {
        parameter: KindExprId,
        result: KindExprId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindRecordField {
    name: Box<str>,
    ty: KindExprId,
}

impl KindRecordField {
    pub fn new(name: impl Into<String>, ty: KindExprId) -> Self {
        Self {
            name: name.into().into_boxed_str(),
            ty,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn ty(&self) -> KindExprId {
        self.ty
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KindStore {
    constructors: Vec<TypeConstructor>,
    parameters: Vec<KindParameter>,
    exprs: Vec<KindExpr>,
}

impl KindStore {
    pub fn add_constructor(&mut self, name: impl Into<String>, kind: Kind) -> TypeConstructorId {
        let id = TypeConstructorId::from_index(self.constructors.len());
        self.constructors.push(TypeConstructor::new(name, kind));
        id
    }

    pub fn add_parameter(&mut self, name: impl Into<String>) -> KindParameterId {
        let id = KindParameterId::from_index(self.parameters.len());
        self.parameters.push(KindParameter::new(name));
        id
    }

    pub fn constructor(&self, id: TypeConstructorId) -> &TypeConstructor {
        &self.constructors[id.index()]
    }

    pub fn parameter(&self, id: KindParameterId) -> &KindParameter {
        &self.parameters[id.index()]
    }

    pub fn expr(&self, id: KindExprId) -> &KindExpr {
        &self.exprs[id.index()]
    }

    pub fn constructor_expr(&mut self, constructor: TypeConstructorId) -> KindExprId {
        self.push_expr(KindExpr::Constructor(constructor))
    }

    pub fn parameter_expr(&mut self, parameter: KindParameterId) -> KindExprId {
        self.push_expr(KindExpr::Parameter(parameter))
    }

    pub fn apply_expr(&mut self, callee: KindExprId, argument: KindExprId) -> KindExprId {
        self.push_expr(KindExpr::Apply { callee, argument })
    }

    pub fn tuple_expr(&mut self, members: Vec<KindExprId>) -> KindExprId {
        self.push_expr(KindExpr::Tuple(members))
    }

    pub fn record_expr(&mut self, fields: Vec<KindRecordField>) -> KindExprId {
        self.push_expr(KindExpr::Record(fields))
    }

    pub fn arrow_expr(&mut self, parameter: KindExprId, result: KindExprId) -> KindExprId {
        self.push_expr(KindExpr::Arrow { parameter, result })
    }

    fn push_expr(&mut self, expr: KindExpr) -> KindExprId {
        let id = KindExprId::from_index(self.exprs.len());
        self.exprs.push(expr);
        id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindCheckError {
    expr: KindExprId,
    kind: KindCheckErrorKind,
}

impl KindCheckError {
    pub fn expr(&self) -> KindExprId {
        self.expr
    }

    pub fn kind(&self) -> &KindCheckErrorKind {
        &self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KindCheckErrorKind {
    CannotApplyNonConstructor {
        callee: KindExprId,
        callee_kind: Kind,
    },
    ArgumentKindMismatch {
        callee: KindExprId,
        expected: Kind,
        argument: KindExprId,
        found: Kind,
    },
    ExpectedType {
        child: KindExprId,
        found: Kind,
    },
    ExpectedKind {
        expected: Kind,
        found: Kind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindSolution {
    parameter_kinds: Vec<Kind>,
    expr_kinds: Vec<Kind>,
}

impl KindSolution {
    pub fn parameter_kind(&self, id: KindParameterId) -> &Kind {
        &self.parameter_kinds[id.index()]
    }

    pub fn expr_kind(&self, id: KindExprId) -> &Kind {
        &self.expr_kinds[id.index()]
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct KindChecker;

impl KindChecker {
    pub fn infer_with_solution(
        self,
        store: &KindStore,
        root: KindExprId,
    ) -> Result<KindSolution, KindCheckError> {
        KindInference::new(store).solve(root, None)
    }

    pub fn expect_kind_with_solution(
        self,
        store: &KindStore,
        expr: KindExprId,
        expected: &Kind,
    ) -> Result<KindSolution, KindCheckError> {
        KindInference::new(store).solve(expr, Some(expected))
    }

    pub fn infer(self, store: &KindStore, root: KindExprId) -> Result<Kind, KindCheckError> {
        Ok(self
            .infer_with_solution(store, root)?
            .expr_kind(root)
            .clone())
    }

    pub fn expect_kind(
        self,
        store: &KindStore,
        expr: KindExprId,
        expected: &Kind,
    ) -> Result<(), KindCheckError> {
        self.expect_kind_with_solution(store, expr, expected)
            .map(|_| ())
    }
}

#[derive(Clone, Debug)]
enum Frame {
    Enter(KindExprId),
    ExitApply {
        expr: KindExprId,
        callee: KindExprId,
        argument: KindExprId,
    },
    ExitExpectType {
        expr: KindExprId,
        children: Vec<KindExprId>,
    },
}

#[derive(Clone, Debug)]
enum KindTerm {
    Type,
    Arrow {
        parameter: KindTermId,
        result: KindTermId,
    },
    Var(KindVarId),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct KindTermId(u32);

impl KindTermId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("kind term arena overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct KindVarId(u32);

impl KindVarId {
    fn from_index(index: usize) -> Self {
        Self(
            index
                .try_into()
                .expect("kind inference variable table overflow"),
        )
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

struct KindInference<'a> {
    store: &'a KindStore,
    terms: Vec<KindTerm>,
    bindings: Vec<Option<KindTermId>>,
    parameter_terms: Vec<KindTermId>,
    expr_terms: Vec<Option<KindTermId>>,
    type_term: KindTermId,
}

impl<'a> KindInference<'a> {
    fn new(store: &'a KindStore) -> Self {
        let mut terms = vec![KindTerm::Type];
        let type_term = KindTermId::from_index(0);
        let mut bindings = Vec::new();
        let mut parameter_terms = Vec::with_capacity(store.parameters.len());
        for _ in &store.parameters {
            let var = KindVarId::from_index(bindings.len());
            bindings.push(None);
            let term = KindTermId::from_index(terms.len());
            terms.push(KindTerm::Var(var));
            parameter_terms.push(term);
        }
        Self {
            store,
            terms,
            bindings,
            parameter_terms,
            expr_terms: vec![None; store.exprs.len()],
            type_term,
        }
    }

    fn solve(
        mut self,
        root: KindExprId,
        expected: Option<&Kind>,
    ) -> Result<KindSolution, KindCheckError> {
        let mut stack = vec![Frame::Enter(root)];

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(expr) => {
                    if self.expr_terms[expr.index()].is_some() {
                        continue;
                    }
                    match self.store.expr(expr) {
                        KindExpr::Parameter(parameter) => {
                            let _ = self.store.parameter(*parameter);
                            self.expr_terms[expr.index()] =
                                Some(self.parameter_terms[parameter.index()]);
                        }
                        KindExpr::Constructor(constructor) => {
                            self.expr_terms[expr.index()] =
                                Some(self.kind_term(self.store.constructor(*constructor).kind()));
                        }
                        KindExpr::Apply { callee, argument } => {
                            stack.push(Frame::ExitApply {
                                expr,
                                callee: *callee,
                                argument: *argument,
                            });
                            stack.push(Frame::Enter(*argument));
                            stack.push(Frame::Enter(*callee));
                        }
                        KindExpr::Tuple(members) => {
                            stack.push(Frame::ExitExpectType {
                                expr,
                                children: members.clone(),
                            });
                            for member in members.iter().rev() {
                                stack.push(Frame::Enter(*member));
                            }
                        }
                        KindExpr::Record(fields) => {
                            let children =
                                fields.iter().map(|field| field.ty()).collect::<Vec<_>>();
                            stack.push(Frame::ExitExpectType { expr, children });
                            for child in fields.iter().rev().map(KindRecordField::ty) {
                                stack.push(Frame::Enter(child));
                            }
                        }
                        KindExpr::Arrow { parameter, result } => {
                            stack.push(Frame::ExitExpectType {
                                expr,
                                children: vec![*parameter, *result],
                            });
                            stack.push(Frame::Enter(*result));
                            stack.push(Frame::Enter(*parameter));
                        }
                    }
                }
                Frame::ExitApply {
                    expr,
                    callee,
                    argument,
                } => {
                    let callee_term = self.expr_terms[callee.index()]
                        .expect("callee kind term should be inferred before application exits");
                    let argument_term = self.expr_terms[argument.index()]
                        .expect("argument kind term should be inferred before application exits");
                    let callee_resolved = self.resolve(callee_term);
                    let result_term = self.fresh_var_term();

                    match self.terms[callee_resolved.index()] {
                        KindTerm::Arrow { parameter, result } => {
                            if !self.unify(parameter, argument_term) {
                                return Err(KindCheckError {
                                    expr,
                                    kind: KindCheckErrorKind::ArgumentKindMismatch {
                                        callee,
                                        expected: self.term_to_kind(parameter),
                                        argument,
                                        found: self.term_to_kind(argument_term),
                                    },
                                });
                            }
                            if !self.unify(result_term, result) {
                                return Err(KindCheckError {
                                    expr,
                                    kind: KindCheckErrorKind::ExpectedKind {
                                        expected: self.term_to_kind(result),
                                        found: self.term_to_kind(result_term),
                                    },
                                });
                            }
                            self.expr_terms[expr.index()] = Some(result_term);
                        }
                        KindTerm::Type => {
                            return Err(KindCheckError {
                                expr,
                                kind: KindCheckErrorKind::CannotApplyNonConstructor {
                                    callee,
                                    callee_kind: Kind::Type,
                                },
                            });
                        }
                        KindTerm::Var(_) => {
                            let expected_parameter = self.fresh_var_term();
                            let arrow_term = self.arrow_term(expected_parameter, result_term);
                            if !self.unify(callee_term, arrow_term) {
                                return Err(KindCheckError {
                                    expr,
                                    kind: KindCheckErrorKind::CannotApplyNonConstructor {
                                        callee,
                                        callee_kind: self.term_to_kind(callee_term),
                                    },
                                });
                            }
                            if !self.unify(expected_parameter, argument_term) {
                                return Err(KindCheckError {
                                    expr,
                                    kind: KindCheckErrorKind::ArgumentKindMismatch {
                                        callee,
                                        expected: self.term_to_kind(expected_parameter),
                                        argument,
                                        found: self.term_to_kind(argument_term),
                                    },
                                });
                            }
                            self.expr_terms[expr.index()] = Some(result_term);
                        }
                    }
                }
                Frame::ExitExpectType { expr, children } => {
                    for child in children {
                        let child_term = self.expr_terms[child.index()].expect(
                            "child kind term should be inferred before structural checks exit",
                        );
                        if !self.unify(child_term, self.type_term) {
                            return Err(KindCheckError {
                                expr,
                                kind: KindCheckErrorKind::ExpectedType {
                                    child,
                                    found: self.term_to_kind(child_term),
                                },
                            });
                        }
                    }
                    self.expr_terms[expr.index()] = Some(self.type_term);
                }
            }
        }

        let root_term = self.expr_terms[root.index()]
            .expect("root kind term should be inferred before solver returns");
        if let Some(expected) = expected {
            let expected_term = self.kind_term(expected);
            if !self.unify(root_term, expected_term) {
                return Err(KindCheckError {
                    expr: root,
                    kind: KindCheckErrorKind::ExpectedKind {
                        expected: expected.clone(),
                        found: self.term_to_kind(root_term),
                    },
                });
            }
        }

        let parameter_terms = self.parameter_terms.clone();
        let expr_terms = self.expr_terms.clone();
        Ok(KindSolution {
            parameter_kinds: parameter_terms
                .into_iter()
                .map(|term| self.term_to_kind(term))
                .collect(),
            expr_kinds: expr_terms
                .into_iter()
                .map(|term| {
                    term.map(|term| self.term_to_kind(term))
                        .unwrap_or(Kind::Type)
                })
                .collect(),
        })
    }

    fn kind_term(&mut self, kind: &Kind) -> KindTermId {
        match kind {
            Kind::Type => self.type_term,
            Kind::Arrow(parameter, result) => {
                let parameter = self.kind_term(parameter);
                let result = self.kind_term(result);
                self.arrow_term(parameter, result)
            }
        }
    }

    fn arrow_term(&mut self, parameter: KindTermId, result: KindTermId) -> KindTermId {
        let id = KindTermId::from_index(self.terms.len());
        self.terms.push(KindTerm::Arrow { parameter, result });
        id
    }

    fn fresh_var_term(&mut self) -> KindTermId {
        let var = KindVarId::from_index(self.bindings.len());
        self.bindings.push(None);
        let id = KindTermId::from_index(self.terms.len());
        self.terms.push(KindTerm::Var(var));
        id
    }

    fn resolve(&mut self, term: KindTermId) -> KindTermId {
        let mut current = term;
        let mut trail = Vec::new();
        loop {
            match self.terms[current.index()] {
                KindTerm::Var(var) => match self.bindings[var.index()] {
                    Some(next) => {
                        trail.push(current);
                        current = next;
                    }
                    None => break,
                },
                KindTerm::Type | KindTerm::Arrow { .. } => break,
            }
        }
        for seen in trail {
            if let KindTerm::Var(var) = self.terms[seen.index()] {
                self.bindings[var.index()] = Some(current);
            }
        }
        current
    }

    fn unify(&mut self, left: KindTermId, right: KindTermId) -> bool {
        let mut work = vec![(left, right)];
        while let Some((left, right)) = work.pop() {
            let left = self.resolve(left);
            let right = self.resolve(right);
            if left == right {
                continue;
            }
            match (
                self.terms[left.index()].clone(),
                self.terms[right.index()].clone(),
            ) {
                (KindTerm::Type, KindTerm::Type) => {}
                (
                    KindTerm::Arrow {
                        parameter: left_parameter,
                        result: left_result,
                    },
                    KindTerm::Arrow {
                        parameter: right_parameter,
                        result: right_result,
                    },
                ) => {
                    work.push((left_parameter, right_parameter));
                    work.push((left_result, right_result));
                }
                (KindTerm::Var(var), _) => {
                    if self.occurs(var, right) {
                        return false;
                    }
                    self.bindings[var.index()] = Some(right);
                }
                (_, KindTerm::Var(var)) => {
                    if self.occurs(var, left) {
                        return false;
                    }
                    self.bindings[var.index()] = Some(left);
                }
                (KindTerm::Type, KindTerm::Arrow { .. })
                | (KindTerm::Arrow { .. }, KindTerm::Type) => return false,
            }
        }
        true
    }

    fn occurs(&mut self, target: KindVarId, term: KindTermId) -> bool {
        let mut stack = vec![term];
        while let Some(term) = stack.pop() {
            let term = self.resolve(term);
            match self.terms[term.index()] {
                KindTerm::Type => {}
                KindTerm::Arrow { parameter, result } => {
                    stack.push(result);
                    stack.push(parameter);
                }
                KindTerm::Var(var) => {
                    if var == target {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn term_to_kind(&mut self, term: KindTermId) -> Kind {
        let term = self.resolve(term);
        match self.terms[term.index()] {
            KindTerm::Type => Kind::Type,
            KindTerm::Arrow { parameter, result } => {
                Kind::arrow(self.term_to_kind(parameter), self.term_to_kind(result))
            }
            KindTerm::Var(_) => Kind::Type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, KindCheckErrorKind, KindChecker, KindRecordField, KindStore};

    fn builtin_store() -> KindStore {
        let mut store = KindStore::default();
        let _ = store.add_constructor("Int", Kind::Type);
        let _ = store.add_constructor("Text", Kind::Type);
        let _ = store.add_constructor("Option", Kind::constructor(1));
        let _ = store.add_constructor("List", Kind::constructor(1));
        let _ = store.add_constructor("Set", Kind::constructor(1));
        let _ = store.add_constructor("Signal", Kind::constructor(1));
        let _ = store.add_constructor("Map", Kind::constructor(2));
        let _ = store.add_constructor("Result", Kind::constructor(2));
        let _ = store.add_constructor("Task", Kind::constructor(2));
        store
    }

    #[test]
    fn infers_partial_application_of_named_constructors() {
        let mut store = builtin_store();
        let http_error = store.add_constructor("HttpError", Kind::Type);
        let result = store.add_constructor("Validation", Kind::constructor(2));

        let result_ctor = store.constructor_expr(result);
        let http_error_expr = store.constructor_expr(http_error);
        let partially_applied = store.apply_expr(result_ctor, http_error_expr);

        assert_eq!(
            KindChecker.infer(&store, partially_applied),
            Ok(Kind::constructor(1))
        );
    }

    #[test]
    fn infers_fully_applied_type_as_type() {
        let mut store = builtin_store();
        let option = store.add_constructor("Option", Kind::constructor(1));
        let text = store.add_constructor("TextAlias", Kind::Type);

        let option_ctor = store.constructor_expr(option);
        let text_expr = store.constructor_expr(text);
        let applied = store.apply_expr(option_ctor, text_expr);

        assert_eq!(KindChecker.infer(&store, applied), Ok(Kind::Type));
    }

    #[test]
    fn infers_map_partial_application_and_fully_applied_set_types() {
        let mut store = builtin_store();
        let map = store.add_constructor("MapAlias", Kind::constructor(2));
        let set = store.add_constructor("SetAlias", Kind::constructor(1));
        let text = store.add_constructor("TextAlias", Kind::Type);

        let map_expr = store.constructor_expr(map);
        let set_expr = store.constructor_expr(set);
        let text_expr = store.constructor_expr(text);
        let map_text = store.apply_expr(map_expr, text_expr);
        let set_text = store.apply_expr(set_expr, text_expr);

        assert_eq!(
            KindChecker.infer(&store, map_text),
            Ok(Kind::constructor(1))
        );
        assert_eq!(KindChecker.infer(&store, set_text), Ok(Kind::Type));
    }

    #[test]
    fn accepts_parameterized_domain_constructor_kinds() {
        let mut store = KindStore::default();
        let non_empty = store.add_constructor("NonEmpty", Kind::constructor(1));
        let element = store.add_parameter("A");

        let non_empty_expr = store.constructor_expr(non_empty);
        let element_expr = store.parameter_expr(element);
        let applied = store.apply_expr(non_empty_expr, element_expr);

        assert_eq!(KindChecker.infer(&store, applied), Ok(Kind::Type));
    }

    #[test]
    fn rejects_applying_non_constructor_types() {
        let mut store = builtin_store();
        let text = store.add_constructor("UserText", Kind::Type);
        let other = store.add_constructor("OtherText", Kind::Type);

        let text_expr = store.constructor_expr(text);
        let other_expr = store.constructor_expr(other);
        let applied = store.apply_expr(text_expr, other_expr);

        let error = KindChecker
            .infer(&store, applied)
            .expect_err("applying a fully saturated type should fail kind inference");
        assert_eq!(
            error.kind(),
            &KindCheckErrorKind::CannotApplyNonConstructor {
                callee: text_expr,
                callee_kind: Kind::Type,
            }
        );
    }

    #[test]
    fn rejects_argument_kind_mismatches() {
        let mut store = builtin_store();
        let option = store.add_constructor("OptionAlias", Kind::constructor(1));
        let result = store.add_constructor("ResultAlias", Kind::constructor(2));

        let result_expr = store.constructor_expr(result);
        let option_expr = store.constructor_expr(option);
        let applied = store.apply_expr(result_expr, option_expr);

        let error = KindChecker
            .infer(&store, applied)
            .expect_err("higher-kinded arguments should be rejected when `Type` is required");
        assert_eq!(
            error.kind(),
            &KindCheckErrorKind::ArgumentKindMismatch {
                callee: result_expr,
                expected: Kind::Type,
                argument: option_expr,
                found: Kind::constructor(1),
            }
        );
    }

    #[test]
    fn rejects_fully_applied_types_where_constructors_are_expected() {
        let mut store = builtin_store();
        let list = store.add_constructor("ListAlias", Kind::constructor(1));
        let int = store.add_constructor("IntAlias", Kind::Type);

        let list_expr = store.constructor_expr(list);
        let int_expr = store.constructor_expr(int);
        let list_int = store.apply_expr(list_expr, int_expr);

        let error = KindChecker
            .expect_kind(&store, list_int, &Kind::constructor(1))
            .expect_err("fully applied types should not satisfy constructor expectations");
        assert_eq!(
            error.kind(),
            &KindCheckErrorKind::ExpectedKind {
                expected: Kind::constructor(1),
                found: Kind::Type,
            }
        );
    }

    #[test]
    fn requires_structural_type_children_to_have_type_kind() {
        let mut store = builtin_store();
        let option = store.add_constructor("OptionAlias", Kind::constructor(1));
        let text = store.add_constructor("TextAlias", Kind::Type);

        let option_expr = store.constructor_expr(option);
        let text_expr = store.constructor_expr(text);
        let record = store.record_expr(vec![
            KindRecordField::new("value", option_expr),
            KindRecordField::new("label", text_expr),
        ]);

        let error = KindChecker
            .infer(&store, record)
            .expect_err("record fields should require `Type`-kinded members");
        assert_eq!(
            error.kind(),
            &KindCheckErrorKind::ExpectedType {
                child: option_expr,
                found: Kind::constructor(1),
            }
        );
    }

    #[test]
    fn expect_kind_infers_higher_kinded_parameter_arity_from_application() {
        let mut store = builtin_store();
        let carrier = store.add_parameter("F");
        let int = store.add_constructor("IntAlias", Kind::Type);

        let carrier_expr = store.parameter_expr(carrier);
        let int_expr = store.constructor_expr(int);
        let applied = store.apply_expr(carrier_expr, int_expr);

        let solution = KindChecker
            .expect_kind_with_solution(&store, applied, &Kind::Type)
            .expect("higher-kinded application should infer the carrier kind");

        assert_eq!(solution.parameter_kind(carrier), &Kind::constructor(1));
        assert_eq!(solution.expr_kind(applied), &Kind::Type);
    }

    #[test]
    fn expect_kind_infers_expected_constructor_kind_for_bare_parameters() {
        let mut store = KindStore::default();
        let carrier = store.add_parameter("F");
        let carrier_expr = store.parameter_expr(carrier);

        let solution = KindChecker
            .expect_kind_with_solution(&store, carrier_expr, &Kind::constructor(1))
            .expect("expected constructor kind should flow into bare parameters");

        assert_eq!(solution.parameter_kind(carrier), &Kind::constructor(1));
    }
}
