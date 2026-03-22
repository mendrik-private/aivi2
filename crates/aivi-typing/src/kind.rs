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

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TypeConstructorId(u32);

impl TypeConstructorId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("type constructor table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct KindParameterId(u32);

impl KindParameterId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("kind parameter table overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct KindExprId(u32);

impl KindExprId {
    fn from_index(index: usize) -> Self {
        Self(index.try_into().expect("kind expression arena overflow"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

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

#[derive(Copy, Clone, Debug, Default)]
pub struct KindChecker;

impl KindChecker {
    pub fn infer(self, store: &KindStore, root: KindExprId) -> Result<Kind, KindCheckError> {
        let mut inferred = vec![None; store.exprs.len()];
        let mut stack = vec![Frame::Enter(root)];

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(expr) => {
                    if inferred[expr.index()].is_some() {
                        continue;
                    }
                    match store.expr(expr) {
                        KindExpr::Parameter(parameter) => {
                            let _ = store.parameter(*parameter);
                            inferred[expr.index()] = Some(Kind::Type);
                        }
                        KindExpr::Constructor(constructor) => {
                            inferred[expr.index()] =
                                Some(store.constructor(*constructor).kind().clone());
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
                    let callee_kind = inferred[callee.index()]
                        .clone()
                        .expect("callee kind should be inferred before application exits");
                    let argument_kind = inferred[argument.index()]
                        .clone()
                        .expect("argument kind should be inferred before application exits");
                    match callee_kind {
                        Kind::Arrow(expected, result) => {
                            let expected = *expected;
                            if expected != argument_kind {
                                return Err(KindCheckError {
                                    expr,
                                    kind: KindCheckErrorKind::ArgumentKindMismatch {
                                        callee,
                                        expected,
                                        argument,
                                        found: argument_kind,
                                    },
                                });
                            }
                            inferred[expr.index()] = Some(*result);
                        }
                        callee_kind => {
                            return Err(KindCheckError {
                                expr,
                                kind: KindCheckErrorKind::CannotApplyNonConstructor {
                                    callee,
                                    callee_kind,
                                },
                            });
                        }
                    }
                }
                Frame::ExitExpectType { expr, children } => {
                    for child in children {
                        let child_kind = inferred[child.index()]
                            .clone()
                            .expect("child kind should be inferred before structural checks exit");
                        if child_kind != Kind::Type {
                            return Err(KindCheckError {
                                expr,
                                kind: KindCheckErrorKind::ExpectedType {
                                    child,
                                    found: child_kind,
                                },
                            });
                        }
                    }
                    inferred[expr.index()] = Some(Kind::Type);
                }
            }
        }

        inferred[root.index()]
            .clone()
            .ok_or_else(|| panic!("root kind should be inferred before inference returns"))
    }

    pub fn expect_kind(
        self,
        store: &KindStore,
        expr: KindExprId,
        expected: &Kind,
    ) -> Result<(), KindCheckError> {
        let found = self.infer(store, expr)?;
        if &found == expected {
            return Ok(());
        }
        Err(KindCheckError {
            expr,
            kind: KindCheckErrorKind::ExpectedKind {
                expected: expected.clone(),
                found,
            },
        })
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
}
