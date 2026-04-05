use std::collections::HashMap;

use aivi_base::{Diagnostic, DiagnosticCode, Severity, SourceSpan};
use aivi_syntax as syn;
use aivi_typing::Kind;

use crate::{
    ApplicativeCluster, ApplicativeSpineHead, AtLeastTwo, BigIntLiteral, BinaryOperator, Binding,
    BindingId, BindingKind, BindingPattern, BuiltinTerm, BuiltinType, CaseControl, ClassItem,
    ClassMember, ClusterFinalizer, ClusterPresentation, ControlNode, ControlNodeId, DebugDecorator,
    DecimalLiteral, Decorator, DecoratorCall, DecoratorId, DecoratorPayload, DeprecatedDecorator,
    DomainItem, DomainMember, DomainMemberKind, DomainMemberResolution, EachControl, EmptyControl,
    ExportItem, ExportResolution, Expr, ExprId, ExprKind, FloatLiteral, FragmentControl,
    FunctionItem, FunctionParameter, HoistItem, HoistKindFilter, ImportBinding,
    ImportBindingMetadata, ImportBindingResolution, ImportBundleKind, ImportId,
    ImportModuleResolution, ImportRecordField, ImportValueType, ImportedDomainLiteralSuffix,
    InstanceItem, InstanceMember, IntegerLiteral, IntrinsicValue, Item, ItemHeader, ItemId,
    ItemKind, LiteralSuffixResolution, MapExpr, MapExprEntry, MarkupAttribute,
    MarkupAttributeValue, MarkupElement, MarkupNode, MarkupNodeId, MarkupNodeKind, MatchControl,
    MockDecorator, Module, Name, NamePath, NonEmpty, PatchBlock, PatchEntry, PatchInstruction,
    PatchInstructionKind, PatchSelector, PatchSelectorSegment, Pattern, PatternId, PatternKind,
    PipeExpr, PipeStage, PipeStageKind, ProjectionBase, ReactiveUpdateBodyMode,
    ReactiveUpdateClause, RecordExpr, RecordExprField, RecordFieldSurface, RecordPatternField,
    RecordRowRename, RecordRowTransform, RecurrenceWakeupDecorator, RecurrenceWakeupDecoratorKind,
    RegexLiteral, ResolutionState, Resolved, ShowControl, SignalItem, SourceDecorator,
    SourceProviderContractItem, SourceProviderRef, SuffixedIntegerLiteral, TermReference,
    TermResolution, TestDecorator, TextFragment, TextInterpolation, TextLiteral, TextSegment,
    TypeField, TypeId, TypeItem, TypeItemBody, TypeKind, TypeNode, TypeParameter, TypeParameterId,
    TypeReference, TypeResolution, TypeVariant, UnaryOperator, Unresolved, UseItem, ValueItem,
    WithControl,
};

pub struct LoweringResult<S = Resolved> {
    module: Module<S>,
    diagnostics: Vec<Diagnostic>,
}

impl<S> LoweringResult<S> {
    pub fn new(module: Module<S>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            module,
            diagnostics,
        }
    }

    pub fn module(&self) -> &Module<S> {
        &self.module
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn into_parts(self) -> (Module<S>, Vec<Diagnostic>) {
        (self.module, self.diagnostics)
    }
}

/// Lowers a syntax module to HIR, leaving all name references as
/// [`ResolutionState::Unresolved`]. Import bindings from `use` declarations are
/// resolved via `resolver` (needed to detect cycles and populate metadata for
/// imported bindings), but term/type/export references inside item bodies are
/// not resolved. Call [`resolve_imports`] on the result to fill those in.
pub fn lower_structure(
    module: &syn::Module,
    resolver: Option<&dyn crate::resolver::ImportResolver>,
) -> LoweringResult<Unresolved> {
    let null_resolver = crate::resolver::NullImportResolver;
    let mut lowerer = Lowerer::new(module.file, resolver.unwrap_or(&null_resolver));
    for item in &module.items {
        lowerer.lower_item(item);
    }
    lowerer.lower_ambient_prelude();
    LoweringResult::new(lowerer.module.into_unresolved(), lowerer.diagnostics)
}

/// Resolves all [`ResolutionState::Unresolved`] name references in a
/// structurally-lowered HIR module produced by [`lower_structure`].
///
/// This pass builds the module-level name namespaces, resolves every term,
/// type, and export reference, and validates cluster normalisation. It does
/// not call any external import resolver — import-binding resolution is
/// already complete after [`lower_structure`].
pub fn resolve_imports(module: Module<Unresolved>) -> LoweringResult {
    let null_resolver = crate::resolver::NullImportResolver;
    // The module is about to be resolved by this function; convert to the
    // resolved type so the Lowerer can work with it uniformly, then perform
    // the resolution pass which fills every Unresolved reference in place.
    let mut lowerer = Lowerer::from_module(module.mark_resolved(), &null_resolver);
    let namespaces = lowerer.build_namespaces();
    lowerer.resolve_module(&namespaces);
    lowerer.validate_cluster_normalization();
    LoweringResult::new(lowerer.module, lowerer.diagnostics)
}

pub fn lower_module(module: &syn::Module) -> LoweringResult {
    lower_module_with_resolver(module, None)
}

pub fn lower_module_with_resolver(
    module: &syn::Module,
    resolver: Option<&dyn crate::resolver::ImportResolver>,
) -> LoweringResult {
    let null_resolver = crate::resolver::NullImportResolver;
    let mut lowerer = Lowerer::new(module.file, resolver.unwrap_or(&null_resolver));
    for item in &module.items {
        lowerer.lower_item(item);
    }
    lowerer.lower_ambient_prelude();
    let namespaces = lowerer.build_namespaces();
    lowerer.resolve_module(&namespaces);
    lowerer.normalize_function_signature_annotations();
    lowerer.validate_cluster_normalization();
    crate::capability_handle_elaboration::elaborate_capability_handles(
        &mut lowerer.module,
        &mut lowerer.diagnostics,
    );
    crate::signal_metadata_elaboration::populate_signal_metadata(&mut lowerer.module);
    LoweringResult::new(lowerer.module, lowerer.diagnostics)
}

const AMBIENT_PRELUDE_SOURCE: &str = r#"type Ordering = Less | Equal | Greater

class Setoid A = {
    equals : A -> A -> Bool
}

class Semigroupoid C = {
    compose : C B C -> C A B -> C A C
}

class Semigroup A = {
    append : A -> A -> A
}

class Foldable F = {
    reduce : (B -> A -> B) -> B -> F A -> B
}

class Functor F = {
    map : (A -> B) -> F A -> F B
}

class Contravariant F = {
    contramap : (B -> A) -> F A -> F B
}

class Filterable F = {
    with Functor F
    filterMap : (A -> Option B) -> F A -> F B
}

class Eq A = {
    (==) : A -> A -> Bool
    (!=) : A -> A -> Bool
}

class Default A = {
    default : A
}

class Ord A = {
    with Eq A
    compare : A -> A -> Ordering
}

class Category C = {
    with Semigroupoid C
    id : C A A
}

class Monoid A = {
    with Semigroup A
    empty : A
}

class Traversable T = {
    with Functor T
    with Foldable T
    traverse : Applicative G => (A -> G B) -> T A -> G (T B)
}

class Profunctor P = {
    dimap : (A2 -> A1) -> (B1 -> B2) -> P A1 B1 -> P A2 B2
}

class Bifunctor F = {
    bimap : (A -> C) -> (B -> D) -> F A B -> F C D
}

class Group A = {
    with Monoid A
    invert : A -> A
}

class Alt F = {
    with Functor F
    alt : F A -> F A -> F A
}

class Apply F = {
    with Functor F
    apply : F (A -> B) -> F A -> F B
}

class Extend W = {
    with Functor W
    extend : (W A -> B) -> W A -> W B
}

class Plus F = {
    with Alt F
    zero : F A
}

class Applicative F = {
    with Apply F
    pure : A -> F A
}

class Chain M = {
    with Apply M
    chain : (A -> M B) -> M A -> M B
}

class Comonad W = {
    with Extend W
    extract : W A -> A
}

class Alternative F = {
    with Applicative F
    with Plus F
    guard : Bool -> F Unit
}

class Monad M = {
    with Applicative M
    with Chain M
    join : M (M A) -> M A
}

class ChainRec M = {
    with Monad M
    chainRec : (A -> M (Result A B)) -> A -> M B
}

type __AiviListTailState A = {
    seenFirst: Bool,
    items: List A
}

type A -> (Option A) -> A
func __aivi_option_getOrElse = fallback opt => opt
    ||> Some item -> item
    ||> None      -> fallback

type A -> (Option A)
func __aivi_list_keepSome = item =>
    Some item

type (Option A) -> A -> (Option A)
func __aivi_list_keepFirst = found item => found
    T|> __aivi_list_keepSome
    F|> Some item

type Int -> A -> Int
func __aivi_list_lengthStep = total item =>
    total + 1

type (List A) -> Int
func __aivi_list_length = items =>
    items
      |> reduce __aivi_list_lengthStep 0

type (List A) -> (Option A)
func __aivi_list_head = items =>
    items
      |> reduce __aivi_list_keepFirst None

type (List A) -> A -> Bool -> (__AiviListTailState A)
func __aivi_list_tailState = items item seenFirst => seenFirst
    T|> { seenFirst: True, items: append items [item] }
    F|> { seenFirst: True, items: [] }

type (__AiviListTailState A) -> A -> (__AiviListTailState A)
func __aivi_list_tailStep = state item => state
    ||> { seenFirst, items } -> __aivi_list_tailState items item seenFirst

type (List A) -> Bool -> (Option (List A))
func __aivi_list_tailItems = items seenFirst => seenFirst
    T|> Some items
    F|> None

type (__AiviListTailState A) -> (Option (List A))
func __aivi_list_tailFromState = state => state
    ||> { seenFirst, items } -> __aivi_list_tailItems items seenFirst

type (List A) -> (Option (List A))
func __aivi_list_tail = items =>
    items
      |> reduce __aivi_list_tailStep { seenFirst: False, items: [] }
      |> __aivi_list_tailFromState

type (A -> Bool) -> Bool -> A -> Bool
func __aivi_list_anyStep = predicate found item => found
    T|> True
    F|> predicate item

type (A -> Bool) -> (List A) -> Bool
func __aivi_list_any = predicate items =>
    items
      |> reduce (__aivi_list_anyStep predicate) False

type Eq A => A -> A -> Bool
func __aivi_binary_eq = left right =>
    left == right

type Eq A => A -> A -> Bool
func __aivi_binary_neq = left right =>
    left != right

domain NonEmptyList A over List A = {
    type (List A) -> (NonEmptyList A)
    lift items = items
}

type A -> (NonEmptyList A)
func __aivi_nel_singleton = item =>
    lift [item]

type A -> (NonEmptyList A) -> (NonEmptyList A)
func __aivi_nel_cons = item nel =>
    lift (append [item] nel.carrier)

type NonEmptyList A -> A
func __aivi_nel_head = nel =>
    nel.carrier
    ||> [h, ...ignored] -> h

type NonEmptyList A -> (List A)
func __aivi_nel_toList = nel =>
    nel.carrier

type A -> (List A) -> (NonEmptyList A)
func __aivi_nel_fromHeadTail = h t =>
    lift (append [h] t)

type NonEmptyList A -> Int
func __aivi_nel_length = nel =>
    __aivi_list_length nel.carrier

type A -> A -> A
func __aivi_nel_lastStep = prev item =>
    item

type A -> (List A) -> A
func __aivi_nel_lastOf = h t => t
    |> reduce __aivi_nel_lastStep h

type NonEmptyList A -> A
func __aivi_nel_last = nel =>
    nel.carrier
    ||> [h, ...t] -> __aivi_nel_lastOf h t

type (A -> B) -> (NonEmptyList A) -> (NonEmptyList B)
func __aivi_nel_mapNel = transform nel =>
    lift (__aivi_list_map transform nel.carrier)

type (NonEmptyList A) -> (NonEmptyList A) -> (NonEmptyList A)
func __aivi_nel_appendNel = left right =>
    lift (append left.carrier right.carrier)

type (List A) -> (Option A) -> (List A)
func __aivi_nel_initAppendPrev = items prev => prev
    ||> Some p -> append items [p]
    ||> None   -> items

type (List A) -> (Option A) -> A -> (List A, Option A)
func __aivi_nel_initAccum = items prev item =>
    (__aivi_nel_initAppendPrev items prev, Some item)

type (List A, Option A) -> A -> (List A, Option A)
func __aivi_nel_initStep = state item => state
    ||> (items, prev) -> __aivi_nel_initAccum items prev item

type (List A, Option A) -> (List A)
func __aivi_nel_initExtract = state => state
    ||> (items, prev) -> items

type NonEmptyList A -> (List A)
func __aivi_nel_init = nel =>
    nel.carrier
    |> reduce __aivi_nel_initStep ([], None)
    |> __aivi_nel_initExtract

type (Option (NonEmptyList A)) -> A -> (Option (NonEmptyList A))
func __aivi_nel_fromListStep = acc item => acc
    ||> None     -> Some (lift [item])
    ||> Some nel -> Some (lift (append nel.carrier [item]))

type (List A) -> (Option (NonEmptyList A))
func __aivi_nel_fromList = items => items
    |> reduce __aivi_nel_fromListStep None

type (A -> B) -> (Option A) -> (Option B)
func __aivi_option_map = transform opt => opt
    ||> Some item -> Some (transform item)
    ||> None      -> None

type (A -> Bool) -> Bool -> A -> Bool
func __aivi_list_containsStep = eq found item => found
    T|> True
    F|> eq item

type (A -> A -> Bool) -> A -> (List A) -> Bool
func __aivi_list_contains = eq target items => items
    |> reduce (__aivi_list_containsStep (eq target)) False

type (A -> B) -> (List B) -> A -> (List B)
func __aivi_list_mapStep = transform acc item =>
    append acc [transform item]

type (A -> B) -> (List A) -> (List B)
func __aivi_list_map = transform items => items
    |> reduce (__aivi_list_mapStep transform) []

type (A -> List B) -> (List B) -> A -> (List B)
func __aivi_list_flatMapStep = transform acc item =>
    append acc (transform item)

type (A -> List B) -> (List A) -> (List B)
func __aivi_list_flatMap = transform items => items
    |> reduce (__aivi_list_flatMapStep transform) []

type (A -> Bool) -> (List A) -> A -> (List A)
func __aivi_list_filterAppend = predicate acc item => predicate item
    T|> append acc [item]
    F|> acc

type (A -> Bool) -> (List A) -> (List A)
func __aivi_list_filter = predicate items => items
    |> reduce (__aivi_list_filterAppend predicate) []

type (A -> Bool) -> Int -> A -> Int
func __aivi_list_countStep = predicate acc item => predicate item
    T|> acc + 1
    F|> acc

type (A -> Bool) -> (List A) -> Int
func __aivi_list_count = predicate items => items
    |> reduce (__aivi_list_countStep predicate) 0

type Int -> Int -> Int
func __aivi_list_sumStep = acc item =>
    acc + item

type (List Int) -> Int
func __aivi_list_sum = items => items
    |> reduce __aivi_list_sumStep 0

type (A -> A -> Bool) -> A -> A -> (Option A)
func __aivi_list_maxPick = gt item prev => gt item prev
    T|> Some item
    F|> Some prev

type (A -> A -> Bool) -> (Option A) -> A -> (Option A)
func __aivi_list_maximumStep = gt best item => best
    ||> None     -> Some item
    ||> Some prev -> __aivi_list_maxPick gt item prev

type (A -> A -> Bool) -> (List A) -> (Option A)
func __aivi_list_maximum = gt items => items
    |> reduce (__aivi_list_maximumStep gt) None

type Int -> (List Int) -> (List Int)
func __aivi_list_rangeDesc = current acc => current < 0
    T|> acc
    F|> __aivi_list_rangeDesc (current - 1) (append [current] acc)

type Int -> (List Int)
func __aivi_list_range = n => n <= 0
    T|> []
    F|> __aivi_list_rangeDesc (n - 1) []

type Text -> Text -> Text -> (Bool, Text)
func __aivi_text_joinFirst = sep result item =>
    (False, item)

type Text -> Text -> Text -> (Bool, Text)
func __aivi_text_joinNext = sep result item =>
    (False, append (append result sep) item)

type Bool -> Text -> Text -> Text -> (Bool, Text)
func __aivi_text_joinPick = isFirst sep result item => isFirst
    T|> __aivi_text_joinFirst sep result item
    F|> __aivi_text_joinNext sep result item

type Text -> (Bool, Text) -> Text -> (Bool, Text)
func __aivi_text_joinStep = sep state item => state
    ||> (isFirst, result) -> __aivi_text_joinPick isFirst sep result item

type (Bool, Text) -> Text
func __aivi_text_joinExtract = state => state
    ||> (isFirst, result) -> result

type Text -> (List Text) -> Text
func __aivi_text_join = sep items => items
    |> reduce (__aivi_text_joinStep sep) (True, "")
    |> __aivi_text_joinExtract

type Matrix A =
  | MkMatrix Int Int (List (List A))

type MatrixError =
  | NegativeWidth Int
  | NegativeHeight Int
  | RaggedRows Int Int Int

type (Matrix A) -> (List (List A))
func __aivi_matrix_rows = matrix => matrix
    ||> MkMatrix w h data -> data

type (Matrix A) -> Int
func __aivi_matrix_width = matrix => matrix
    ||> MkMatrix w h data -> w

type (Matrix A) -> Int
func __aivi_matrix_height = matrix => matrix
    ||> MkMatrix w h data -> h

type Bool -> Int -> A -> (Int, Option A)
func __aivi_listAt_match = matches idx item => matches
    T|> (idx + 1, Some item)
    F|> (idx + 1, None)

type Int -> Int -> (Option A) -> A -> (Int, Option A)
func __aivi_listAt_check = target idx found item => found
    ||> Some already -> (idx + 1, Some already)
    ||> None -> __aivi_listAt_match (idx == target) idx item

type Int -> (Int, Option A) -> A -> (Int, Option A)
func __aivi_listAt_step = target state item => state
    ||> (idx, found) -> __aivi_listAt_check target idx found item

type (Int, Option A) -> (Option A)
func __aivi_listAt_extract = state => state
    ||> (idx, found) -> found

type Int -> (List A) -> (Option A)
func __aivi_listAt = target items => items
    |> reduce (__aivi_listAt_step target) (0, None)
    |> __aivi_listAt_extract

type (Option (List A)) -> Int -> (Option A)
func __aivi_matrix_atRow = rowOpt x => rowOpt
    ||> Some row -> __aivi_listAt x row
    ||> None     -> None

type (Matrix A) -> Int -> Int -> (Option A)
func __aivi_matrix_at = matrix x y => matrix
    ||> MkMatrix w h data ->
        __aivi_matrix_atRow (__aivi_listAt y data) x

type Bool -> A -> Int -> (List A) -> A -> (Int, List A)
func __aivi_listReplace_pick = matches newVal idx result item => matches
    T|> (idx + 1, append result [newVal])
    F|> (idx + 1, append result [item])

type Int -> A -> Int -> (List A) -> A -> (Int, List A)
func __aivi_listReplace_check = target newVal idx result item =>
    __aivi_listReplace_pick (idx == target) newVal idx result item

type Int -> A -> (Int, List A) -> A -> (Int, List A)
func __aivi_listReplace_step = target newVal state item => state
    ||> (idx, result) -> __aivi_listReplace_check target newVal idx result item

type (Int, List A) -> (List A)
func __aivi_listReplace_extract = state => state
    ||> (idx, result) -> result

type Int -> A -> (List A) -> (List A)
func __aivi_listReplace = target newVal items => items
    |> reduce (__aivi_listReplace_step target newVal) (0, [])
    |> __aivi_listReplace_extract

type Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_doReplace = x y w h data value =>
    __aivi_listAt y data
        ||> Some row -> Some (MkMatrix w h (__aivi_listReplace y (__aivi_listReplace x value row) data))
        ||> None     -> None

type Bool -> Bool -> Bool -> Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck = xOk yOk xLt yLt x y w h data value => xOk
    T|> __aivi_matrix_boundsCheck2 yOk xLt yLt x y w h data value
    F|> None

type Bool -> Bool -> Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck2 = yOk xLt yLt x y w h data value => yOk
    T|> __aivi_matrix_boundsCheck3 xLt yLt x y w h data value
    F|> None

type Bool -> Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck3 = xLt yLt x y w h data value => xLt
    T|> __aivi_matrix_boundsCheck4 yLt x y w h data value
    F|> None

type Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck4 = yLt x y w h data value => yLt
    T|> __aivi_matrix_doReplace x y w h data value
    F|> None

type (Matrix A) -> Int -> Int -> A -> (Option (Matrix A))
func __aivi_matrix_replaceCoord = matrix x y value => matrix
    ||> MkMatrix w h data -> __aivi_matrix_boundsCheck (x >= 0) (y >= 0) (x < w) (y < h) x y w h data value

type (Matrix A) -> (Int, Int) -> A -> (Option (Matrix A))
func __aivi_matrix_replaceAt = matrix coord value => coord
    ||> (x, y) -> __aivi_matrix_replaceCoord matrix x y value

type Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateFirstRow = rowIdx expectedWidth row =>
    (1, __aivi_list_length row, None)

type Bool -> Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateLengthMatch = matches rowIdx expectedWidth row => matches
    T|> (rowIdx + 1, expectedWidth, None)
    F|> (rowIdx + 1, expectedWidth, Some (RaggedRows rowIdx expectedWidth (__aivi_list_length row)))

type Bool -> Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateSubsequentRow = isFirst rowIdx expectedWidth row => isFirst
    T|> __aivi_matrix_validateFirstRow rowIdx expectedWidth row
    F|> __aivi_matrix_validateLengthMatch (__aivi_list_length row == expectedWidth) rowIdx expectedWidth row

type (Option MatrixError) -> Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateRow = prevError rowIdx expectedWidth row => prevError
    ||> Some e -> (rowIdx + 1, expectedWidth, Some e)
    ||> None -> __aivi_matrix_validateSubsequentRow (rowIdx == 0) rowIdx expectedWidth row

type (Int, Int, Option MatrixError) -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_fromRowsStep = state row => state
    ||> (rowIdx, width, error) -> __aivi_matrix_validateRow error rowIdx width row

type (Option MatrixError) -> Int -> Int -> (List (List A)) -> (Result MatrixError (Matrix A))
func __aivi_matrix_fromRowsDecide = error rowCount width inputRows => error
    ||> Some e -> Err e
    ||> None   -> Ok (MkMatrix width rowCount inputRows)

type (List (List A)) -> (Int, Int, Option MatrixError) -> (Result MatrixError (Matrix A))
func __aivi_matrix_fromRowsFinish = inputRows state => state
    ||> (rowCount, width, error) -> __aivi_matrix_fromRowsDecide error rowCount width inputRows

type (List (List A)) -> (Result MatrixError (Matrix A))
func __aivi_matrix_fromRows = inputRows => inputRows
    |> reduce __aivi_matrix_fromRowsStep (0, 0, None)
    |> __aivi_matrix_fromRowsFinish inputRows

type (A -> Bool) -> A -> (Option A)
func __aivi_list_findTry = predicate item => predicate item
    T|> Some item
    F|> None

type (A -> Bool) -> (Option A) -> A -> (Option A)
func __aivi_list_findStep = predicate acc item => acc
    ||> Some v -> Some v
    ||> None   -> __aivi_list_findTry predicate item

type (A -> Bool) -> (List A) -> (Option A)
func __aivi_list_find = predicate items => items
    |> reduce (__aivi_list_findStep predicate) None

type Int -> Int -> (List A) -> A -> (Int, List A)
func __aivi_list_takeHelp = n count acc item => count >= n
    T|> (count, acc)
    F|> (count + 1, append acc [item])

type Int -> (Int, List A) -> A -> (Int, List A)
func __aivi_list_takeStep = n state item => state
    ||> (count, acc) -> __aivi_list_takeHelp n count acc item

type (Int, List A) -> (List A)
func __aivi_list_takeExtract = state => state
    ||> (_, acc) -> acc

type Int -> (List A) -> (List A)
func __aivi_list_take = n items => items
    |> reduce (__aivi_list_takeStep n) (0, [])
    |> __aivi_list_takeExtract

type (A -> A -> Bool) -> A -> A -> (List A) -> (Bool, List A)
func __aivi_list_sortByInsertFalse = cmp newItem current acc => cmp newItem current
    T|> (True, append (append acc [newItem]) [current])
    F|> (False, append acc [current])

type (A -> A -> Bool) -> A -> (Bool, List A) -> A -> (Bool, List A)
func __aivi_list_sortByInsertStep = cmp newItem state current => state
    ||> (True, acc) -> (True, append acc [current])
    ||> (False, acc) -> __aivi_list_sortByInsertFalse cmp newItem current acc

type (A -> A -> Bool) -> A -> (Bool, List A) -> (List A)
func __aivi_list_sortByInsertFinish = cmp newItem state => state
    ||> (True, result) -> result
    ||> (False, acc) -> append acc [newItem]

type (A -> A -> Bool) -> A -> (List A) -> (List A)
func __aivi_list_sortByInsert = cmp newItem sorted => sorted
    |> reduce (__aivi_list_sortByInsertStep cmp newItem) (False, [])
    |> __aivi_list_sortByInsertFinish cmp newItem

type (A -> A -> Bool) -> (List A) -> A -> (List A)
func __aivi_list_sortByStep = cmp sorted item =>
    __aivi_list_sortByInsert cmp item sorted

type (A -> A -> Bool) -> (List A) -> (List A)
func __aivi_list_sortBy = cmp items => items
    |> reduce (__aivi_list_sortByStep cmp) []

type Text -> Bool
func __aivi_text_isEmpty = text => text == ""

type Text -> Bool
func __aivi_text_nonEmpty = text => text == ""
    T|> False
    F|> True

"#;

const MAX_COMPILE_TIME_RANGE_ELEMENTS: u64 = 4096;

struct Lowerer<'a> {
    module: Module,
    diagnostics: Vec<Diagnostic>,
    resolver: &'a dyn crate::resolver::ImportResolver,
}

#[derive(Clone, Copy)]
enum AmbientProjectionWork {
    Expr {
        expr: ExprId,
        ambient_allowed: bool,
    },
    Markup {
        node: MarkupNodeId,
        ambient_allowed: bool,
    },
    Control {
        node: ControlNodeId,
        ambient_allowed: bool,
    },
}

#[derive(Clone, Copy)]
struct NamedSite<T> {
    value: T,
    span: SourceSpan,
}

#[derive(Default)]
struct Namespaces {
    term_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    ambient_term_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    domain_terms: HashMap<String, Vec<NamedSite<DomainMemberResolution>>>,
    class_terms: HashMap<String, Vec<NamedSite<crate::hir::ClassMemberResolution>>>,
    ambient_class_terms: HashMap<String, Vec<NamedSite<crate::hir::ClassMemberResolution>>>,
    type_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    ambient_type_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    any_items: HashMap<String, Vec<NamedSite<ItemId>>>,
    provider_contracts: HashMap<String, Vec<NamedSite<ItemId>>>,
    literal_suffixes: HashMap<String, Vec<NamedSite<LiteralSuffixResolution>>>,
    term_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    type_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    /// Names made available project-wide by `hoist` declarations.  Consulted
    /// after explicit `use` imports but before class/builtin fallbacks.
    hoisted_term_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    hoisted_type_imports: HashMap<String, Vec<NamedSite<ImportId>>>,
    /// Module paths (dot-joined) that have already been registered via a local
    /// `hoist` declaration.  Prevents double-registration when the workspace
    /// scan returns the same module path.
    hoisted_module_paths: std::collections::HashSet<String>,
}

#[derive(Clone, Copy)]
enum LookupResult<T> {
    Unique(T),
    Ambiguous,
    Missing,
}

#[derive(Clone, Default)]
struct ResolveEnv {
    term_scopes: Vec<HashMap<String, BindingId>>,
    type_scopes: Vec<HashMap<String, TypeParameterId>>,
    implicit_type_parameters: Vec<TypeParameterId>,
    allow_implicit_type_parameters: bool,
    prefer_ambient_names: bool,
}

#[derive(Clone, Copy)]
enum MarkupPlacement {
    Renderable,
    EachEmpty,
    MatchCase,
}

enum LoweredMarkup {
    Renderable(MarkupNodeId),
    Empty(ControlNodeId),
    Case(ControlNodeId),
}

impl<'a> Lowerer<'a> {
    fn new(file: aivi_base::FileId, resolver: &'a dyn crate::resolver::ImportResolver) -> Self {
        Self {
            module: Module::new(file),
            diagnostics: Vec::new(),
            resolver,
        }
    }

    fn from_module(module: Module, resolver: &'a dyn crate::resolver::ImportResolver) -> Self {
        Self {
            module,
            diagnostics: Vec::new(),
            resolver,
        }
    }

    fn lower_ambient_prelude(&mut self) {
        let source = aivi_base::SourceFile::new(
            self.module.file(),
            "<aivi.prelude>",
            AMBIENT_PRELUDE_SOURCE,
        );
        let parsed = syn::parse_module(&source);
        debug_assert!(
            !parsed.has_errors(),
            "ambient prelude must parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        for item in &parsed.module.items {
            self.lower_ambient_item(item);
        }
    }

    fn lower_item(&mut self, item: &syn::Item) {
        self.lower_item_with_storage(item, false);
    }

    fn lower_ambient_item(&mut self, item: &syn::Item) {
        self.lower_item_with_storage(item, true);
    }

    fn lower_item_with_storage(&mut self, item: &syn::Item, ambient: bool) {
        if let syn::Item::Export(item) = item {
            for export in self.lower_export_items(item) {
                self.store_item(Item::Export(export), ambient);
            }
            return;
        }

        let lowered = match item {
            syn::Item::Type(item) => Some(Item::Type(self.lower_type_item(item))),
            syn::Item::Fun(item) => Some(Item::Function(self.lower_function_item(item))),
            syn::Item::Value(item) => Some(Item::Value(self.lower_value_item(item))),
            syn::Item::Signal(item) => Some(Item::Signal(self.lower_signal_item(item))),
            syn::Item::Class(item) => Some(Item::Class(self.lower_class_item(item))),
            syn::Item::Instance(item) => Some(Item::Instance(self.lower_instance_item(item))),
            syn::Item::Domain(item) => Some(Item::Domain(self.lower_domain_item(item))),
            syn::Item::SourceProviderContract(item) => Some(Item::SourceProviderContract(
                self.lower_source_provider_contract_item(item),
            )),
            syn::Item::Use(item) => Some(Item::Use(self.lower_use_item(item))),
            syn::Item::Hoist(item) => Some(Item::Hoist(self.lower_hoist_item(item))),
            syn::Item::Export(_) => {
                unreachable!("export items are handled before single-item lowering")
            }
            syn::Item::Error(item) => {
                self.emit_error(
                    item.base.span,
                    "error recovery item cannot enter Milestone 2 HIR",
                    code("error-item"),
                );
                None
            }
        };

        if let Some(item) = lowered {
            self.store_item(item, ambient);
        }
    }

    fn store_item(&mut self, item: Item, ambient: bool) {
        if ambient {
            if self.module.push_ambient_item(item).is_err() {
                self.emit_arena_overflow("HIR ambient item arena");
            }
        } else if self.module.push_item(item).is_err() {
            self.emit_arena_overflow("HIR item arena");
        }
    }

    fn lower_type_item(&mut self, item: &syn::NamedItem) -> TypeItem {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Type, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "type declaration");
        let parameters = self.lower_type_parameters(&item.type_parameters);
        let body = match item.type_body() {
            Some(syn::TypeDeclBody::Alias(expr)) => TypeItemBody::Alias(self.lower_type_expr(expr)),
            Some(syn::TypeDeclBody::Sum(variants)) => {
                let variants = variants
                    .iter()
                    .map(|variant| TypeVariant {
                        span: variant.span,
                        name: self.required_name(
                            variant.name.as_ref(),
                            variant.span,
                            "type variant",
                        ),
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| crate::hir::TypeVariantField {
                                label: field.label.as_ref().map(|l| l.text.as_str().into()),
                                ty: self.lower_type_expr(&field.ty),
                            })
                            .collect(),
                    })
                    .collect::<Vec<_>>();
                match crate::NonEmpty::from_vec(variants) {
                    Ok(variants) => TypeItemBody::Sum(variants),
                    Err(_) => {
                        self.emit_error(
                            item.base.span,
                            "sum type must contain at least one constructor",
                            code("empty-sum-type"),
                        );
                        TypeItemBody::Alias(self.placeholder_type(item.base.span))
                    }
                }
            }
            None => {
                self.emit_error(
                    item.base.span,
                    "type declaration is missing a body",
                    code("missing-type-body"),
                );
                TypeItemBody::Alias(self.placeholder_type(item.base.span))
            }
        };

        TypeItem {
            header,
            name,
            parameters,
            body,
        }
    }

    fn lower_value_item(&mut self, item: &syn::NamedItem) -> ValueItem {
        if !item.type_parameters.is_empty() {
            let type_param_span = item
                .type_parameters
                .first()
                .unwrap()
                .span
                .join(item.type_parameters.last().unwrap().span)
                .unwrap_or(item.base.span);
            self.emit_warning(
                type_param_span,
                "generic value declarations are not yet supported and will be ignored",
                code("unsupported-generic-value"),
            );
        }
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Value, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "value declaration");
        let annotation = item
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation));
        let body = item
            .expr_body()
            .map(|expr| self.lower_expr(expr))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "value declaration is missing a body",
                    code("missing-value-body"),
                );
                self.placeholder_expr(item.base.span)
            });

        ValueItem {
            header,
            name,
            annotation,
            body,
        }
    }

    fn lower_function_item(&mut self, item: &syn::NamedItem) -> FunctionItem {
        if !item.type_parameters.is_empty() {
            self.emit_warning(
                item.base.span,
                "generic function type parameters are not yet supported and will be ignored",
                code("unsupported-generic-function"),
            );
        }
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Function, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "function declaration");
        let parameters = item
            .parameters
            .iter()
            .map(|parameter| self.lower_function_parameter(parameter))
            .collect();
        let context = item
            .constraints
            .iter()
            .map(|constraint| self.lower_type_expr(constraint))
            .collect();
        let annotation = item
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation));
        let body = item
            .expr_body()
            .map(|expr| self.lower_expr(expr))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "function declaration is missing a body",
                    code("missing-function-body"),
                );
                self.placeholder_expr(item.base.span)
            });

        FunctionItem {
            header,
            name,
            type_parameters: Vec::new(),
            context,
            parameters,
            annotation,
            body,
        }
    }

    fn lower_signal_item(&mut self, item: &syn::NamedItem) -> SignalItem {
        if !item.type_parameters.is_empty() {
            let type_param_span = item
                .type_parameters
                .first()
                .unwrap()
                .span
                .join(item.type_parameters.last().unwrap().span)
                .unwrap_or(item.base.span);
            self.emit_warning(
                type_param_span,
                "generic signal declarations are not yet supported and will be ignored",
                code("unsupported-generic-signal"),
            );
        }
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Signal, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "signal declaration");
        let annotation = item
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation));

        let (body, reactive_updates) = if let Some(merge) = item.merge_body() {
            let (seed, updates) = self.lower_signal_merge_arms(merge, item.keyword_span);
            (seed, updates)
        } else {
            let body = item.expr_body().map(|expr| self.lower_expr(expr));
            (body, Vec::new())
        };

        SignalItem {
            header,
            name,
            annotation,
            body,
            reactive_updates,
            signal_dependencies: Vec::new(),
            import_signal_dependencies: Vec::new(),
            source_metadata: None,
            is_source_capability_handle: false,
        }
    }

    fn lower_signal_merge_arms(
        &mut self,
        merge: &syn::SignalMergeBody,
        _keyword_span: SourceSpan,
    ) -> (Option<ExprId>, Vec<ReactiveUpdateClause>) {
        let mut updates = Vec::new();
        let mut seed_body: Option<ExprId> = None;

        // Resolve source signals.
        let source_items: Vec<Option<ItemId>> = merge
            .sources
            .iter()
            .map(|source| self.resolve_merge_source(source))
            .collect();

        for arm in &merge.arms {
            let Some(pattern) = arm.pattern.as_ref() else {
                self.emit_error(
                    arm.span,
                    "signal reactive arm is missing its pattern",
                    code("merge-arm-missing-pattern"),
                );
                continue;
            };
            let Some(body) = arm.body.as_ref() else {
                self.emit_error(
                    arm.span,
                    "signal reactive arm is missing its body expression",
                    code("merge-arm-missing-body"),
                );
                continue;
            };

            // Determine if this is a default/wildcard arm.
            let is_default_arm = arm.source.is_none()
                && matches!(
                    pattern.kind,
                    syn::PatternKind::Wildcard
                );

            // Determine trigger source from the arm's source prefix.
            let trigger_source = if is_default_arm {
                // Default arm (wildcard): no specific trigger source.
                None
            } else if let Some(source_ident) = &arm.source {
                // Multi-source arm: find the source in the merge list.
                let pos = merge.sources.iter().position(|s| s.text == source_ident.text);
                match pos {
                    Some(idx) => source_items[idx],
                    None => {
                        self.emit_error(
                            source_ident.span,
                            format!(
                                "signal name `{}` does not appear in the merge source list",
                                source_ident.text
                            ),
                            code("merge-arm-unknown-source"),
                        );
                        None
                    }
                }
            } else if merge.sources.len() == 1 {
                // Single-source arm: the only source is the trigger.
                source_items[0]
            } else {
                None
            };

            // Build the source expression for pattern matching.
            let source_expr_ident = if is_default_arm {
                None
            } else {
                arm.source
                    .as_ref()
                    .or_else(|| {
                        if merge.sources.len() == 1 {
                            Some(&merge.sources[0])
                        } else {
                            None
                        }
                    })
            };

            if let Some(source_ident) = source_expr_ident {
                let source_expr =
                    self.lower_unresolved_name_expr(source_ident.text.as_str(), source_ident.span);
                let guard =
                    self.make_pattern_match_bool_expr(source_expr, pattern, arm.span);
                let body_expr =
                    self.make_pattern_match_optional_expr(source_expr, pattern, body, arm.span);
                updates.push(ReactiveUpdateClause {
                    span: arm.span,
                    keyword_span: arm.span,
                    target_span: arm.span,
                    guard,
                    body: body_expr,
                    body_mode: ReactiveUpdateBodyMode::OptionalPayload,
                    trigger_source,
                });
            } else {
                // Default arm: becomes the signal's seed body (initial value).
                let body_expr = self.lower_expr(body);
                seed_body = Some(body_expr);
            }
        }

        (seed_body, updates)
    }

    fn resolve_merge_source(&mut self, source: &syn::Identifier) -> Option<ItemId> {
        let Some(source_item) = self.find_predeclared_named_item(source.text.as_str()) else {
            self.emit_error(
                source.span,
                format!(
                    "merge source `{}` must name a previously declared signal",
                    source.text
                ),
                code("merge-unknown-source"),
            );
            return None;
        };
        if !matches!(self.module.items()[source_item], Item::Signal(_)) {
            self.emit_error(
                source.span,
                format!(
                    "merge source `{}` must refer to a signal, not another kind of declaration",
                    source.text
                ),
                code("merge-source-not-signal"),
            );
            return None;
        }
        Some(source_item)
    }

    fn make_pattern_match_bool_expr(
        &mut self,
        subject: ExprId,
        pattern: &syn::Pattern,
        span: SourceSpan,
    ) -> ExprId {
        let on_match = self.lower_unresolved_name_expr("True", span);
        let on_fallback = self.lower_unresolved_name_expr("False", span);
        self.make_pattern_match_pipe_expr(subject, pattern, on_match, on_fallback, span)
    }

    fn make_pattern_match_optional_expr(
        &mut self,
        subject: ExprId,
        pattern: &syn::Pattern,
        body: &syn::Expr,
        span: SourceSpan,
    ) -> ExprId {
        let matched_body = self.lower_expr(body);
        let on_match = self.lower_constructor_apply_expr("Some", span, vec![matched_body]);
        let on_fallback = self.lower_unresolved_name_expr("None", span);
        self.make_pattern_match_pipe_expr(subject, pattern, on_match, on_fallback, span)
    }

    fn make_pattern_match_pipe_expr(
        &mut self,
        subject: ExprId,
        pattern: &syn::Pattern,
        on_match: ExprId,
        on_fallback: ExprId,
        span: SourceSpan,
    ) -> ExprId {
        let match_stage = PipeStage {
            span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: self.lower_pattern(pattern),
                body: on_match,
            },
        };
        let fallback_stage = PipeStage {
            span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: self.alloc_pattern(Pattern {
                    span,
                    kind: PatternKind::Wildcard,
                }),
                body: on_fallback,
            },
        };
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Pipe(PipeExpr {
                head: subject,
                stages: NonEmpty::new(match_stage, vec![fallback_stage]),
                result_block_desugaring: false,
            }),
        })
    }

    fn find_predeclared_named_item(&self, name: &str) -> Option<ItemId> {
        self.module
            .root_items()
            .iter()
            .rev()
            .copied()
            .find(|item_id| match &self.module.items()[*item_id] {
                Item::Type(item) => item.name.text() == name,
                Item::Value(item) => item.name.text() == name,
                Item::Function(item) => item.name.text() == name,
                Item::Signal(item) => item.name.text() == name,
                Item::Class(item) => item.name.text() == name,
                Item::Domain(item) => item.name.text() == name,
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
            | Item::Hoist(_) => false,
            })
    }

    fn lower_class_item(&mut self, item: &syn::NamedItem) -> ClassItem {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Class, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "class declaration");
        let mut parameters = self.lower_type_parameters(&item.type_parameters);
        if parameters.is_empty() {
            self.emit_error(
                item.base.span,
                "class declarations require at least one type parameter",
                code("missing-class-parameter"),
            );
            parameters.push(self.alloc_type_parameter(TypeParameter {
                span: item.base.span,
                name: self.make_name("A", item.base.span),
            }));
        }
        let parameters = crate::NonEmpty::from_vec(parameters)
            .expect("class fallback parameter list should be non-empty");
        let (superclasses, param_constraints, members) = item
            .class_body()
            .map(|body| {
                let superclasses: Vec<TypeId> = body
                    .with_decls
                    .iter()
                    .map(|w| self.lower_type_expr(&w.superclass))
                    .collect();
                let param_constraints: Vec<TypeId> = body
                    .require_decls
                    .iter()
                    .map(|r| self.lower_type_expr(&r.constraint))
                    .collect();
                let members = body
                    .members
                    .iter()
                    .map(|member| ClassMember {
                        span: member.span,
                        name: self.make_name(member.name.text(), member.name.span()),
                        type_parameters: Vec::new(),
                        context: member
                            .constraints
                            .iter()
                            .map(|constraint| self.lower_type_expr(constraint))
                            .collect(),
                        annotation: member
                            .annotation
                            .as_ref()
                            .map(|annotation| self.lower_type_expr(annotation))
                            .unwrap_or_else(|| {
                                self.emit_error(
                                    member.span,
                                    "class member is missing a type annotation",
                                    code("missing-class-member-type"),
                                );
                                self.placeholder_type(member.span)
                            }),
                    })
                    .collect();
                (superclasses, param_constraints, members)
            })
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "class declaration is missing a body",
                    code("missing-class-body"),
                );
                (Vec::new(), Vec::new(), Vec::new())
            });

        ClassItem {
            header,
            name,
            parameters,
            superclasses,
            param_constraints,
            members,
        }
    }

    fn lower_instance_item(&mut self, item: &syn::InstanceItem) -> InstanceItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Instance, item.base.span);
        let class = item
            .class
            .as_ref()
            .map(|class| TypeReference::unresolved(self.lower_qualified_name(class)))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "instance declaration is missing its class name",
                    code("missing-instance-class"),
                );
                TypeReference::unresolved(
                    self.make_path(&[self.make_name("invalid", item.base.span)]),
                )
            });
        let argument = item
            .target
            .as_ref()
            .map(|target| self.lower_type_expr(target))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "instance declaration is missing its target type",
                    code("missing-instance-target"),
                );
                self.placeholder_type(item.base.span)
            });
        let members = item
            .body
            .as_ref()
            .map(|body| {
                body.members
                    .iter()
                    .map(|member| InstanceMember {
                        span: member.span,
                        name: self.make_name(member.name.text(), member.name.span()),
                        parameters: member
                            .parameters
                            .iter()
                            .map(|parameter| self.lower_instance_parameter(parameter))
                            .collect(),
                        annotation: None,
                        body: member
                            .body
                            .as_ref()
                            .map(|body| self.lower_expr(body))
                            .unwrap_or_else(|| {
                                self.emit_error(
                                    member.span,
                                    "instance member is missing a body",
                                    code("missing-instance-member-body"),
                                );
                                self.placeholder_expr(member.span)
                            }),
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "instance declaration is missing a body",
                    code("missing-instance-body"),
                );
                Vec::new()
            });

        InstanceItem {
            header,
            class,
            arguments: crate::NonEmpty::new(argument, Vec::new()),
            type_parameters: Vec::new(),
            context: item
                .context
                .iter()
                .map(|constraint| self.lower_type_expr(constraint))
                .collect(),
            members,
        }
    }

    fn lower_domain_item(&mut self, item: &syn::DomainItem) -> DomainItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Domain, item.base.span);
        let name = self.required_name(item.name.as_ref(), item.base.span, "domain declaration");
        let parameters = self.lower_type_parameters(&item.type_parameters);
        let carrier = item
            .carrier
            .as_ref()
            .map(|carrier| self.lower_type_expr(carrier))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "domain declaration is missing a carrier type after `over`",
                    code("missing-domain-carrier"),
                );
                self.placeholder_type(item.base.span)
            });

        let mut members = Vec::new();
        if let Some(body) = &item.body {
            let mut seen_keys = HashMap::<String, SourceSpan>::new();
            for member in &body.members {
                let key = domain_member_surface_key(&member.name);
                if let Some(previous_span) = seen_keys.get(&key) {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "duplicate domain member `{}`",
                            domain_member_surface_name(&member.name)
                        ))
                        .with_code(code("duplicate-domain-member"))
                        .with_primary_label(
                            member.span,
                            "this domain member reuses an existing member name",
                        )
                        .with_secondary_label(*previous_span, "previous domain member here"),
                    );
                } else {
                    seen_keys.insert(key, member.span);
                }
                members.push(self.lower_domain_member(
                    member,
                    item.name.as_ref(),
                    &item.type_parameters,
                ));
            }
        }

        DomainItem {
            header,
            name,
            parameters,
            carrier,
            members,
        }
    }

    fn lower_domain_member(
        &mut self,
        member: &syn::DomainMember,
        domain_name: Option<&syn::Identifier>,
        domain_type_parameters: &[syn::Identifier],
    ) -> DomainMember {
        let (kind, name) = match &member.name {
            syn::DomainMemberName::Signature(signature) => match signature {
                syn::ClassMemberName::Identifier(identifier) => (
                    DomainMemberKind::Method,
                    self.make_name(&identifier.text, identifier.span),
                ),
                syn::ClassMemberName::Operator(operator) => (
                    DomainMemberKind::Operator,
                    self.make_name(&operator.text, operator.span),
                ),
            },
            syn::DomainMemberName::Literal(identifier) => (
                DomainMemberKind::Literal,
                self.make_name(&identifier.text, identifier.span),
            ),
        };

        let uses_self = member
            .body
            .as_ref()
            .is_some_and(|body| body.contains_self_reference());

        let annotation = member
            .annotation
            .as_ref()
            .map(|annotation| self.lower_type_expr(annotation))
            .unwrap_or_else(|| {
                self.emit_error(
                    member.span,
                    format!(
                        "domain member `{}` is missing a type annotation",
                        domain_member_surface_name(&member.name)
                    ),
                    code("missing-domain-member-type"),
                );
                self.placeholder_type(member.span)
            });

        // When `self` is used, prepend `DomainType ->` to the annotation.
        let annotation = if uses_self {
            if let Some(domain_id) = domain_name {
                let domain_type_id =
                    self.synthesise_domain_self_type(domain_id, domain_type_parameters);
                self.alloc_type(TypeNode {
                    span: member.span,
                    kind: TypeKind::Arrow {
                        parameter: domain_type_id,
                        result: annotation,
                    },
                })
            } else {
                annotation
            }
        } else {
            annotation
        };

        // Synthesise implicit `self` binding before explicit parameters.
        let mut parameters: Vec<FunctionParameter> = if uses_self {
            let self_name = self.make_name("self", member.span);
            let self_binding = self.alloc_binding(Binding {
                span: member.span,
                name: self_name,
                kind: BindingKind::FunctionParameter,
            });
            vec![FunctionParameter {
                span: member.span,
                binding: self_binding,
                annotation: None,
            }]
        } else {
            Vec::new()
        };

        parameters.extend(
            member
                .parameters
                .iter()
                .map(|parameter| self.lower_instance_parameter(parameter)),
        );

        let body = member.body.as_ref().map(|body| self.lower_expr(body));

        DomainMember {
            span: member.span,
            kind,
            name,
            annotation,
            parameters,
            body,
        }
    }

    /// Construct an unresolved HIR type for the domain itself, applying type
    /// parameters when the domain is generic (e.g. `NonEmpty A`).
    fn synthesise_domain_self_type(
        &mut self,
        domain_name: &syn::Identifier,
        type_parameters: &[syn::Identifier],
    ) -> TypeId {
        let name_type = self.alloc_type(TypeNode {
            span: domain_name.span,
            kind: TypeKind::Name(TypeReference::unresolved(
                self.make_path(&[self.make_name(&domain_name.text, domain_name.span)]),
            )),
        });
        if type_parameters.is_empty() {
            return name_type;
        }
        let arguments: Vec<TypeId> = type_parameters
            .iter()
            .map(|param| {
                self.alloc_type(TypeNode {
                    span: param.span,
                    kind: TypeKind::Name(TypeReference::unresolved(
                        self.make_path(&[self.make_name(&param.text, param.span)]),
                    )),
                })
            })
            .collect();
        let arguments = NonEmpty::from_vec(arguments).expect("non-empty type parameter list");
        self.alloc_type(TypeNode {
            span: domain_name.span,
            kind: TypeKind::Apply {
                callee: name_type,
                arguments,
            },
        })
    }

    fn lower_source_provider_contract_item(
        &mut self,
        item: &syn::SourceProviderContractItem,
    ) -> SourceProviderContractItem {
        let header = self.lower_item_header(
            &item.base.decorators,
            ItemKind::SourceProviderContract,
            item.base.span,
        );
        let provider_path = item
            .provider
            .as_ref()
            .map(|provider| self.lower_qualified_name(provider));
        let provider = SourceProviderRef::from_path(provider_path.as_ref());
        match &provider {
            SourceProviderRef::Builtin(provider_ref) => {
                self.emit_error(
                    item.base.span,
                    format!(
                        "provider contract declarations cannot target built-in source provider `{}`",
                        provider_ref.key()
                    ),
                    code("builtin-source-provider-contract"),
                );
            }
            SourceProviderRef::InvalidShape(key) => {
                self.emit_error(
                    item.base.span,
                    format!(
                        "provider contract `{key}` must use a qualified provider key such as `custom.feed`"
                    ),
                    code("invalid-source-provider-contract-shape"),
                );
            }
            SourceProviderRef::Missing | SourceProviderRef::Custom(_) => {}
        }

        SourceProviderContractItem {
            header,
            provider,
            contract: self.lower_custom_source_contract(item.body.as_ref()),
        }
    }

    fn lower_custom_source_contract(
        &mut self,
        body: Option<&syn::SourceProviderContractBody>,
    ) -> crate::CustomSourceContractMetadata {
        let mut contract = crate::CustomSourceContractMetadata::default();
        let mut wakeup_span = None;
        let mut argument_spans = HashMap::new();
        let mut option_spans = HashMap::new();
        let mut capability_member_spans = HashMap::new();
        let Some(body) = body else {
            return contract;
        };

        for member in &body.members {
            match member {
                syn::SourceProviderContractMember::FieldValue(member) => {
                    let Some(name) = member.name.as_ref() else {
                        continue;
                    };
                    match name.text.as_str() {
                        "wakeup" => {
                            if let Some(previous_span) = wakeup_span {
                                self.diagnostics.push(
                                    Diagnostic::error(
                                        "provider contract field `wakeup` is duplicated",
                                    )
                                    .with_code(code("duplicate-source-provider-contract-field"))
                                    .with_primary_label(
                                        member.span,
                                        "this `wakeup` field repeats an earlier contract field",
                                    )
                                    .with_secondary_label(
                                        previous_span,
                                        "previous `wakeup` field declared here",
                                    ),
                                );
                                continue;
                            }
                            wakeup_span = Some(member.span);
                            let Some(value) = member.value.as_ref() else {
                                continue;
                            };
                            contract.recurrence_wakeup =
                                self.lower_custom_source_contract_wakeup(value);
                        }
                        _ => {
                            self.emit_error(
                                name.span,
                                format!("unknown provider contract field `{}`", name.text),
                                code("unknown-source-provider-contract-field"),
                            );
                        }
                    }
                }
                syn::SourceProviderContractMember::ArgumentSchema(member) => {
                    let Some(name) = member.name.as_ref() else {
                        continue;
                    };
                    if let Some(previous_span) =
                        argument_spans.insert(name.text.clone(), member.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "provider contract argument `{}` is duplicated",
                                name.text
                            ))
                            .with_code(code("duplicate-source-provider-contract-field"))
                            .with_primary_label(
                                member.span,
                                "this argument schema repeats an earlier declaration",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous argument schema declared here",
                            ),
                        );
                        continue;
                    }
                    let Some(annotation) = member.annotation.as_ref() else {
                        continue;
                    };
                    contract.arguments.push(crate::CustomSourceArgumentSchema {
                        span: member.span,
                        name: self.make_name(&name.text, name.span),
                        annotation: self.lower_type_expr(annotation),
                    });
                }
                syn::SourceProviderContractMember::OptionSchema(member) => {
                    let Some(name) = member.name.as_ref() else {
                        continue;
                    };
                    if let Some(previous_span) = option_spans.insert(name.text.clone(), member.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "provider contract option `{}` is duplicated",
                                name.text
                            ))
                            .with_code(code("duplicate-source-provider-contract-field"))
                            .with_primary_label(
                                member.span,
                                "this option schema repeats an earlier declaration",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous option schema declared here",
                            ),
                        );
                        continue;
                    }
                    let Some(annotation) = member.annotation.as_ref() else {
                        continue;
                    };
                    contract.options.push(crate::CustomSourceOptionSchema {
                        span: member.span,
                        name: self.make_name(&name.text, name.span),
                        annotation: self.lower_type_expr(annotation),
                    });
                }
                syn::SourceProviderContractMember::OperationSchema(member) => {
                    self.lower_custom_source_contract_capability_member(
                        member,
                        "operation",
                        &mut contract.operations,
                        &mut capability_member_spans,
                    );
                }
                syn::SourceProviderContractMember::CommandSchema(member) => {
                    self.lower_custom_source_contract_capability_member(
                        member,
                        "command",
                        &mut contract.commands,
                        &mut capability_member_spans,
                    );
                }
            }
        }

        contract
    }

    fn lower_custom_source_contract_capability_member(
        &mut self,
        member: &syn::SourceProviderContractSchemaMember,
        kind: &'static str,
        members: &mut Vec<crate::CustomSourceCapabilityMember>,
        seen: &mut HashMap<String, (SourceSpan, &'static str)>,
    ) {
        let Some(name) = member.name.as_ref() else {
            return;
        };
        if let Some((previous_span, previous_kind)) =
            seen.insert(name.text.clone(), (member.span, kind))
        {
            let detail = if previous_kind == kind {
                format!("this {kind} repeats an earlier declaration")
            } else {
                format!("this {kind} conflicts with an earlier {previous_kind} declaration")
            };
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "provider contract capability member `{}` is duplicated",
                    name.text
                ))
                .with_code(code("duplicate-source-provider-contract-field"))
                .with_primary_label(member.span, detail)
                .with_secondary_label(
                    previous_span,
                    format!("previous {previous_kind} declared here"),
                ),
            );
            return;
        }
        let Some(annotation) = member.annotation.as_ref() else {
            return;
        };
        members.push(crate::CustomSourceCapabilityMember {
            span: member.span,
            name: self.make_name(&name.text, name.span),
            annotation: self.lower_type_expr(annotation),
        });
    }

    fn lower_custom_source_contract_wakeup(
        &mut self,
        value: &syn::Identifier,
    ) -> Option<crate::CustomSourceRecurrenceWakeup> {
        match value.text.as_str() {
            "timer" => Some(crate::CustomSourceRecurrenceWakeup::Timer),
            "backoff" => Some(crate::CustomSourceRecurrenceWakeup::Backoff),
            "sourceEvent" => Some(crate::CustomSourceRecurrenceWakeup::SourceEvent),
            "providerTrigger" => Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
            _ => {
                self.emit_error(
                    value.span,
                    format!("unknown provider contract wakeup `{}`", value.text),
                    code("unknown-source-provider-contract-wakeup"),
                );
                None
            }
        }
    }

    fn lower_use_item(&mut self, item: &syn::UseItem) -> UseItem {
        let header = self.lower_item_header(&item.base.decorators, ItemKind::Use, item.base.span);
        let module = item
            .path
            .as_ref()
            .map(|path| self.lower_qualified_name(path))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "use declaration is missing a module path",
                    code("missing-use-path"),
                );
                self.make_path(&[self.make_name("invalid", item.base.span)])
            });
        let module_name = path_text(&module);
        let module_segments = module.segments().iter().map(Name::text).collect::<Vec<_>>();
        let module_resolution = self.resolver.resolve(&module_segments);
        if let ImportModuleResolution::Cycle(cycle) = &module_resolution {
            // A module may import its own intrinsics by name (e.g. `aivi.core.bytes`
            // importing `length` from `aivi.core.bytes`).  Detect this as a direct
            // self-import where every requested name is a known intrinsic and suppress
            // the cycle error in that case.
            let is_direct_self_import = cycle.modules().iter().all(|m| m.as_ref() == module_name);
            let all_intrinsics = item.imports.iter().all(|import| {
                import
                    .path
                    .segments
                    .last()
                    .map(|s| known_import_metadata(&module_name, &s.text).is_some())
                    .unwrap_or(false)
            });
            if !(is_direct_self_import && all_intrinsics) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "import cycle detected: {}",
                        cycle
                            .modules()
                            .iter()
                            .map(|module| module.as_ref())
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    ))
                    .with_code(code("import-cycle"))
                    .with_primary_label(
                        item.base.span,
                        "this `use` item closes a cycle in the workspace import graph",
                    ),
                );
            }
        }
        let mut imports = item
            .imports
            .iter()
            .map(|import| {
                let imported_name = import
                    .path
                    .segments
                    .last()
                    .map(|segment| self.make_name(&segment.text, segment.span))
                    .unwrap_or_else(|| self.make_name("invalid", item.base.span));
                let local_name = import
                    .alias
                    .as_ref()
                    .map(|alias| self.make_name(&alias.text, alias.span))
                    .unwrap_or_else(|| imported_name.clone());
                let (resolution, metadata, callable_type, deprecation) =
                    self.resolve_import_binding(&module_name, &imported_name, &module_resolution);
                self.alloc_import(ImportBinding {
                    span: import.path.span,
                    imported_name: imported_name.clone(),
                    local_name,
                    resolution,
                    metadata,
                    callable_type,
                    deprecation,
                })
            })
            .collect::<Vec<_>>();
        // Auto-register imported instance member bindings so cross-module instance
        // resolution can find them without the user explicitly importing by name.
        if let ImportModuleResolution::Resolved(exports) = &module_resolution {
            for instance_decl in &exports.instances {
                for member in &instance_decl.members {
                    let synthetic_name = format!(
                        "__instance_{}_{}_{}_{}",
                        instance_decl.class_name,
                        member.name,
                        instance_decl.subject,
                        import_value_type_label(&member.ty),
                    );
                    let name = self.make_name(&synthetic_name, item.base.span);
                    imports.push(self.alloc_import(ImportBinding {
                        span: item.base.span,
                        imported_name: self.make_name(&member.name, item.base.span),
                        local_name: name,
                        resolution: ImportBindingResolution::Resolved,
                        metadata: ImportBindingMetadata::InstanceMember {
                            class_name: instance_decl.class_name.clone(),
                            member_name: member.name.clone(),
                            subject: instance_decl.subject.clone(),
                            ty: member.ty.clone(),
                        },
                        callable_type: None,
                        deprecation: None,
                    }));
                }
            }
        }
        if imports.is_empty() {
            self.emit_error(
                item.base.span,
                "use declaration must import at least one member",
                code("empty-use-imports"),
            );
            imports.push(self.alloc_import(ImportBinding {
                span: item.base.span,
                imported_name: self.make_name("invalid", item.base.span),
                local_name: self.make_name("invalid", item.base.span),
                resolution: ImportBindingResolution::UnknownModule,
                metadata: ImportBindingMetadata::Unknown,
                callable_type: None,
                deprecation: None,
            }));
        }
        let imports =
            crate::NonEmpty::from_vec(imports).expect("fallback import list should be non-empty");

        UseItem {
            header,
            module,
            imports,
        }
    }

    fn lower_export_items(&mut self, item: &syn::ExportItem) -> Vec<ExportItem> {
        if item.targets.is_empty() {
            return vec![self.lower_export_target(item, None)];
        }

        item.targets
            .iter()
            .map(|target| self.lower_export_target(item, Some(target)))
            .collect()
    }

    fn lower_export_target(
        &mut self,
        item: &syn::ExportItem,
        target: Option<&syn::Identifier>,
    ) -> ExportItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Export, item.base.span);
        let target_name = self.required_name(target, item.base.span, "export declaration");
        let target = self.make_path(&[target_name]);
        ExportItem {
            header,
            target,
            resolution: ResolutionState::Unresolved,
        }
    }

    fn lower_hoist_item(&mut self, item: &syn::HoistItem) -> HoistItem {
        let header =
            self.lower_item_header(&item.base.decorators, ItemKind::Hoist, item.base.span);
        let module = item
            .path
            .as_ref()
            .map(|path| self.lower_qualified_name(path))
            .unwrap_or_else(|| {
                self.emit_error(
                    item.base.span,
                    "hoist declaration is missing a module path",
                    code("missing-use-path"),
                );
                self.make_path(&[self.make_name("invalid", item.base.span)])
            });

        let kind_filters = item
            .kind_filters
            .iter()
            .filter_map(|f| match f.text.as_str() {
                "func" => Some(HoistKindFilter::Func),
                "value" => Some(HoistKindFilter::Value),
                "signal" => Some(HoistKindFilter::Signal),
                "type" => Some(HoistKindFilter::Type),
                "domain" => Some(HoistKindFilter::Domain),
                "class" => Some(HoistKindFilter::Class),
                other => {
                    self.emit_error(
                        f.span,
                        format!("unknown hoist kind filter `{other}`; expected one of: func, value, signal, type, domain, class"),
                        code("unknown-hoist-kind-filter"),
                    );
                    None
                }
            })
            .collect();

        let hiding = item
            .hiding
            .iter()
            .map(|ident| self.make_name(&ident.text, ident.span))
            .collect();

        HoistItem {
            header,
            module,
            kind_filters,
            hiding,
        }
    }

    fn resolve_import_binding(
        &self,
        module_name: &str,
        imported_name: &Name,
        module_resolution: &ImportModuleResolution,
    ) -> (
        ImportBindingResolution,
        ImportBindingMetadata,
        Option<ImportValueType>,
        Option<crate::DeprecationNotice>,
    ) {
        match module_resolution {
            ImportModuleResolution::Resolved(exports) => match exports.find(imported_name.text()) {
                Some(exported) => {
                    // Ambient values provide full polymorphic type inference through the
                    // prelude item system. Always prefer them over exported callable_type,
                    // which only carries the portable ImportValueType representation.
                    if let Some(metadata) =
                        known_import_metadata(module_name, imported_name.text())
                    {
                        if matches!(
                            metadata,
                            ImportBindingMetadata::AmbientValue { .. }
                                | ImportBindingMetadata::AmbientType
                        ) {
                            return (ImportBindingResolution::Resolved, metadata, None, None);
                        }
                    }
                    // For non-ambient exports, fall back to known_import_metadata when
                    // the export is opaque (e.g. stdlib modules that can't resolve their
                    // own self-imports).
                    if matches!(exported.metadata, ImportBindingMetadata::OpaqueValue) {
                        if let Some(metadata) =
                            known_import_metadata(module_name, imported_name.text())
                        {
                            return (ImportBindingResolution::Resolved, metadata, None, None);
                        }
                    }
                    (
                        ImportBindingResolution::Resolved,
                        exported.metadata.clone(),
                        exported.callable_type.clone(),
                        exported.deprecation.clone(),
                    )
                }
                // Fall back to compiler-known intrinsics when the stdlib file exists but
                // does not re-export the builtin function (e.g. aivi.stdio.stdoutWrite).
                None => match known_import_metadata(module_name, imported_name.text()) {
                    Some(metadata) => (ImportBindingResolution::Resolved, metadata, None, None),
                    None => (
                        ImportBindingResolution::MissingExport,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                },
            },
            ImportModuleResolution::Missing => {
                match known_import_metadata(module_name, imported_name.text()) {
                    Some(metadata) => (ImportBindingResolution::Resolved, metadata, None, None),
                    None if is_known_module(module_name) => (
                        ImportBindingResolution::MissingExport,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                    None => (
                        ImportBindingResolution::UnknownModule,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                }
            }
            ImportModuleResolution::Cycle(_) => {
                // For direct self-imports, intrinsics are still accessible by name.
                match known_import_metadata(module_name, imported_name.text()) {
                    Some(metadata) => (ImportBindingResolution::Resolved, metadata, None, None),
                    None => (
                        ImportBindingResolution::Cycle,
                        ImportBindingMetadata::Unknown,
                        None,
                        None,
                    ),
                }
            }
        }
    }

    fn lower_item_header(
        &mut self,
        decorators: &[syn::Decorator],
        target: ItemKind,
        span: SourceSpan,
    ) -> ItemHeader {
        let lowered_decorators = decorators
            .iter()
            .map(|decorator| self.lower_decorator(decorator, target))
            .collect();
        self.validate_recurrence_wakeup_decorator_set(decorators, target);
        ItemHeader {
            span,
            decorators: lowered_decorators,
        }
    }

    fn lower_decorator(&mut self, decorator: &syn::Decorator, target: ItemKind) -> DecoratorId {
        let name = self.lower_qualified_name(&decorator.name);
        let payload = if is_source_decorator(&name) {
            if target != ItemKind::Signal {
                self.emit_error(
                    decorator.span,
                    "`@source` is only valid on `sig` declarations in Milestone 2",
                    code("invalid-source-target"),
                );
            }
            match &decorator.payload {
                syn::DecoratorPayload::Source(source) => {
                    let source = SourceDecorator {
                        provider: source
                            .provider
                            .as_ref()
                            .map(|provider| self.lower_qualified_name(provider)),
                        arguments: source
                            .arguments
                            .iter()
                            .map(|expr| self.lower_expr(expr))
                            .collect(),
                        options: source
                            .options
                            .as_ref()
                            .map(|record| self.lower_record_expr_as_expr(record)),
                    };
                    self.validate_source_decorator_shape(decorator.span, &source);
                    DecoratorPayload::Source(source)
                }
                syn::DecoratorPayload::Arguments(arguments) => {
                    let source = SourceDecorator {
                        provider: None,
                        arguments: arguments
                            .arguments
                            .iter()
                            .map(|expr| self.lower_expr(expr))
                            .collect(),
                        options: arguments
                            .options
                            .as_ref()
                            .map(|record| self.lower_record_expr_as_expr(record)),
                    };
                    self.validate_source_decorator_shape(decorator.span, &source);
                    DecoratorPayload::Source(source)
                }
                syn::DecoratorPayload::Bare => {
                    let source = SourceDecorator {
                        provider: None,
                        arguments: Vec::new(),
                        options: None,
                    };
                    self.validate_source_decorator_shape(decorator.span, &source);
                    DecoratorPayload::Source(source)
                }
            }
        } else if let Some(kind) = recurrence_wakeup_decorator_kind(&name) {
            if !matches!(
                target,
                ItemKind::Value | ItemKind::Function | ItemKind::Signal
            ) {
                self.emit_error(
                    decorator.span,
                    format!(
                        "`@{}` is only valid on `value`, `func`, or `signal` declarations in Milestone 2",
                        path_text(&name)
                    ),
                    code("invalid-recurrence-wakeup-target"),
                );
            }
            self.lower_recurrence_wakeup_decorator_payload(decorator, kind)
        } else if is_test_decorator(&name) {
            if target != ItemKind::Value {
                self.emit_error(
                    decorator.span,
                    "`@test` is only valid on top-level `val` declarations",
                    code("invalid-test-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            if !call.arguments.is_empty() || call.options.is_some() {
                self.emit_error(
                    decorator.span,
                    "`@test` does not accept arguments or `with { ... }` options",
                    code("invalid-test-decorator"),
                );
                DecoratorPayload::Call(call)
            } else {
                DecoratorPayload::Test(TestDecorator)
            }
        } else if is_debug_decorator(&name) {
            if !matches!(
                target,
                ItemKind::Value | ItemKind::Function | ItemKind::Signal
            ) {
                self.emit_error(
                    decorator.span,
                    "`@debug` is only valid on top-level `value`, `func`, or `signal` declarations",
                    code("invalid-debug-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            if !call.arguments.is_empty() || call.options.is_some() {
                self.emit_error(
                    decorator.span,
                    "`@debug` does not accept arguments or `with { ... }` options",
                    code("invalid-debug-decorator"),
                );
                DecoratorPayload::Call(call)
            } else {
                DecoratorPayload::Debug(DebugDecorator)
            }
        } else if is_deprecated_decorator(&name) {
            if !matches!(
                target,
                ItemKind::Type
                    | ItemKind::Value
                    | ItemKind::Function
                    | ItemKind::Signal
                    | ItemKind::Class
                    | ItemKind::Domain
            ) {
                self.emit_error(
                    decorator.span,
                    "`@deprecated` is only valid on top-level named type, value, function, signal, class, or domain declarations",
                    code("invalid-deprecated-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            if call.arguments.len() > 1 {
                self.emit_error(
                    decorator.span,
                    "`@deprecated` accepts at most one positional text message",
                    code("invalid-deprecated-decorator"),
                );
                DecoratorPayload::Call(call)
            } else {
                DecoratorPayload::Deprecated(DeprecatedDecorator {
                    message: call.arguments.first().copied(),
                    options: call.options,
                })
            }
        } else if is_mock_decorator(&name) {
            if target != ItemKind::Value {
                self.emit_error(
                    decorator.span,
                    "`@mock` is only valid on top-level `val` declarations",
                    code("invalid-mock-target"),
                );
            }
            let call = self.lower_call_like_decorator_payload(&decorator.payload);
            let mock_arguments = if call.options.is_some() {
                None
            } else if call.arguments.len() == 2 {
                Some((call.arguments[0], call.arguments[1]))
            } else if call.arguments.len() == 1 {
                match &self.module.exprs()[call.arguments[0]].kind {
                    ExprKind::Tuple(arguments) if arguments.len() == 2 => {
                        Some((*arguments.first(), *arguments.second()))
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some((target, replacement)) = mock_arguments {
                DecoratorPayload::Mock(MockDecorator {
                    target,
                    replacement,
                })
            } else {
                self.emit_error(
                    decorator.span,
                    "`@mock` must carry exactly two positional references and no `with { ... }` options",
                    code("invalid-mock-decorator"),
                );
                DecoratorPayload::Call(call)
            }
        } else {
            self.emit_error(
                decorator.span,
                format!("unknown decorator `@{}`", path_text(&name)),
                code("unknown-decorator"),
            );
            match &decorator.payload {
                syn::DecoratorPayload::Bare => DecoratorPayload::Bare,
                syn::DecoratorPayload::Arguments(_) | syn::DecoratorPayload::Source(_) => {
                    DecoratorPayload::Call(
                        self.lower_call_like_decorator_payload(&decorator.payload),
                    )
                }
            }
        };

        self.alloc_decorator(Decorator {
            span: decorator.span,
            name,
            payload,
        })
    }

    fn lower_recurrence_wakeup_decorator_payload(
        &mut self,
        decorator: &syn::Decorator,
        kind: RecurrenceWakeupDecoratorKind,
    ) -> DecoratorPayload {
        let call = self.lower_call_like_decorator_payload(&decorator.payload);
        if call.options.is_some() || call.arguments.len() != 1 {
            self.emit_error(
                decorator.span,
                format!(
                    "`@{}` must carry exactly one positional witness expression and no `with {{ ... }}` options",
                    decorator.name.as_dotted()
                ),
                code("invalid-recurrence-wakeup-decorator"),
            );
            return DecoratorPayload::Call(call);
        }
        DecoratorPayload::RecurrenceWakeup(RecurrenceWakeupDecorator {
            kind,
            witness: call.arguments[0],
        })
    }

    fn lower_call_like_decorator_payload(
        &mut self,
        payload: &syn::DecoratorPayload,
    ) -> DecoratorCall {
        match payload {
            syn::DecoratorPayload::Bare => DecoratorCall {
                arguments: Vec::new(),
                options: None,
            },
            syn::DecoratorPayload::Arguments(arguments) => DecoratorCall {
                arguments: arguments
                    .arguments
                    .iter()
                    .map(|expr| self.lower_expr(expr))
                    .collect(),
                options: arguments
                    .options
                    .as_ref()
                    .map(|record| self.lower_record_expr_as_expr(record)),
            },
            syn::DecoratorPayload::Source(source) => DecoratorCall {
                arguments: source
                    .arguments
                    .iter()
                    .map(|expr| self.lower_expr(expr))
                    .collect(),
                options: source
                    .options
                    .as_ref()
                    .map(|record| self.lower_record_expr_as_expr(record)),
            },
        }
    }

    fn validate_recurrence_wakeup_decorator_set(
        &mut self,
        decorators: &[syn::Decorator],
        target: ItemKind,
    ) {
        let source = decorators
            .iter()
            .find(|decorator| decorator.name.as_dotted() == "source");
        let mut first_recurrence: Option<&syn::Decorator> = None;
        for decorator in decorators {
            if !is_recurrence_wakeup_decorator_name(&decorator.name) {
                continue;
            }
            if let Some(first) = first_recurrence {
                self.emit_error(
                    decorator.span,
                    format!(
                        "`@{}` cannot be combined with `@{}`; current non-source recurrence proofs accept at most one wakeup witness per declaration",
                        decorator.name.as_dotted(),
                        first.name.as_dotted()
                    ),
                    code("duplicate-recurrence-wakeup-decorator"),
                );
            } else {
                first_recurrence = Some(decorator);
            }
            if target == ItemKind::Signal && source.is_some() {
                self.emit_error(
                    decorator.span,
                    format!(
                        "`@{}` is only valid on non-`@source` recurrence declarations in the current wakeup slice",
                        decorator.name.as_dotted()
                    ),
                    code("invalid-source-recurrence-wakeup"),
                );
            }
        }
    }

    fn validate_source_decorator_shape(&mut self, span: SourceSpan, source: &SourceDecorator) {
        let provider = match source.provider.as_ref() {
            Some(provider) => match SourceProviderRef::from_path(Some(provider)) {
                SourceProviderRef::Builtin(provider_ref) => Some(provider_ref),
                SourceProviderRef::Custom(_) => None,
                SourceProviderRef::InvalidShape(_) => {
                    if !crate::capability_handle_elaboration::is_builtin_source_capability_family_path(provider)
                    {
                        self.emit_error(
                            provider.span(),
                            "source decorators must name a provider variant such as `timer.every`",
                            code("invalid-source-provider"),
                        );
                    }
                    None
                }
                SourceProviderRef::Missing => unreachable!(
                    "classifying an explicit source provider should never yield Missing"
                ),
            },
            None => {
                self.emit_error(
                    span,
                    "source decorators must name a provider variant such as `timer.every`",
                    code("missing-source-provider"),
                );
                None
            }
        };

        let Some(options) = source.options else {
            return;
        };
        let ExprKind::Record(record) = &self.module.exprs()[options].kind else {
            self.emit_error(
                span,
                "`@source ... with` options must lower to a closed record literal",
                code("invalid-source-options"),
            );
            return;
        };

        let mut seen = HashMap::new();
        for field in &record.fields {
            if let Some(previous_span) = seen.insert(field.label.text().to_owned(), field.span) {
                self.diagnostics.push(
                    Diagnostic::error(format!("duplicate source option `{}`", field.label.text()))
                        .with_code(code("duplicate-source-option"))
                        .with_primary_label(field.span, "this option label is repeated")
                        .with_secondary_label(previous_span, "previous source option here"),
                );
            }
            if let Some(provider) = provider {
                let contract = provider.contract();
                if contract.option(field.label.text()).is_none() {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "unknown source option `{}` for `{}`",
                            field.label.text(),
                            provider.key()
                        ))
                        .with_code(code("unknown-source-option"))
                        .with_primary_label(
                            field.span,
                            "this option is not supported for the selected source provider",
                        ),
                    );
                }
            }
        }
    }

    fn lower_function_parameter(&mut self, parameter: &syn::FunctionParam) -> FunctionParameter {
        let name = self.required_name(
            parameter.name.as_ref(),
            parameter.span,
            "function parameter",
        );
        let binding = self.alloc_binding(Binding {
            span: parameter.span,
            name: name.clone(),
            kind: BindingKind::FunctionParameter,
        });
        FunctionParameter {
            span: parameter.span,
            binding,
            annotation: parameter
                .annotation
                .as_ref()
                .map(|annotation| self.lower_type_expr(annotation)),
        }
    }

    fn lower_instance_parameter(&mut self, parameter: &syn::Identifier) -> FunctionParameter {
        let name = self.make_name(&parameter.text, parameter.span);
        let binding = self.alloc_binding(Binding {
            span: parameter.span,
            name,
            kind: BindingKind::FunctionParameter,
        });
        FunctionParameter {
            span: parameter.span,
            binding,
            annotation: None,
        }
    }

    fn lower_type_parameters(&mut self, parameters: &[syn::Identifier]) -> Vec<TypeParameterId> {
        parameters
            .iter()
            .map(|parameter| {
                self.alloc_type_parameter(TypeParameter {
                    span: parameter.span,
                    name: self.make_name(&parameter.text, parameter.span),
                })
            })
            .collect()
    }

    fn lower_expr(&mut self, expr: &syn::Expr) -> ExprId {
        match &expr.kind {
            syn::ExprKind::Group(inner) => self.lower_expr(inner),
            syn::ExprKind::Name(name) => {
                let reference = TermReference::unresolved(
                    self.make_path(&[self.make_name(&name.text, name.span)]),
                );
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Name(reference),
                })
            }
            syn::ExprKind::Integer(integer) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Integer(IntegerLiteral {
                    raw: integer.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::Float(float) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Float(FloatLiteral {
                    raw: float.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::Decimal(decimal) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Decimal(DecimalLiteral {
                    raw: decimal.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::BigInt(bigint) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::BigInt(BigIntLiteral {
                    raw: bigint.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::SuffixedInteger(literal) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::SuffixedInteger(SuffixedIntegerLiteral {
                    raw: literal.literal.raw.clone().into_boxed_str(),
                    suffix: self.make_name(&literal.suffix.text, literal.suffix.span),
                    resolution: ResolutionState::Unresolved,
                }),
            }),
            syn::ExprKind::Text(text) => {
                let text = self.lower_text_literal(text);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Text(text),
                })
            }
            syn::ExprKind::Regex(regex) => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::Regex(RegexLiteral {
                    raw: regex.raw.clone().into_boxed_str(),
                }),
            }),
            syn::ExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_expr(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            expr.span,
                            "tuple expressions require at least two elements",
                            code("short-tuple-expr"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_expr(expr.span),
                            self.placeholder_expr(expr.span),
                            Vec::new(),
                        )
                    }
                };
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Tuple(elements),
                })
            }
            syn::ExprKind::List(elements) => {
                if let [
                    syn::Expr {
                        kind: syn::ExprKind::Range { start, end },
                        ..
                    },
                ] = elements.as_slice()
                {
                    return self.lower_integer_range_expr(expr.span, start, end);
                }
                let elements = elements
                    .iter()
                    .map(|element| self.lower_expr(element))
                    .collect();
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::List(elements),
                })
            }
            syn::ExprKind::Map(map) => {
                let map = self.lower_map_expr(map);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Map(map),
                })
            }
            syn::ExprKind::Set(elements) => {
                let mut seen_elements =
                    Vec::<(&syn::Expr, SourceSpan)>::with_capacity(elements.len());
                let mut lowered_elements = Vec::with_capacity(elements.len());
                for element in elements {
                    if let Some((_, previous_span)) = seen_elements
                        .iter()
                        .find(|(previous, _)| surface_exprs_equal(previous, element))
                    {
                        self.diagnostics.push(
                            Diagnostic::warning(
                                "duplicate set element is redundant and will be ignored",
                            )
                            .with_code(code("duplicate-set-element"))
                            .with_primary_label(
                                element.span,
                                "this element duplicates an earlier set entry",
                            )
                            .with_secondary_label(
                                *previous_span,
                                "previous equivalent set element here",
                            )
                            .with_note(
                                "set literals are canonicalized during HIR lowering to one structurally equal entry",
                            ),
                        );
                        continue;
                    }
                    seen_elements.push((element, element.span));
                    lowered_elements.push(self.lower_expr(element));
                }
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Set(lowered_elements),
                })
            }
            syn::ExprKind::Record(record) => {
                // Detect record projection: { field: . } or { a.b.c: . }
                // When a field value is SubjectPlaceholder, this is a projection
                // from the ambient subject, not record construction.
                if let Some(proj_field) = record.fields.iter().find(|f| {
                    matches!(
                        f.value.as_ref().map(|v| &v.kind),
                        Some(syn::ExprKind::SubjectPlaceholder)
                    )
                }) {
                    let mut names = vec![
                        self.make_name(&proj_field.label.text, proj_field.label.span),
                    ];
                    for seg in &proj_field.label_path {
                        names.push(self.make_name(&seg.text, seg.span));
                    }
                    let path = self.make_path(&names);
                    self.alloc_expr(Expr {
                        span: expr.span,
                        kind: ExprKind::Projection {
                            base: ProjectionBase::Ambient,
                            path,
                        },
                    })
                } else {
                    let record = self.lower_record_expr(record);
                    self.alloc_expr(Expr {
                        span: expr.span,
                        kind: ExprKind::Record(record),
                    })
                }
            }
            syn::ExprKind::SubjectPlaceholder => self.alloc_expr(Expr {
                span: expr.span,
                kind: ExprKind::AmbientSubject,
            }),
            syn::ExprKind::AmbientProjection(path) => {
                let path = self.lower_projection_path(path);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        path,
                    },
                })
            }
            syn::ExprKind::Range { start, end } => {
                self.lower_integer_range_expr(expr.span, start, end)
            }
            syn::ExprKind::Projection { base, path } => {
                let base = self.lower_expr(base);
                let path = self.lower_projection_path(path);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        path,
                    },
                })
            }
            syn::ExprKind::Apply { callee, arguments } => {
                let callee = self.lower_expr(callee);
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower_expr(argument))
                    .collect::<Vec<_>>();
                let arguments = match crate::NonEmpty::from_vec(arguments) {
                    Ok(arguments) => arguments,
                    Err(_) => {
                        self.emit_error(
                            expr.span,
                            "applications require at least one argument",
                            code("empty-apply-args"),
                        );
                        crate::NonEmpty::new(self.placeholder_expr(expr.span), Vec::new())
                    }
                };
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Apply { callee, arguments },
                })
            }
            syn::ExprKind::Unary {
                operator,
                expr: inner,
            } => {
                let inner = self.lower_expr(inner);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Unary {
                        operator: lower_unary_operator(*operator),
                        expr: inner,
                    },
                })
            }
            syn::ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.lower_expr(left);
                let right = self.lower_expr(right);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Binary {
                        left,
                        operator: lower_binary_operator(*operator),
                        right,
                    },
                })
            }
            syn::ExprKind::ResultBlock(block) => self.lower_result_block_expr(block),
            syn::ExprKind::PatchApply { target, patch } => {
                let target = self.lower_expr(target);
                let patch = self.lower_patch_block(patch);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::PatchApply { target, patch },
                })
            }
            syn::ExprKind::PatchLiteral(patch) => {
                let patch = self.lower_patch_block(patch);
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::PatchLiteral(patch),
                })
            }
            syn::ExprKind::Pipe(pipe) => self.lower_pipe_expr(pipe),
            syn::ExprKind::Markup(markup) => {
                let markup = self.lower_markup_node(markup, MarkupPlacement::Renderable);
                let span = markup.span(self);
                let markup = self.renderable_markup(markup, span, "top-level markup");
                self.alloc_expr(Expr {
                    span: expr.span,
                    kind: ExprKind::Markup(markup),
                })
            }
            syn::ExprKind::OperatorSection(op) => self.lower_operator_section(*op, expr.span),
        }
    }

    fn lower_operator_section(
        &mut self,
        op: syn::BinaryOperator,
        span: aivi_base::SourceSpan,
    ) -> ExprId {
        let ambient_name = match op {
            syn::BinaryOperator::Equals => Some("__aivi_binary_eq"),
            syn::BinaryOperator::NotEquals => Some("__aivi_binary_neq"),
            _ => None,
        };
        if let Some(name) = ambient_name {
            let reference =
                TermReference::unresolved(self.make_path(&[self.make_name(name, span)]));
            self.alloc_expr(Expr {
                span,
                kind: ExprKind::Name(reference),
            })
        } else {
            self.diagnostics.push(
                aivi_base::Diagnostic::error("operator section not supported for this operator")
                    .with_code(code("unsupported-operator-section"))
                    .with_primary_label(
                        span,
                        "no built-in function backing for this operator section",
                    ),
            );
            let reference = TermReference::unresolved(
                self.make_path(&[self.make_name("__aivi_binary_eq", span)]),
            );
            self.alloc_expr(Expr {
                span,
                kind: ExprKind::Name(reference),
            })
        }
    }

    fn lower_patch_block(&mut self, patch: &syn::PatchBlock) -> PatchBlock {
        PatchBlock {
            entries: patch
                .entries
                .iter()
                .map(|entry| PatchEntry {
                    span: entry.span,
                    selector: self.lower_patch_selector(&entry.selector),
                    instruction: self.lower_patch_instruction(&entry.instruction),
                })
                .collect(),
        }
    }

    fn lower_patch_selector(&mut self, selector: &syn::PatchSelector) -> PatchSelector {
        PatchSelector {
            span: selector.span,
            segments: selector
                .segments
                .iter()
                .map(|segment| match segment {
                    syn::PatchSelectorSegment::Named { name, dotted, span } => {
                        PatchSelectorSegment::Named {
                            name: self.make_name(&name.text, name.span),
                            dotted: *dotted,
                            span: *span,
                        }
                    }
                    syn::PatchSelectorSegment::BracketTraverse { span } => {
                        PatchSelectorSegment::BracketTraverse { span: *span }
                    }
                    syn::PatchSelectorSegment::BracketExpr { expr, span } => {
                        PatchSelectorSegment::BracketExpr {
                            expr: self.lower_expr(expr),
                            span: *span,
                        }
                    }
                })
                .collect(),
        }
    }

    fn lower_patch_instruction(&mut self, instruction: &syn::PatchInstruction) -> PatchInstruction {
        PatchInstruction {
            span: instruction.span,
            kind: match &instruction.kind {
                syn::PatchInstructionKind::Replace(expr) => {
                    PatchInstructionKind::Replace(self.lower_expr(expr))
                }
                syn::PatchInstructionKind::Store(expr) => {
                    PatchInstructionKind::Store(self.lower_expr(expr))
                }
                syn::PatchInstructionKind::Remove => PatchInstructionKind::Remove,
            },
        }
    }

    fn lower_integer_range_expr(
        &mut self,
        span: SourceSpan,
        start: &syn::Expr,
        end: &syn::Expr,
    ) -> ExprId {
        let Some(start) = self.parse_compile_time_range_bound(start) else {
            self.emit_error(
                span,
                "range bounds must be plain `Int` literals in this surface revision",
                code("invalid-range-bounds"),
            );
            return self.placeholder_expr(span);
        };
        let Some(end) = self.parse_compile_time_range_bound(end) else {
            self.emit_error(
                span,
                "range bounds must be plain `Int` literals in this surface revision",
                code("invalid-range-bounds"),
            );
            return self.placeholder_expr(span);
        };

        let element_count = start.abs_diff(end).saturating_add(1);
        if element_count > MAX_COMPILE_TIME_RANGE_ELEMENTS {
            self.emit_error(
                span,
                format!(
                    "range expands to {element_count} elements, which exceeds the compile-time limit of {MAX_COMPILE_TIME_RANGE_ELEMENTS}"
                ),
                code("range-too-large"),
            );
            return self.placeholder_expr(span);
        }

        let step = if start <= end { 1 } else { -1 };
        let mut current = start;
        let mut elements = Vec::with_capacity(element_count as usize);
        loop {
            elements.push(self.alloc_expr(Expr {
                span,
                kind: ExprKind::Integer(IntegerLiteral {
                    raw: current.to_string().into_boxed_str(),
                }),
            }));
            if current == end {
                break;
            }
            current += step;
        }

        self.alloc_expr(Expr {
            span,
            kind: ExprKind::List(elements),
        })
    }

    fn parse_compile_time_range_bound(&self, expr: &syn::Expr) -> Option<i64> {
        let syn::ExprKind::Integer(integer) = &expr.kind else {
            return None;
        };
        integer.raw.parse::<i64>().ok()
    }

    fn lower_result_block_expr(&mut self, block: &syn::ResultBlockExpr) -> ExprId {
        let Some(mut current) = self.lower_result_block_tail(block) else {
            self.emit_error(
                block.span,
                "result blocks must produce a final success value",
                code("empty-result-block"),
            );
            return self.placeholder_expr(block.span);
        };
        for binding in block.bindings.iter().rev() {
            let source = self.lower_expr(&binding.expr);
            current = self.lower_result_binding(binding, source, current);
        }
        current
    }

    fn lower_result_block_tail(&mut self, block: &syn::ResultBlockExpr) -> Option<ExprId> {
        let tail = match block.tail.as_deref() {
            Some(expr) => self.lower_expr(expr),
            None => {
                let binding = block.bindings.last()?;
                self.lower_unresolved_name_expr(&binding.name.text, binding.name.span)
            }
        };
        Some(self.lower_constructor_apply_expr("Ok", block.span, vec![tail]))
    }

    fn lower_result_binding(
        &mut self,
        binding: &syn::ResultBinding,
        source: ExprId,
        ok_body: ExprId,
    ) -> ExprId {
        let ok_binding_name = self.make_name(&binding.name.text, binding.name.span);
        let ok_binding = self.alloc_binding(Binding {
            span: binding.name.span,
            name: ok_binding_name.clone(),
            kind: BindingKind::Pattern,
        });
        let ok_argument = self.alloc_pattern(Pattern {
            span: binding.name.span,
            kind: PatternKind::Binding(BindingPattern {
                binding: ok_binding,
                name: ok_binding_name,
            }),
        });
        let ok_pattern = self.alloc_pattern(Pattern {
            span: binding.span,
            kind: PatternKind::Constructor {
                callee: self.make_unresolved_term_reference("Ok", binding.name.span),
                arguments: vec![ok_argument],
            },
        });

        let error_name = format!("__resultBlockErr{}", self.module.bindings().len());
        let error_span = binding.expr.span;
        let error_binding_name = self.make_name(&error_name, error_span);
        let error_binding = self.alloc_binding(Binding {
            span: error_span,
            name: error_binding_name.clone(),
            kind: BindingKind::Pattern,
        });
        let error_argument = self.alloc_pattern(Pattern {
            span: error_span,
            kind: PatternKind::Binding(BindingPattern {
                binding: error_binding,
                name: error_binding_name,
            }),
        });
        let err_pattern = self.alloc_pattern(Pattern {
            span: binding.span,
            kind: PatternKind::Constructor {
                callee: self.make_unresolved_term_reference("Err", binding.expr.span),
                arguments: vec![error_argument],
            },
        });
        let err_value = self.lower_unresolved_name_expr(&error_name, error_span);
        let err_body = self.lower_constructor_apply_expr("Err", binding.expr.span, vec![err_value]);

        let ok_stage = PipeStage {
            span: binding.span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: ok_pattern,
                body: ok_body,
            },
        };
        let err_stage = PipeStage {
            span: binding.span,
            subject_memo: None,
            result_memo: None,
            kind: PipeStageKind::Case {
                pattern: err_pattern,
                body: err_body,
            },
        };
        self.alloc_expr(Expr {
            span: binding.span,
            kind: ExprKind::Pipe(PipeExpr {
                head: source,
                stages: crate::NonEmpty::new(ok_stage, vec![err_stage]),
                result_block_desugaring: true,
            }),
        })
    }

    fn lower_constructor_apply_expr(
        &mut self,
        constructor: &str,
        span: SourceSpan,
        arguments: Vec<ExprId>,
    ) -> ExprId {
        let callee = self.lower_unresolved_name_expr(constructor, span);
        let arguments = crate::NonEmpty::from_vec(arguments)
            .expect("result block constructors always receive at least one argument");
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Apply { callee, arguments },
        })
    }

    fn lower_unresolved_name_expr(&mut self, name: &str, span: SourceSpan) -> ExprId {
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Name(self.make_unresolved_term_reference(name, span)),
        })
    }

    fn make_unresolved_term_reference(&self, name: &str, span: SourceSpan) -> TermReference {
        TermReference::unresolved(self.make_path(&[self.make_name(name, span)]))
    }

    fn lower_text_literal(&mut self, text: &syn::TextLiteral) -> TextLiteral {
        TextLiteral {
            segments: text
                .segments
                .iter()
                .map(|segment| match segment {
                    syn::TextSegment::Text(fragment) => TextSegment::Text(TextFragment {
                        raw: fragment.raw.clone().into_boxed_str(),
                        span: fragment.span,
                    }),
                    syn::TextSegment::Interpolation(interpolation) => {
                        TextSegment::Interpolation(TextInterpolation {
                            span: interpolation.span,
                            expr: self.lower_expr(&interpolation.expr),
                        })
                    }
                })
                .collect(),
        }
    }

    fn lower_pipe_expr(&mut self, pipe: &syn::PipeExpr) -> ExprId {
        self.validate_pipe_stages(&pipe.stages);
        let mut current = pipe.head.as_ref().map(|head| self.lower_expr(head));
        let mut ordinary = Vec::new();
        let mut index = 0;
        while index < pipe.stages.len() {
            match &pipe.stages[index].kind {
                syn::PipeStageKind::Apply { .. } => {
                    self.flush_pipe_segment(&mut current, &mut ordinary, pipe.span);
                    let cluster_expr =
                        self.lower_cluster_segment(current.take(), &pipe.stages, &mut index);
                    current = Some(cluster_expr);
                }
                syn::PipeStageKind::ClusterFinalizer { expr } => {
                    self.emit_error(
                        pipe.stages[index].span,
                        "cluster finalizer appeared without an active `&|>` region",
                        code("orphan-cluster-finalizer"),
                    );
                    ordinary.push(PipeStage {
                        span: pipe.stages[index].span,
                        subject_memo: None,
                        result_memo: None,
                        kind: PipeStageKind::Transform {
                            expr: self.lower_expr(expr),
                        },
                    });
                    index += 1;
                }
                _ => {
                    ordinary.push(self.lower_pipe_stage(&pipe.stages[index]));
                    index += 1;
                }
            }
        }
        self.flush_pipe_segment(&mut current, &mut ordinary, pipe.span);
        current.unwrap_or_else(|| {
            self.emit_error(
                pipe.span,
                "pipe expression is missing a head expression",
                code("missing-pipe-head"),
            );
            self.placeholder_expr(pipe.span)
        })
    }

    fn flush_pipe_segment(
        &mut self,
        current: &mut Option<ExprId>,
        ordinary: &mut Vec<PipeStage>,
        span: SourceSpan,
    ) {
        if ordinary.is_empty() {
            return;
        }
        let head = current.take().unwrap_or_else(|| {
            self.emit_error(
                span,
                "pipe stage sequence is missing a head expression",
                code("missing-pipe-head"),
            );
            self.placeholder_expr(span)
        });
        let stages = std::mem::take(ordinary);
        let stages =
            crate::NonEmpty::from_vec(stages).expect("flush only runs for non-empty stage buffers");
        let expr = self.alloc_expr(Expr {
            span,
            kind: ExprKind::Pipe(PipeExpr {
                head,
                stages,
                result_block_desugaring: false,
            }),
        });
        *current = Some(expr);
    }

    fn lower_cluster_segment(
        &mut self,
        head: Option<ExprId>,
        stages: &[syn::PipeStage],
        index: &mut usize,
    ) -> ExprId {
        let presentation = if head.is_some() {
            ClusterPresentation::ExpressionHeaded
        } else {
            ClusterPresentation::Leading
        };
        let mut members = head.into_iter().collect::<Vec<_>>();
        let mut cluster_span = members
            .first()
            .and_then(|expr| self.module.exprs().get(*expr))
            .map(|expr| expr.span)
            .unwrap_or(stages[*index].span);
        while *index < stages.len() {
            match &stages[*index].kind {
                syn::PipeStageKind::Apply { expr } => {
                    let lowered = self.lower_expr(expr);
                    cluster_span = cluster_span
                        .join(self.module.exprs()[lowered].span)
                        .unwrap_or(cluster_span);
                    members.push(lowered);
                    *index += 1;
                }
                _ => break,
            }
        }

        let finalizer = if *index < stages.len() {
            if let syn::PipeStageKind::ClusterFinalizer { expr } = &stages[*index].kind {
                let lowered = self.lower_expr(expr);
                cluster_span = cluster_span
                    .join(self.module.exprs()[lowered].span)
                    .unwrap_or(cluster_span);
                *index += 1;
                ClusterFinalizer::Explicit(lowered)
            } else {
                self.emit_error(
                    stages[*index].span,
                    "unfinished `&|>` cluster cannot continue with this pipe stage",
                    code("illegal-unfinished-cluster"),
                );
                ClusterFinalizer::ImplicitTuple
            }
        } else {
            ClusterFinalizer::ImplicitTuple
        };

        if members.len() < 2 {
            self.emit_error(
                cluster_span,
                "`&|>` clusters require at least two members",
                code("short-cluster"),
            );
            members.push(self.placeholder_expr(cluster_span));
        }
        let members =
            AtLeastTwo::from_vec(members).expect("cluster fallback should guarantee two members");
        let cluster = self.alloc_cluster(ApplicativeCluster {
            span: cluster_span,
            presentation,
            members,
            finalizer,
        });
        self.alloc_expr(Expr {
            span: cluster_span,
            kind: ExprKind::Cluster(cluster),
        })
    }

    fn lower_pipe_stage(&mut self, stage: &syn::PipeStage) -> PipeStage {
        let supports_memos = matches!(
            stage.kind,
            syn::PipeStageKind::Transform { .. } | syn::PipeStageKind::Tap { .. }
        );
        let subject_memo = if supports_memos {
            stage
                .subject_memo
                .as_ref()
                .map(|memo| self.lower_pipe_memo_binding(memo, BindingKind::PipeSubjectMemo))
        } else {
            if let Some(memo) = &stage.subject_memo {
                self.emit_unsupported_pipe_memo(memo.span);
            }
            None
        };
        let result_memo = if supports_memos {
            stage
                .result_memo
                .as_ref()
                .map(|memo| self.lower_pipe_memo_binding(memo, BindingKind::PipeResultMemo))
        } else {
            if let Some(memo) = &stage.result_memo {
                self.emit_unsupported_pipe_memo(memo.span);
            }
            None
        };
        let kind = match &stage.kind {
            syn::PipeStageKind::Transform { expr } => PipeStageKind::Transform {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Gate { expr } => PipeStageKind::Gate {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Case(arm) => PipeStageKind::Case {
                pattern: self.lower_pattern(&arm.pattern),
                body: self.lower_expr(&arm.body),
            },
            syn::PipeStageKind::Map { expr } => PipeStageKind::Map {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Apply { expr } => PipeStageKind::Apply {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::ClusterFinalizer { expr } => PipeStageKind::Transform {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::RecurStart { expr } => PipeStageKind::RecurStart {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::RecurStep { expr } => PipeStageKind::RecurStep {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Tap { expr } => PipeStageKind::Tap {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::FanIn { expr } => PipeStageKind::FanIn {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Truthy { expr } => PipeStageKind::Truthy {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Falsy { expr } => PipeStageKind::Falsy {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Validate { expr } => PipeStageKind::Validate {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Previous { expr } => PipeStageKind::Previous {
                expr: self.lower_expr(expr),
            },
            syn::PipeStageKind::Accumulate { seed, step } => PipeStageKind::Accumulate {
                seed: self.lower_expr(seed),
                step: self.lower_expr(step),
            },
            syn::PipeStageKind::Diff { expr } => PipeStageKind::Diff {
                expr: self.lower_expr(expr),
            },
        };
        PipeStage {
            span: stage.span,
            subject_memo,
            result_memo,
            kind,
        }
    }

    fn lower_pipe_memo_binding(&mut self, memo: &syn::Identifier, kind: BindingKind) -> BindingId {
        self.alloc_binding(Binding {
            span: memo.span,
            name: self.make_name(&memo.text, memo.span),
            kind,
        })
    }

    fn emit_unsupported_pipe_memo(&mut self, span: SourceSpan) {
        self.diagnostics.push(
            Diagnostic::error(
                "pipe memo bindings are currently supported only on `|>` and `|` stages",
            )
            .with_code(code("unsupported-pipe-memo-stage"))
            .with_primary_label(
                span,
                "move this memo to a plain transform or tap stage for now",
            ),
        );
    }

    fn validate_pipe_stages(&mut self, stages: &[syn::PipeStage]) {
        self.validate_pipe_branch_and_join_stages(stages);
        self.validate_pipe_recurrence_stages(stages);
    }

    fn validate_pipe_branch_and_join_stages(&mut self, stages: &[syn::PipeStage]) {
        let mut index = 0;
        while index < stages.len() {
            match &stages[index].kind {
                syn::PipeStageKind::Truthy { .. } | syn::PipeStageKind::Falsy { .. } => {
                    let run_start = index;
                    let mut truthy = 0usize;
                    let mut falsy = 0usize;
                    while index < stages.len() {
                        match &stages[index].kind {
                            syn::PipeStageKind::Truthy { .. } => {
                                truthy += 1;
                                index += 1;
                            }
                            syn::PipeStageKind::Falsy { .. } => {
                                falsy += 1;
                                index += 1;
                            }
                            _ => break,
                        }
                    }
                    if index - run_start != 2 || truthy != 1 || falsy != 1 {
                        let mut diagnostic = Diagnostic::error(
                            "`T|>` and `F|>` must appear as one adjacent pair within a pipe spine",
                        )
                        .with_code(code("invalid-truthy-falsy-pair"))
                        .with_primary_label(
                            stages[run_start].span,
                            "this truthy/falsy shorthand run is not a single adjacent pair",
                        );
                        for stage in stages[run_start + 1..index].iter().take(2) {
                            diagnostic = diagnostic.with_secondary_label(
                                stage.span,
                                "paired truthy/falsy stage involved here",
                            );
                        }
                        self.diagnostics.push(diagnostic);
                    }
                }
                syn::PipeStageKind::FanIn { .. } => {
                    let mut scan = index;
                    while scan > 0
                        && matches!(&stages[scan - 1].kind, syn::PipeStageKind::Gate { .. })
                    {
                        scan -= 1;
                    }
                    if scan == 0
                        || !matches!(&stages[scan - 1].kind, syn::PipeStageKind::Map { .. })
                    {
                        self.diagnostics.push(
                            Diagnostic::error(
                                "`<|*` must close a `*|>` fan-out body with only `?|>` filters in between",
                            )
                                .with_code(code("orphan-fan-in"))
                                .with_primary_label(
                                    stages[index].span,
                                    "place `<|*` after the `*|>` stage it joins, allowing only `?|>` filters between them",
                                ),
                        );
                    }
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    fn validate_pipe_recurrence_stages(&mut self, stages: &[syn::PipeStage]) {
        #[derive(Clone, Copy)]
        enum RecurrenceState {
            Outside,
            AwaitingStep { start: usize },
            InSuffix { start: usize },
            AfterSuffix { start: usize },
        }

        let mut state = RecurrenceState::Outside;
        let mut index = 0usize;
        while index < stages.len() {
            match state {
                RecurrenceState::Outside => match &stages[index].kind {
                    syn::PipeStageKind::RecurStart { .. } => {
                        state = RecurrenceState::AwaitingStep { start: index };
                        index += 1;
                    }
                    syn::PipeStageKind::RecurStep { .. } => {
                        self.emit_orphan_recur_step(stages[index].span);
                        index += 1;
                    }
                    _ => index += 1,
                },
                RecurrenceState::AwaitingStep { start } => match &stages[index].kind {
                    syn::PipeStageKind::Gate { .. } => {
                        index += 1;
                    }
                    syn::PipeStageKind::RecurStep { .. } => {
                        state = RecurrenceState::InSuffix { start };
                        index += 1;
                    }
                    _ => {
                        self.emit_unfinished_recurrence(
                            stages[start].span,
                            Some(stages[index].span),
                        );
                        state = RecurrenceState::AfterSuffix { start };
                        index += 1;
                    }
                },
                RecurrenceState::InSuffix { start } => match &stages[index].kind {
                    syn::PipeStageKind::RecurStep { .. } => index += 1,
                    _ => {
                        self.emit_illegal_recurrence_continuation(
                            stages[start].span,
                            stages[index].span,
                            "this stage appears after a recurrent pipe suffix",
                        );
                        state = RecurrenceState::AfterSuffix { start };
                        index += 1;
                    }
                },
                RecurrenceState::AfterSuffix { start } => {
                    if matches!(
                        &stages[index].kind,
                        syn::PipeStageKind::RecurStart { .. }
                            | syn::PipeStageKind::RecurStep { .. }
                    ) {
                        self.emit_illegal_recurrence_continuation(
                            stages[start].span,
                            stages[index].span,
                            "this recurrent stage appears after the recurrent suffix has already ended",
                        );
                    }
                    index += 1;
                }
            }
        }

        if let RecurrenceState::AwaitingStep { start } = state {
            self.emit_unfinished_recurrence(stages[start].span, None);
        }
    }

    fn emit_orphan_recur_step(&mut self, span: SourceSpan) {
        self.diagnostics.push(
            Diagnostic::error("`<|@` must appear inside a recurrent pipe suffix started by `@|>`")
                .with_code(code("orphan-recur-step"))
                .with_primary_label(
                    span,
                    "add `@|>` before this recurrence step or remove `<|@`",
                )
                .with_note(
                    "the current structural recurrence form is a trailing suffix shaped `@|> init (?|> gate)* <|@ step (<|@ step)*`",
                ),
        );
    }

    fn emit_unfinished_recurrence(
        &mut self,
        start_span: SourceSpan,
        continuation_span: Option<SourceSpan>,
    ) {
        let mut diagnostic = Diagnostic::error(
            "`@|>` must be followed by zero or more `?|>` guards and one or more `<|@` stages",
        )
            .with_code(code("unfinished-recurrence"))
            .with_primary_label(
                start_span,
                "this recurrent suffix never receives a recurrence step",
            )
            .with_note(
                "the current structural recurrence form is a trailing suffix shaped `@|> init (?|> gate)* <|@ step (<|@ step)*`",
            );
        if let Some(span) = continuation_span {
            diagnostic = diagnostic.with_secondary_label(
                span,
                "a recurrent suffix may only use `?|>` guards before its first `<|@` step",
            );
        }
        self.diagnostics.push(diagnostic);
    }

    fn emit_illegal_recurrence_continuation(
        &mut self,
        start_span: SourceSpan,
        span: SourceSpan,
        label: &'static str,
    ) {
        self.diagnostics.push(
            Diagnostic::error(
                "a recurrent pipe suffix may only use `?|>` guards before its first `<|@`, then only `<|@` stages, and must reach pipe end",
            )
            .with_code(code("illegal-recurrence-continuation"))
            .with_primary_label(span, label)
            .with_secondary_label(start_span, "recurrent suffix started here")
            .with_note(
                "keep recurrence as one trailing `@|> ... <|@ ...` suffix so the scheduler-node handoff stays explicit",
            ),
        );
    }

    fn lower_record_expr(&mut self, record: &syn::RecordExpr) -> RecordExpr {
        let mut seen_fields = HashMap::<String, SourceSpan>::with_capacity(record.fields.len());
        RecordExpr {
            fields: record
                .fields
                .iter()
                .map(|field| {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate record field `{}`",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }
                    let value = field
                        .value
                        .as_ref()
                        .map(|value| self.lower_expr(value))
                        .unwrap_or_else(|| {
                            self.alloc_expr(Expr {
                                span: field.label.span,
                                kind: ExprKind::Name(TermReference::unresolved(self.make_path(&[
                                    self.make_name(&field.label.text, field.label.span),
                                ]))),
                            })
                        });
                    RecordExprField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        value,
                        surface: if field.value.is_some() {
                            RecordFieldSurface::Explicit
                        } else {
                            RecordFieldSurface::Shorthand
                        },
                    }
                })
                .collect(),
        }
    }

    fn lower_map_expr(&mut self, map: &syn::MapExpr) -> MapExpr {
        let mut seen_keys = Vec::<(&syn::Expr, SourceSpan)>::with_capacity(map.entries.len());
        let mut entries = Vec::with_capacity(map.entries.len());
        for entry in &map.entries {
            // This slice keeps duplicate-key checking purely structural so later typed equality
            // semantics can widen it without rewriting the literal surface.
            if let Some((_, previous_span)) = seen_keys
                .iter()
                .find(|(previous_key, _)| surface_exprs_equal(previous_key, &entry.key))
            {
                self.diagnostics.push(
                    Diagnostic::error("duplicate map key")
                        .with_code(code("duplicate-map-key"))
                        .with_primary_label(entry.key.span, "this map key is repeated")
                        .with_secondary_label(*previous_span, "previous map key here"),
                );
            }
            seen_keys.push((&entry.key, entry.key.span));
            let key = self.lower_expr(&entry.key);
            let value = self.lower_expr(&entry.value);
            entries.push(MapExprEntry {
                span: entry.span,
                key,
                value,
            });
        }
        MapExpr { entries }
    }

    fn lower_record_expr_as_expr(&mut self, record: &syn::RecordExpr) -> ExprId {
        let span = record.span;
        let record = self.lower_record_expr(record);
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Record(record),
        })
    }

    fn lower_pattern(&mut self, pattern: &syn::Pattern) -> PatternId {
        if let syn::PatternKind::Group(inner) = &pattern.kind {
            return self.lower_pattern(inner);
        }
        let kind = match &pattern.kind {
            syn::PatternKind::Wildcard => PatternKind::Wildcard,
            syn::PatternKind::Name(name) => self.lower_name_pattern(name),
            syn::PatternKind::Integer(integer) => PatternKind::Integer(IntegerLiteral {
                raw: integer.raw.clone().into_boxed_str(),
            }),
            syn::PatternKind::Text(text) => {
                if text.has_interpolation() {
                    self.emit_error(
                        text.span,
                        "pattern text literals cannot contain interpolation",
                        code("interpolated-pattern-text"),
                    );
                }
                PatternKind::Text(self.lower_text_literal(text))
            }
            syn::PatternKind::Group(_) => unreachable!("group patterns are handled above"),
            syn::PatternKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_pattern(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            pattern.span,
                            "tuple patterns require at least two elements",
                            code("short-tuple-pattern"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_pattern(pattern.span),
                            self.placeholder_pattern(pattern.span),
                            Vec::new(),
                        )
                    }
                };
                PatternKind::Tuple(elements)
            }
            syn::PatternKind::List { elements, rest } => PatternKind::List {
                elements: elements
                    .iter()
                    .map(|element| self.lower_pattern(element))
                    .collect(),
                rest: rest.as_deref().map(|rest| self.lower_pattern(rest)),
            },
            syn::PatternKind::Record(fields) => {
                let mut seen_fields = HashMap::<String, SourceSpan>::with_capacity(fields.len());
                let mut lowered_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate field `{}` in record pattern",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record pattern entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }

                    // Build the leaf pattern (innermost binding).
                    let leaf_pat = field
                        .pattern
                        .as_ref()
                        .map(|pattern| self.lower_pattern(pattern))
                        .unwrap_or_else(|| {
                            // Shorthand: bind the leaf segment name.
                            let leaf_ident = field
                                .label_path
                                .last()
                                .unwrap_or(&field.label);
                            let binding_name =
                                self.make_name(&leaf_ident.text, leaf_ident.span);
                            let binding = self.alloc_binding(Binding {
                                span: leaf_ident.span,
                                name: binding_name.clone(),
                                kind: BindingKind::Pattern,
                            });
                            self.alloc_pattern(Pattern {
                                span: leaf_ident.span,
                                kind: PatternKind::Binding(BindingPattern {
                                    binding,
                                    name: binding_name,
                                }),
                            })
                        });

                    // Wrap in nested record patterns for dotted paths:
                    // { a.b.c } → { a: { b: { c } } }
                    let pat = if field.label_path.is_empty() {
                        leaf_pat
                    } else {
                        // Build from inside out: start with leaf_pat, wrap in
                        // record patterns for each path segment (reversed).
                        let mut current = leaf_pat;
                        for seg in field.label_path.iter().rev() {
                            let seg_name = self.make_name(&seg.text, seg.span);
                            let inner_field = RecordPatternField {
                                span: seg.span,
                                label: seg_name,
                                pattern: current,
                                surface: RecordFieldSurface::Explicit,
                            };
                            current = self.alloc_pattern(Pattern {
                                span: seg.span,
                                kind: PatternKind::Record(vec![inner_field]),
                            });
                        }
                        current
                    };

                    lowered_fields.push(RecordPatternField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        pattern: pat,
                        surface: if field.pattern.is_some() || !field.label_path.is_empty() {
                            RecordFieldSurface::Explicit
                        } else {
                            RecordFieldSurface::Shorthand
                        },
                    });
                }
                PatternKind::Record(lowered_fields)
            }
            syn::PatternKind::Apply { callee, arguments } => PatternKind::Constructor {
                callee: self.pattern_callee_from_pattern(callee, pattern.span),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_pattern(argument))
                    .collect(),
            },
        };
        self.alloc_pattern(Pattern {
            span: pattern.span,
            kind,
        })
    }

    fn lower_expr_pattern(&mut self, expr: &syn::Expr) -> PatternId {
        if let syn::ExprKind::Group(inner) = &expr.kind {
            return self.lower_expr_pattern(inner);
        }
        let kind = match &expr.kind {
            syn::ExprKind::Name(name) => self.lower_name_pattern(name),
            syn::ExprKind::Integer(integer) => PatternKind::Integer(IntegerLiteral {
                raw: integer.raw.clone().into_boxed_str(),
            }),
            syn::ExprKind::Text(text) => {
                if text.has_interpolation() {
                    self.emit_error(
                        text.span,
                        "pattern text literals cannot contain interpolation",
                        code("interpolated-pattern-text"),
                    );
                }
                PatternKind::Text(self.lower_text_literal(text))
            }
            syn::ExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_expr_pattern(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            expr.span,
                            "tuple patterns require at least two elements",
                            code("short-tuple-pattern"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_pattern(expr.span),
                            self.placeholder_pattern(expr.span),
                            Vec::new(),
                        )
                    }
                };
                PatternKind::Tuple(elements)
            }
            syn::ExprKind::List(elements) => PatternKind::List {
                elements: elements
                    .iter()
                    .map(|element| self.lower_expr_pattern(element))
                    .collect(),
                rest: None,
            },
            syn::ExprKind::Record(record) => {
                let mut seen_fields =
                    HashMap::<String, SourceSpan>::with_capacity(record.fields.len());
                let mut lowered_fields = Vec::with_capacity(record.fields.len());
                for field in &record.fields {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate field `{}` in record pattern",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record pattern entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }
                    let pat = field
                        .value
                        .as_ref()
                        .map(|value| self.lower_expr_pattern(value))
                        .unwrap_or_else(|| {
                            let binding_name = self.make_name(&field.label.text, field.label.span);
                            let binding = self.alloc_binding(Binding {
                                span: field.label.span,
                                name: binding_name.clone(),
                                kind: BindingKind::Pattern,
                            });
                            self.alloc_pattern(Pattern {
                                span: field.label.span,
                                kind: PatternKind::Binding(BindingPattern {
                                    binding,
                                    name: binding_name,
                                }),
                            })
                        });
                    lowered_fields.push(RecordPatternField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        pattern: pat,
                        surface: if field.value.is_some() {
                            RecordFieldSurface::Explicit
                        } else {
                            RecordFieldSurface::Shorthand
                        },
                    });
                }
                PatternKind::Record(lowered_fields)
            }
            syn::ExprKind::Apply { callee, arguments } => PatternKind::Constructor {
                callee: self.pattern_callee_from_expr(callee, expr.span),
                arguments: arguments
                    .iter()
                    .map(|argument| self.lower_expr_pattern(argument))
                    .collect(),
            },
            syn::ExprKind::Group(_) => unreachable!("group expressions are handled above"),
            _ => {
                self.emit_error(
                    expr.span,
                    "markup `pattern={...}` expressions must stay within the pattern subset",
                    code("invalid-pattern-expr"),
                );
                PatternKind::Wildcard
            }
        };
        self.alloc_pattern(Pattern {
            span: expr.span,
            kind,
        })
    }

    fn lower_name_pattern(&mut self, name: &syn::Identifier) -> PatternKind {
        if name.is_uppercase_initial() {
            PatternKind::UnresolvedName(TermReference::unresolved(
                self.make_path(&[self.make_name(&name.text, name.span)]),
            ))
        } else {
            let binding_name = self.make_name(&name.text, name.span);
            let binding = self.alloc_binding(Binding {
                span: name.span,
                name: binding_name.clone(),
                kind: BindingKind::Pattern,
            });
            PatternKind::Binding(BindingPattern {
                binding,
                name: binding_name,
            })
        }
    }

    fn pattern_callee_from_pattern(
        &mut self,
        callee: &syn::Pattern,
        span: SourceSpan,
    ) -> TermReference {
        match &callee.kind {
            syn::PatternKind::Name(name) => {
                TermReference::unresolved(self.make_path(&[self.make_name(&name.text, name.span)]))
            }
            syn::PatternKind::Group(inner) => self.pattern_callee_from_pattern(inner, span),
            _ => {
                self.emit_error(
                    span,
                    "pattern constructor heads must be names",
                    code("invalid-pattern-callee"),
                );
                TermReference::unresolved(self.make_path(&[self.make_name("invalid", span)]))
            }
        }
    }

    fn pattern_callee_from_expr(&mut self, callee: &syn::Expr, span: SourceSpan) -> TermReference {
        match &callee.kind {
            syn::ExprKind::Name(name) => {
                TermReference::unresolved(self.make_path(&[self.make_name(&name.text, name.span)]))
            }
            syn::ExprKind::Group(inner) => self.pattern_callee_from_expr(inner, span),
            _ => {
                self.emit_error(
                    span,
                    "pattern constructor heads must be names",
                    code("invalid-pattern-callee"),
                );
                TermReference::unresolved(self.make_path(&[self.make_name("invalid", span)]))
            }
        }
    }

    fn lower_type_expr(&mut self, ty: &syn::TypeExpr) -> TypeId {
        match &ty.kind {
            syn::TypeExprKind::Group(inner) => self.lower_type_expr(inner),
            syn::TypeExprKind::Name(name) => {
                let path = self.make_path(&[self.make_name(&name.text, name.span)]);
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Name(TypeReference {
                        path,
                        resolution: ResolutionState::Unresolved,
                    }),
                })
            }
            syn::TypeExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.lower_type_expr(element))
                    .collect::<Vec<_>>();
                let elements = match AtLeastTwo::from_vec(elements) {
                    Ok(elements) => elements,
                    Err(_) => {
                        self.emit_error(
                            ty.span,
                            "tuple types require at least two elements",
                            code("short-tuple-type"),
                        );
                        AtLeastTwo::new(
                            self.placeholder_type(ty.span),
                            self.placeholder_type(ty.span),
                            Vec::new(),
                        )
                    }
                };
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Tuple(elements),
                })
            }
            syn::TypeExprKind::Record(fields) => {
                let mut seen_fields = HashMap::<String, SourceSpan>::with_capacity(fields.len());
                let mut lowered_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    if let Some(previous_span) =
                        seen_fields.insert(field.label.text.clone(), field.label.span)
                    {
                        self.diagnostics.push(
                            Diagnostic::error(format!(
                                "duplicate field `{}` in record type",
                                field.label.text
                            ))
                            .with_code(code("duplicate-record-field"))
                            .with_primary_label(
                                field.label.span,
                                "this field label repeats an earlier record type entry",
                            )
                            .with_secondary_label(
                                previous_span,
                                "previous field with the same label here",
                            ),
                        );
                    }
                    let field_ty = field
                        .ty
                        .as_ref()
                        .map(|field_ty| self.lower_type_expr(field_ty))
                        .unwrap_or_else(|| {
                            self.emit_error(
                                field.span,
                                "record type field is missing a type",
                                code("missing-record-field-type"),
                            );
                            self.placeholder_type(field.span)
                        });
                    lowered_fields.push(TypeField {
                        span: field.span,
                        label: self.make_name(&field.label.text, field.label.span),
                        ty: field_ty,
                    });
                }
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Record(lowered_fields),
                })
            }
            syn::TypeExprKind::Arrow { parameter, result } => {
                let parameter = self.lower_type_expr(parameter);
                let result = self.lower_type_expr(result);
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Arrow { parameter, result },
                })
            }
            syn::TypeExprKind::Apply { callee, arguments } => {
                if let Some(transform) = self.lower_record_row_transform(ty, callee, arguments) {
                    return transform;
                }
                let callee = self.lower_type_expr(callee);
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower_type_expr(argument))
                    .collect::<Vec<_>>();
                let arguments = match crate::NonEmpty::from_vec(arguments) {
                    Ok(arguments) => arguments,
                    Err(_) => {
                        self.emit_error(
                            ty.span,
                            "type application requires at least one argument",
                            code("empty-type-args"),
                        );
                        crate::NonEmpty::new(self.placeholder_type(ty.span), Vec::new())
                    }
                };
                self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Apply { callee, arguments },
                })
            }
        }
    }

    fn lower_record_row_transform(
        &mut self,
        ty: &syn::TypeExpr,
        callee: &syn::TypeExpr,
        arguments: &[syn::TypeExpr],
    ) -> Option<TypeId> {
        let syn::TypeExprKind::Name(name) = &callee.kind else {
            return None;
        };
        let transform = match name.text.as_str() {
            "Pick" => Some("Pick"),
            "Omit" => Some("Omit"),
            "Optional" => Some("Optional"),
            "Required" => Some("Required"),
            "Defaulted" => Some("Defaulted"),
            "Rename" => Some("Rename"),
            _ => None,
        }?;
        if arguments.len() != 2 {
            self.emit_error(
                ty.span,
                format!("record row transform `{transform}` expects exactly two arguments"),
                code("invalid-record-row-transform"),
            );
            return Some(self.placeholder_type(ty.span));
        }
        let source = self.lower_type_expr(&arguments[1]);
        let transform = match transform {
            "Pick" => RecordRowTransform::Pick(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Pick",
            )),
            "Omit" => RecordRowTransform::Omit(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Omit",
            )),
            "Optional" => RecordRowTransform::Optional(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Optional",
            )),
            "Required" => RecordRowTransform::Required(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Required",
            )),
            "Defaulted" => RecordRowTransform::Defaulted(self.lower_record_row_labels(
                ty.span,
                &arguments[0],
                "Defaulted",
            )),
            "Rename" => {
                RecordRowTransform::Rename(self.lower_record_row_renames(ty.span, &arguments[0]))
            }
            _ => unreachable!("checked transform names should stay exhaustive"),
        };
        Some(self.alloc_type(TypeNode {
            span: ty.span,
            kind: TypeKind::RecordTransform { transform, source },
        }))
    }

    fn lower_record_row_labels(
        &mut self,
        transform_span: SourceSpan,
        labels: &syn::TypeExpr,
        transform_name: &str,
    ) -> Vec<Name> {
        match &labels.kind {
            syn::TypeExprKind::Group(inner) => {
                self.lower_record_row_labels(transform_span, inner, transform_name)
            }
            syn::TypeExprKind::Name(name) => vec![self.make_name(&name.text, name.span)],
            syn::TypeExprKind::Tuple(elements) => elements
                .iter()
                .flat_map(|element| {
                    self.lower_record_row_labels(transform_span, element, transform_name)
                })
                .collect(),
            _ => {
                self.emit_error(
                    labels.span,
                    format!(
                        "record row transform `{transform_name}` expects a tuple of field labels"
                    ),
                    code("invalid-record-row-transform"),
                );
                vec![self.make_name("invalid", transform_span)]
            }
        }
    }

    fn lower_record_row_renames(
        &mut self,
        transform_span: SourceSpan,
        mapping: &syn::TypeExpr,
    ) -> Vec<RecordRowRename> {
        let syn::TypeExprKind::Record(fields) = &mapping.kind else {
            self.emit_error(
                mapping.span,
                "record row transform `Rename` expects a record-shaped mapping",
                code("invalid-record-row-transform"),
            );
            return vec![RecordRowRename {
                span: transform_span,
                from: self.make_name("invalid", transform_span),
                to: self.make_name("invalid", transform_span),
            }];
        };
        let mut renames = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(target) = field.ty.as_ref() else {
                self.emit_error(
                    field.span,
                    "rename mappings must use `old: new` field pairs",
                    code("invalid-record-row-transform"),
                );
                continue;
            };
            let target = match &target.kind {
                syn::TypeExprKind::Group(inner) => inner.as_ref(),
                _ => target,
            };
            let syn::TypeExprKind::Name(target_name) = &target.kind else {
                self.emit_error(
                    target.span,
                    "rename mapping values must be field labels",
                    code("invalid-record-row-transform"),
                );
                continue;
            };
            renames.push(RecordRowRename {
                span: field.span,
                from: self.make_name(&field.label.text, field.label.span),
                to: self.make_name(&target_name.text, target_name.span),
            });
        }
        if renames.is_empty() {
            renames.push(RecordRowRename {
                span: transform_span,
                from: self.make_name("invalid", transform_span),
                to: self.make_name("invalid", transform_span),
            });
        }
        renames
    }

    fn lower_markup_node(
        &mut self,
        node: &syn::MarkupNode,
        placement: MarkupPlacement,
    ) -> LoweredMarkup {
        let tag_name = match node.name.segments.as_slice() {
            [segment] => Some(segment.text.as_str()),
            _ => None,
        };
        match tag_name {
            Some("show") => {
                let control = ControlNode::Show(self.lower_show_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("each") => {
                let control = ControlNode::Each(self.lower_each_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("match") => {
                let control = ControlNode::Match(self.lower_match_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("fragment") => {
                let control = ControlNode::Fragment(self.lower_fragment_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("with") => {
                let control = ControlNode::With(self.lower_with_control(node));
                LoweredMarkup::Renderable(self.wrap_control(control))
            }
            Some("empty") => {
                let control = ControlNode::Empty(self.lower_empty_control(node));
                let control = self.alloc_control(control);
                match placement {
                    MarkupPlacement::EachEmpty => LoweredMarkup::Empty(control),
                    _ => LoweredMarkup::Renderable(self.invalid_branch_control(
                        control,
                        node.span,
                        "`<empty>` is only valid directly under `<each>`",
                    )),
                }
            }
            Some("case") => {
                let control = ControlNode::Case(self.lower_case_control(node));
                let control = self.alloc_control(control);
                match placement {
                    MarkupPlacement::MatchCase => LoweredMarkup::Case(control),
                    _ => LoweredMarkup::Renderable(self.invalid_branch_control(
                        control,
                        node.span,
                        "`<case>` is only valid directly under `<match>`",
                    )),
                }
            }
            _ => LoweredMarkup::Renderable(self.lower_markup_element(node)),
        }
    }

    fn lower_markup_element(&mut self, node: &syn::MarkupNode) -> MarkupNodeId {
        let attributes = node
            .attributes
            .iter()
            .map(|attribute| MarkupAttribute {
                span: attribute.span,
                name: self.make_name(&attribute.name.text, attribute.name.span),
                value: match &attribute.value {
                    Some(syn::MarkupAttributeValue::Text(text)) => {
                        MarkupAttributeValue::Text(self.lower_text_literal(text))
                    }
                    Some(syn::MarkupAttributeValue::Expr(expr)) => {
                        MarkupAttributeValue::Expr(self.lower_expr(expr))
                    }
                    Some(syn::MarkupAttributeValue::Pattern(_)) => {
                        self.emit_error(
                            attribute.span,
                            "only `<case pattern={...}>` accepts pattern-valued markup attributes",
                            code("invalid-markup-pattern-attr"),
                        );
                        MarkupAttributeValue::Expr(self.placeholder_expr(attribute.span))
                    }
                    None => MarkupAttributeValue::ImplicitTrue,
                },
            })
            .collect();
        let children = node
            .children
            .iter()
            .map(|child| {
                let lowered = self.lower_markup_node(child, MarkupPlacement::Renderable);
                self.renderable_markup(lowered, child.span, "ordinary markup element")
            })
            .collect();
        let name_segments = node
            .name
            .segments
            .iter()
            .map(|segment| self.make_name(&segment.text, segment.span))
            .collect::<Vec<_>>();
        let name = self.make_path(&name_segments);
        let close_name = node.close_name.as_ref().map(|close_name| {
            self.make_path(
                &close_name
                    .segments
                    .iter()
                    .map(|segment| self.make_name(&segment.text, segment.span))
                    .collect::<Vec<_>>(),
            )
        });
        self.alloc_markup_node(MarkupNode {
            span: node.span,
            kind: MarkupNodeKind::Element(MarkupElement {
                name,
                attributes,
                children,
                close_name,
                self_closing: node.self_closing,
            }),
        })
    }

    fn lower_show_control(&mut self, node: &syn::MarkupNode) -> ShowControl {
        ShowControl {
            span: node.span,
            when: self.required_markup_expr_attr(node, "when"),
            keep_mounted: self.optional_markup_expr_attr(node, "keepMounted"),
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<show>`",
            ),
        }
    }

    fn lower_each_control(&mut self, node: &syn::MarkupNode) -> EachControl {
        let binding = self.required_markup_binder_attr(node, "as", BindingKind::MarkupEach);
        let mut children = Vec::new();
        let mut empty = None;
        for child in &node.children {
            match self.lower_markup_node(child, MarkupPlacement::EachEmpty) {
                LoweredMarkup::Renderable(node_id) => children.push(node_id),
                LoweredMarkup::Empty(control_id) => {
                    if empty.is_some() {
                        self.emit_error(
                            child.span,
                            "`<each>` may contain at most one `<empty>` branch",
                            code("duplicate-empty-branch"),
                        );
                    } else {
                        empty = Some(control_id);
                    }
                }
                LoweredMarkup::Case(control_id) => {
                    children.push(self.invalid_branch_control(
                        control_id,
                        child.span,
                        "`<case>` is only valid directly under `<match>`",
                    ));
                }
            }
        }
        EachControl {
            span: node.span,
            collection: self.required_markup_expr_attr(node, "of"),
            binding,
            key: self.optional_markup_expr_attr(node, "key"),
            children,
            empty,
        }
    }

    fn lower_match_control(&mut self, node: &syn::MarkupNode) -> MatchControl {
        let mut cases = Vec::new();
        for child in &node.children {
            match self.lower_markup_node(child, MarkupPlacement::MatchCase) {
                LoweredMarkup::Case(control_id) => cases.push(control_id),
                LoweredMarkup::Renderable(_) | LoweredMarkup::Empty(_) => {
                    self.emit_error(
                        child.span,
                        "`<match>` children must be `<case>` branches",
                        code("invalid-match-child"),
                    );
                }
            }
        }
        if cases.is_empty() {
            self.emit_error(
                node.span,
                "`<match>` requires at least one `<case>` branch",
                code("missing-match-case"),
            );
            let wildcard = self.alloc_pattern(Pattern {
                span: node.span,
                kind: PatternKind::Wildcard,
            });
            cases.push(self.alloc_control(ControlNode::Case(CaseControl {
                span: node.span,
                pattern: wildcard,
                children: Vec::new(),
            })));
        }
        MatchControl {
            span: node.span,
            scrutinee: self.required_markup_expr_attr(node, "on"),
            cases: crate::NonEmpty::from_vec(cases)
                .expect("match fallback should provide one case"),
        }
    }

    fn lower_fragment_control(&mut self, node: &syn::MarkupNode) -> FragmentControl {
        FragmentControl {
            span: node.span,
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<fragment>`",
            ),
        }
    }

    fn lower_with_control(&mut self, node: &syn::MarkupNode) -> WithControl {
        WithControl {
            span: node.span,
            value: self.required_markup_expr_attr(node, "value"),
            binding: self.required_markup_binder_attr(node, "as", BindingKind::MarkupWith),
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<with>`",
            ),
        }
    }

    fn lower_empty_control(&mut self, node: &syn::MarkupNode) -> EmptyControl {
        EmptyControl {
            span: node.span,
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<empty>`",
            ),
        }
    }

    fn lower_case_control(&mut self, node: &syn::MarkupNode) -> CaseControl {
        let pattern = self
            .required_markup_pattern_attr(node, "pattern")
            .unwrap_or_else(|| self.placeholder_pattern(node.span));
        CaseControl {
            span: node.span,
            pattern,
            children: self.lower_renderable_children(
                &node.children,
                MarkupPlacement::Renderable,
                "`<case>`",
            ),
        }
    }

    fn lower_renderable_children(
        &mut self,
        children: &[syn::MarkupNode],
        placement: MarkupPlacement,
        parent: &str,
    ) -> Vec<MarkupNodeId> {
        children
            .iter()
            .map(|child| {
                let lowered = self.lower_markup_node(child, placement);
                self.renderable_markup(lowered, child.span, parent)
            })
            .collect()
    }

    fn renderable_markup(
        &mut self,
        lowered: LoweredMarkup,
        span: SourceSpan,
        parent: &str,
    ) -> MarkupNodeId {
        match lowered {
            LoweredMarkup::Renderable(id) => id,
            LoweredMarkup::Empty(control_id) => self.invalid_branch_control(
                control_id,
                span,
                format!("`<empty>` cannot render directly under {parent}"),
            ),
            LoweredMarkup::Case(control_id) => self.invalid_branch_control(
                control_id,
                span,
                format!("`<case>` cannot render directly under {parent}"),
            ),
        }
    }

    fn invalid_branch_control(
        &mut self,
        control_id: ControlNodeId,
        span: SourceSpan,
        message: impl Into<String>,
    ) -> MarkupNodeId {
        self.emit_error(span, message, code("misplaced-control-branch"));
        let children = match self
            .module
            .control_nodes()
            .get(control_id)
            .expect("misplaced control branch should exist")
            .clone()
        {
            ControlNode::Empty(node) => node.children,
            ControlNode::Case(node) => node.children,
            _ => Vec::new(),
        };
        let control = self.alloc_control(ControlNode::Fragment(FragmentControl { span, children }));
        self.alloc_markup_node(MarkupNode {
            span,
            kind: MarkupNodeKind::Control(control),
        })
    }

    fn required_markup_expr_attr(&mut self, node: &syn::MarkupNode, name: &str) -> ExprId {
        self.required_markup_attr(node, name)
            .as_ref()
            .map(|expr| self.lower_expr(expr))
            .unwrap_or_else(|| self.placeholder_expr(node.span))
    }

    fn optional_markup_expr_attr(&mut self, node: &syn::MarkupNode, name: &str) -> Option<ExprId> {
        self.find_markup_attr(node, name)
            .and_then(|attribute| match &attribute.value {
                Some(syn::MarkupAttributeValue::Expr(expr)) => Some(self.lower_expr(expr)),
                Some(syn::MarkupAttributeValue::Text(_)) => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects an expression"),
                        code("invalid-control-attr"),
                    );
                    Some(self.placeholder_expr(attribute.span))
                }
                Some(syn::MarkupAttributeValue::Pattern(_)) => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects an expression"),
                        code("invalid-control-attr"),
                    );
                    Some(self.placeholder_expr(attribute.span))
                }
                None => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects an expression"),
                        code("invalid-control-attr"),
                    );
                    Some(self.placeholder_expr(attribute.span))
                }
            })
    }

    fn required_markup_binder_attr(
        &mut self,
        node: &syn::MarkupNode,
        name: &str,
        kind: BindingKind,
    ) -> BindingId {
        let Some(attribute) = self.find_markup_attr(node, name) else {
            self.emit_error(
                node.span,
                format!("markup control node is missing required `{name}` attribute"),
                code("missing-control-attr"),
            );
            return self.alloc_binding(Binding {
                span: node.span,
                name: self.make_name("invalid", node.span),
                kind,
            });
        };
        match &attribute.value {
            Some(syn::MarkupAttributeValue::Expr(expr)) => match &expr.kind {
                syn::ExprKind::Name(identifier) => self.alloc_binding(Binding {
                    span: identifier.span,
                    name: self.make_name(&identifier.text, identifier.span),
                    kind,
                }),
                syn::ExprKind::Group(inner) => match &inner.kind {
                    syn::ExprKind::Name(identifier) => self.alloc_binding(Binding {
                        span: identifier.span,
                        name: self.make_name(&identifier.text, identifier.span),
                        kind,
                    }),
                    _ => {
                        self.emit_error(
                            attribute.span,
                            format!("attribute `{name}` expects a binder name"),
                            code("invalid-binder-attr"),
                        );
                        self.alloc_binding(Binding {
                            span: attribute.span,
                            name: self.make_name("invalid", attribute.span),
                            kind,
                        })
                    }
                },
                _ => {
                    self.emit_error(
                        attribute.span,
                        format!("attribute `{name}` expects a binder name"),
                        code("invalid-binder-attr"),
                    );
                    self.alloc_binding(Binding {
                        span: attribute.span,
                        name: self.make_name("invalid", attribute.span),
                        kind,
                    })
                }
            },
            Some(syn::MarkupAttributeValue::Pattern(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a binder name"),
                    code("invalid-binder-attr"),
                );
                self.alloc_binding(Binding {
                    span: attribute.span,
                    name: self.make_name("invalid", attribute.span),
                    kind,
                })
            }
            _ => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a binder name"),
                    code("invalid-binder-attr"),
                );
                self.alloc_binding(Binding {
                    span: attribute.span,
                    name: self.make_name("invalid", attribute.span),
                    kind,
                })
            }
        }
    }

    fn required_markup_attr<'node>(
        &mut self,
        node: &'node syn::MarkupNode,
        name: &str,
    ) -> Option<&'node syn::Expr> {
        let Some(attribute) = self.find_markup_attr(node, name) else {
            self.emit_error(
                node.span,
                format!("markup control node is missing required `{name}` attribute"),
                code("missing-control-attr"),
            );
            return None;
        };
        match &attribute.value {
            Some(syn::MarkupAttributeValue::Expr(expr)) => Some(expr),
            Some(syn::MarkupAttributeValue::Text(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects an expression"),
                    code("invalid-control-attr"),
                );
                None
            }
            Some(syn::MarkupAttributeValue::Pattern(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects an expression"),
                    code("invalid-control-attr"),
                );
                None
            }
            None => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects an expression"),
                    code("invalid-control-attr"),
                );
                None
            }
        }
    }

    fn required_markup_pattern_attr(
        &mut self,
        node: &syn::MarkupNode,
        name: &str,
    ) -> Option<PatternId> {
        let Some(attribute) = self.find_markup_attr(node, name) else {
            self.emit_error(
                node.span,
                format!("markup control node is missing required `{name}` attribute"),
                code("missing-control-attr"),
            );
            return None;
        };
        match &attribute.value {
            Some(syn::MarkupAttributeValue::Pattern(pattern)) => Some(self.lower_pattern(pattern)),
            Some(syn::MarkupAttributeValue::Expr(expr)) => Some(self.lower_expr_pattern(expr)),
            Some(syn::MarkupAttributeValue::Text(_)) => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a pattern"),
                    code("invalid-control-attr"),
                );
                None
            }
            None => {
                self.emit_error(
                    attribute.span,
                    format!("attribute `{name}` expects a pattern"),
                    code("invalid-control-attr"),
                );
                None
            }
        }
    }

    fn find_markup_attr<'node>(
        &self,
        node: &'node syn::MarkupNode,
        name: &str,
    ) -> Option<&'node syn::MarkupAttribute> {
        node.attributes
            .iter()
            .find(|attribute| attribute.name.text == name)
    }

    fn lower_projection_path(&mut self, path: &syn::ProjectionPath) -> NamePath {
        let names = path
            .fields
            .iter()
            .map(|field| self.make_name(&field.text, field.span))
            .collect::<Vec<_>>();
        self.make_path(&names)
    }

    fn lower_qualified_name(&mut self, name: &syn::QualifiedName) -> NamePath {
        let segments = name
            .segments
            .iter()
            .map(|segment| self.make_name(&segment.text, segment.span))
            .collect::<Vec<_>>();
        self.make_path(&segments)
    }

    fn required_name(
        &mut self,
        name: Option<&syn::Identifier>,
        span: SourceSpan,
        subject: &str,
    ) -> Name {
        match name {
            Some(name) => self.make_name(&name.text, name.span),
            None => {
                self.emit_error(
                    span,
                    format!("{subject} is missing a name"),
                    code("missing-name"),
                );
                self.make_name("invalid", span)
            }
        }
    }

    fn build_namespaces(&mut self) -> Namespaces {
        let mut namespaces = Namespaces::default();
        let root_ids = self.module.root_items().to_vec();
        for item_id in root_ids {
            let item = self.module.items()[item_id].clone();
            match item {
                Item::Type(item) => {
                    insert_named(
                        &mut namespaces.type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-type-name"),
                        "type",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            insert_named(
                                &mut namespaces.term_items,
                                variant.name.text(),
                                item_id,
                                variant.span,
                                &mut self.diagnostics,
                                code("duplicate-constructor-name"),
                                "constructor",
                            );
                        }
                    }
                }
                Item::Value(item) => {
                    insert_named(
                        &mut namespaces.term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-term-name"),
                        "term",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                }
                Item::Function(item) => {
                    insert_named(
                        &mut namespaces.term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-term-name"),
                        "term",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                }
                Item::Signal(item) => {
                    insert_named(
                        &mut namespaces.term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-term-name"),
                        "term",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                }
                Item::Class(item) => {
                    insert_named(
                        &mut namespaces.type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-type-name"),
                        "type",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        insert_site(
                            &mut namespaces.class_terms,
                            member.name.text(),
                            crate::hir::ClassMemberResolution {
                                class: item_id,
                                member_index,
                            },
                            member.span,
                        );
                    }
                }
                Item::Domain(item) => {
                    insert_named(
                        &mut namespaces.type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-type-name"),
                        "type",
                    );
                    insert_named(
                        &mut namespaces.any_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                        &mut self.diagnostics,
                        code("duplicate-item-name"),
                        "item",
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        if member.kind == DomainMemberKind::Literal
                            && member.name.text().chars().count() >= 2
                        {
                            insert_site(
                                &mut namespaces.literal_suffixes,
                                member.name.text(),
                                LiteralSuffixResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                            continue;
                        }
                        if member.kind == DomainMemberKind::Method {
                            insert_site(
                                &mut namespaces.domain_terms,
                                member.name.text(),
                                DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                        }
                    }
                }
                Item::SourceProviderContract(item) => {
                    if let Some(key) = item.provider.custom_key() {
                        insert_named(
                            &mut namespaces.provider_contracts,
                            key,
                            item_id,
                            item.header.span,
                            &mut self.diagnostics,
                            code("duplicate-source-provider-contract"),
                            "provider contract",
                        );
                    }
                }
                Item::Use(item) => self.register_use_item(&item, &mut namespaces),
                Item::Hoist(item) => self.register_hoist_item(&item, &mut namespaces),
                Item::Export(_) | Item::Instance(_) => {}
            }
        }
        for item_id in self.module.ambient_items().to_vec() {
            let item = self.module.items()[item_id].clone();
            match item {
                Item::Type(item) => {
                    insert_site(
                        &mut namespaces.ambient_type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                    if let TypeItemBody::Sum(variants) = &item.body {
                        for variant in variants.iter() {
                            insert_site(
                                &mut namespaces.ambient_term_items,
                                variant.name.text(),
                                item_id,
                                variant.span,
                            );
                        }
                    }
                }
                Item::Class(item) => {
                    insert_site(
                        &mut namespaces.ambient_type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        insert_site(
                            &mut namespaces.ambient_class_terms,
                            member.name.text(),
                            crate::hir::ClassMemberResolution {
                                class: item_id,
                                member_index,
                            },
                            member.span,
                        );
                    }
                }
                Item::Value(item) => {
                    insert_site(
                        &mut namespaces.ambient_term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                }
                Item::Function(item) => {
                    insert_site(
                        &mut namespaces.ambient_term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                }
                Item::Signal(item) => {
                    insert_site(
                        &mut namespaces.ambient_term_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                }
                Item::Domain(item) => {
                    insert_site(
                        &mut namespaces.ambient_type_items,
                        item.name.text(),
                        item_id,
                        item.header.span,
                    );
                    for (member_index, member) in item.members.iter().enumerate() {
                        if member.kind == DomainMemberKind::Literal
                            && member.name.text().chars().count() >= 2
                        {
                            insert_site(
                                &mut namespaces.literal_suffixes,
                                member.name.text(),
                                LiteralSuffixResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                            continue;
                        }
                        if member.kind == DomainMemberKind::Method {
                            insert_site(
                                &mut namespaces.domain_terms,
                                member.name.text(),
                                DomainMemberResolution {
                                    domain: item_id,
                                    member_index,
                                },
                                member.span,
                            );
                        }
                    }
                }
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
            | Item::Hoist(_) => {}
            }
        }
        namespaces
    }

    fn register_use_item(&mut self, item: &UseItem, namespaces: &mut Namespaces) {
        let module_name = path_text(&item.module);
        for import_id in item.imports.iter() {
            let import = self.module.imports()[*import_id].clone();
            match import.resolution {
                ImportBindingResolution::Resolved => {}
                ImportBindingResolution::UnknownModule => {
                    self.diagnostics.push(
                        Diagnostic::error(format!("unknown import module `{module_name}`"))
                            .with_code(code("unknown-import-module"))
                            .with_primary_label(
                                import.span,
                                "this workspace does not contain the imported module",
                            )
                            .with_secondary_label(item.header.span, "declared by this `use` item"),
                    );
                    continue;
                }
                ImportBindingResolution::MissingExport => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "module `{module_name}` does not export `{}`",
                            import.imported_name.text()
                        ))
                        .with_code(code("unknown-imported-name"))
                        .with_primary_label(
                            import.span,
                            "this imported name is not exported by the target module",
                        )
                        .with_secondary_label(item.header.span, "declared by this `use` item"),
                    );
                    continue;
                }
                ImportBindingResolution::Cycle => continue,
            }

            match import.metadata.clone() {
                ImportBindingMetadata::Value { .. }
                | ImportBindingMetadata::IntrinsicValue { .. }
                | ImportBindingMetadata::OpaqueValue
                | ImportBindingMetadata::AmbientValue { .. }
                | ImportBindingMetadata::BuiltinTerm(_) => insert_site(
                    &mut namespaces.term_imports,
                    import.local_name.text(),
                    *import_id,
                    import.span,
                ),
                ImportBindingMetadata::TypeConstructor { .. }
                | ImportBindingMetadata::BuiltinType(_)
                | ImportBindingMetadata::AmbientType => insert_site(
                    &mut namespaces.type_imports,
                    import.local_name.text(),
                    *import_id,
                    import.span,
                ),
                ImportBindingMetadata::Domain { literal_suffixes, .. } => {
                    insert_site(
                        &mut namespaces.type_imports,
                        import.local_name.text(),
                        *import_id,
                        import.span,
                    );
                    if !literal_suffixes.is_empty() {
                        self.register_imported_domain_literal_suffixes(
                            &import.local_name,
                            import.span,
                            &literal_suffixes,
                            &mut namespaces.literal_suffixes,
                        );
                    }
                }
                ImportBindingMetadata::Bundle(_) | ImportBindingMetadata::InstanceMember { .. } => {}
                ImportBindingMetadata::Unknown => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "import `{}` from `{module_name}` resolved without compiler-known metadata",
                            import.imported_name.text()
                        ))
                            .with_code(code("invalid-import-resolution"))
                            .with_primary_label(
                                import.span,
                                "resolved imports must carry explicit metadata before name registration",
                            )
                            .with_secondary_label(item.header.span, "declared by this `use` item"),
                    );
                    continue;
                }
            }
        }
    }

    fn register_hoist_item(&mut self, item: &HoistItem, namespaces: &mut Namespaces) {
        use crate::exports::ExportedNameKind;

        let module_name = path_text(&item.module);
        let module_key = module_name.clone();
        namespaces.hoisted_module_paths.insert(module_key);
        let module_segments = item.module.segments().iter().map(Name::text).collect::<Vec<_>>();
        let module_resolution = self.resolver.resolve(&module_segments);

        let ImportModuleResolution::Resolved(ref exports) = module_resolution else {
            if matches!(module_resolution, ImportModuleResolution::Missing) {
                self.diagnostics.push(
                    Diagnostic::error(format!("unknown hoist module `{module_name}`"))
                        .with_code(code("unknown-hoist-module"))
                        .with_primary_label(
                            item.header.span,
                            "this workspace does not contain the hoisted module",
                        ),
                );
            }
            return;
        };

        for exported in exports.names.iter() {
            // Apply kind filters — if no filters specified, accept all.
            if !item.kind_filters.is_empty() {
                let kind_matches = item.kind_filters.iter().any(|f| match (f, &exported.kind) {
                    (HoistKindFilter::Func, ExportedNameKind::Function) => true,
                    (HoistKindFilter::Value, ExportedNameKind::Value) => true,
                    (HoistKindFilter::Signal, ExportedNameKind::Signal) => true,
                    (HoistKindFilter::Type, ExportedNameKind::Type) => true,
                    (HoistKindFilter::Domain, ExportedNameKind::Domain) => true,
                    (HoistKindFilter::Class, ExportedNameKind::Class) => true,
                    _ => false,
                });
                if !kind_matches {
                    continue;
                }
            }

            // Apply hiding list.
            if item.hiding.iter().any(|h| h.text() == exported.name) {
                continue;
            }

            let imported_name = self.make_name(&exported.name, item.header.span);
            let (resolution, metadata, callable_type, deprecation) =
                self.resolve_import_binding(&module_name, &imported_name, &module_resolution);

            if !matches!(resolution, ImportBindingResolution::Resolved) {
                continue;
            }

            let import_id = self.alloc_import(ImportBinding {
                span: item.header.span,
                imported_name: imported_name.clone(),
                local_name: imported_name.clone(),
                resolution,
                metadata: metadata.clone(),
                callable_type,
                deprecation,
            });

            match &metadata {
                ImportBindingMetadata::TypeConstructor { .. }
                | ImportBindingMetadata::BuiltinType(_)
                | ImportBindingMetadata::AmbientType => insert_site(
                    &mut namespaces.hoisted_type_imports,
                    imported_name.text(),
                    import_id,
                    item.header.span,
                ),
                ImportBindingMetadata::Domain { literal_suffixes, .. } => {
                    let suffixes = literal_suffixes.clone();
                    insert_site(
                        &mut namespaces.hoisted_type_imports,
                        imported_name.text(),
                        import_id,
                        item.header.span,
                    );
                    if !suffixes.is_empty() {
                        self.register_imported_domain_literal_suffixes(
                            &imported_name,
                            item.header.span,
                            &suffixes,
                            &mut namespaces.literal_suffixes,
                        );
                    }
                }
                ImportBindingMetadata::Bundle(_) | ImportBindingMetadata::InstanceMember { .. } => {}
                _ => insert_site(
                    &mut namespaces.hoisted_term_imports,
                    imported_name.text(),
                    import_id,
                    item.header.span,
                ),
            }
        }

        // Auto-register instance members from the hoisted module so cross-module
        // class dispatch can find them the same way explicit `use` items do.
        for instance_decl in &exports.instances {
            for member in &instance_decl.members {
                let synthetic_name = format!(
                    "__instance_{}_{}_{}_{}",
                    instance_decl.class_name,
                    member.name,
                    instance_decl.subject,
                    import_value_type_label(&member.ty),
                );
                let name = self.make_name(&synthetic_name, item.header.span);
                self.alloc_import(ImportBinding {
                    span: item.header.span,
                    imported_name: self.make_name(&member.name, item.header.span),
                    local_name: name.clone(),
                    resolution: ImportBindingResolution::Resolved,
                    metadata: ImportBindingMetadata::InstanceMember {
                        class_name: instance_decl.class_name.clone(),
                        member_name: member.name.clone(),
                        subject: instance_decl.subject.clone(),
                        ty: member.ty.clone(),
                    },
                    callable_type: None,
                    deprecation: None,
                });
            }
        }
    }

    /// Register all workspace-level hoist items from other modules.
    ///
    /// Called after `build_namespaces()` processes local `Item::Hoist`
    /// declarations.  Module paths already registered locally are skipped to
    /// prevent double-registration.  Missing modules are silently ignored (no
    /// diagnostic) since they may belong to a different workspace scope.
    fn register_workspace_hoists(&mut self, namespaces: &mut Namespaces) {
        use aivi_base::{FileId, Span};
        use crate::resolver::RawHoistItem;

        let workspace_hoists: Vec<RawHoistItem> = self.resolver.workspace_hoist_items();
        if workspace_hoists.is_empty() {
            return;
        }

        let synthetic_span = SourceSpan::new(FileId::ZERO, Span::from(0..0));

        for raw in workspace_hoists {
            let module_key = raw.module_path.join(".");
            if namespaces.hoisted_module_paths.contains(&module_key) {
                continue;
            }
            namespaces.hoisted_module_paths.insert(module_key.clone());

            let module_segments: Vec<&str> = raw.module_path.iter().map(String::as_str).collect();
            let module_resolution = self.resolver.resolve(&module_segments);
            let crate::resolver::ImportModuleResolution::Resolved(ref exports) = module_resolution
            else {
                continue; // silent — workspace hoists may reference optional modules
            };

            self.register_hoist_exports(
                &module_key,
                exports,
                &raw.kind_filters,
                &raw.hiding,
                synthetic_span,
                namespaces,
            );
        }
    }

    /// Core of hoist registration: given a resolved module's `ExportedNames`,
    /// apply kind filters and hiding, then insert synthetic imports into the
    /// hoisted namespace maps.  Shared by both local `Item::Hoist` and
    /// workspace hoist propagation.
    fn register_hoist_exports(
        &mut self,
        module_name: &str,
        exports: &crate::exports::ExportedNames,
        kind_filters: &[HoistKindFilter],
        hiding: &[impl AsRef<str>],
        span: SourceSpan,
        namespaces: &mut Namespaces,
    ) {
        use crate::exports::ExportedNameKind;
        use crate::resolver::ImportModuleResolution;

        let module_resolution = self.resolver.resolve(
            &module_name.split('.').collect::<Vec<_>>(),
        );
        // Re-use the already-resolved exports passed in; build a local
        // ImportModuleResolution::Resolved so resolve_import_binding works.
        let wrapped = ImportModuleResolution::Resolved(exports.clone());

        for exported in exports.names.iter() {
            if !kind_filters.is_empty() {
                let kind_matches = kind_filters.iter().any(|f| match (f, &exported.kind) {
                    (HoistKindFilter::Func, ExportedNameKind::Function) => true,
                    (HoistKindFilter::Value, ExportedNameKind::Value) => true,
                    (HoistKindFilter::Signal, ExportedNameKind::Signal) => true,
                    (HoistKindFilter::Type, ExportedNameKind::Type) => true,
                    (HoistKindFilter::Domain, ExportedNameKind::Domain) => true,
                    (HoistKindFilter::Class, ExportedNameKind::Class) => true,
                    _ => false,
                });
                if !kind_matches {
                    continue;
                }
            }

            if hiding.iter().any(|h| h.as_ref() == exported.name) {
                continue;
            }

            let imported_name = self.make_name(&exported.name, span);
            let (resolution, metadata, callable_type, deprecation) =
                self.resolve_import_binding(module_name, &imported_name, &wrapped);

            if !matches!(resolution, ImportBindingResolution::Resolved) {
                continue;
            }

            let import_id = self.alloc_import(ImportBinding {
                span,
                imported_name: imported_name.clone(),
                local_name: imported_name.clone(),
                resolution,
                metadata: metadata.clone(),
                callable_type,
                deprecation,
            });

            match &metadata {
                ImportBindingMetadata::TypeConstructor { .. }
                | ImportBindingMetadata::BuiltinType(_)
                | ImportBindingMetadata::AmbientType => insert_site(
                    &mut namespaces.hoisted_type_imports,
                    imported_name.text(),
                    import_id,
                    span,
                ),
                ImportBindingMetadata::Domain { literal_suffixes, .. } => {
                    let suffixes = literal_suffixes.clone();
                    insert_site(
                        &mut namespaces.hoisted_type_imports,
                        imported_name.text(),
                        import_id,
                        span,
                    );
                    if !suffixes.is_empty() {
                        self.register_imported_domain_literal_suffixes(
                            &imported_name,
                            span,
                            &suffixes,
                            &mut namespaces.literal_suffixes,
                        );
                    }
                }
                ImportBindingMetadata::Bundle(_) | ImportBindingMetadata::InstanceMember { .. } => {}
                _ => insert_site(
                    &mut namespaces.hoisted_term_imports,
                    imported_name.text(),
                    import_id,
                    span,
                ),
            }
        }

        // Auto-register instance members for class dispatch.
        for instance_decl in &exports.instances {
            for member in &instance_decl.members {
                let synthetic_name = format!(
                    "__instance_{}_{}_{}_{}",
                    instance_decl.class_name,
                    member.name,
                    instance_decl.subject,
                    import_value_type_label(&member.ty),
                );
                let name = self.make_name(&synthetic_name, span);
                self.alloc_import(ImportBinding {
                    span,
                    imported_name: self.make_name(&member.name, span),
                    local_name: name,
                    resolution: ImportBindingResolution::Resolved,
                    metadata: ImportBindingMetadata::InstanceMember {
                        class_name: instance_decl.class_name.clone(),
                        member_name: member.name.clone(),
                        subject: instance_decl.subject.clone(),
                        ty: member.ty.clone(),
                    },
                    callable_type: None,
                    deprecation: None,
                });
            }
        }
        let _ = module_resolution; // used only for resolve_import_binding
    }

    /// Synthesise a minimal `Item::Domain` stub in the current module so that
    /// `LiteralSuffixResolution.domain` has a valid `ItemId` to point at.
    /// The stub is allocated but NOT appended to `root_items`, so it is invisible
    /// to name resolution and exports — it exists only to satisfy look-up code
    /// that accesses `module.items()[resolution.domain]` at the type-checking
    /// and validation layer.
    fn register_imported_domain_literal_suffixes(
        &mut self,
        domain_name: &Name,
        span: SourceSpan,
        literal_suffixes: &[ImportedDomainLiteralSuffix],
        target: &mut HashMap<String, Vec<NamedSite<LiteralSuffixResolution>>>,
    ) {
        // Allocate a placeholder carrier type — resolved to Unit so validation
        // does not report spurious "unresolved-name" errors on synthetic stubs.
        let stub_path = self.make_path(&[self.make_name("Unit", span)]);
        let stub_type_kind = || TypeKind::Name(TypeReference {
            path: stub_path.clone(),
            resolution: ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Unit)),
        });

        let carrier = self.alloc_type(TypeNode {
            span,
            kind: stub_type_kind(),
        });

        // Build stub members — one per literal suffix, in member_index order.
        // We need indices stable across the allocated slice, so compute the
        // max member_index first and pre-fill with placeholder members at
        // non-suffix positions.
        let max_index = literal_suffixes
            .iter()
            .map(|s| s.member_index)
            .max()
            .unwrap_or(0);
        let mut members: Vec<Option<DomainMember>> = vec![None; max_index + 1];
        for suffix in literal_suffixes {
            let annotation = self.alloc_type(TypeNode {
                span,
                kind: stub_type_kind(),
            });
            members[suffix.member_index] = Some(DomainMember {
                span,
                kind: DomainMemberKind::Literal,
                name: self.make_name(&suffix.name, span),
                annotation,
                parameters: Vec::new(),
                body: None,
            });
        }
        // Fill gaps with placeholder members so member indices are consistent.
        let members: Vec<DomainMember> = members
            .into_iter()
            .enumerate()
            .map(|(i, m)| m.unwrap_or_else(|| {
                let annotation = self.alloc_type(TypeNode {
                    span,
                    kind: stub_type_kind(),
                });
                DomainMember {
                    span,
                    kind: DomainMemberKind::Literal,
                    name: self.make_name(&format!("__stub_{i}"), span),
                    annotation,
                    parameters: Vec::new(),
                    body: None,
                }
            }))
            .collect();

        let domain_stub = Item::Domain(DomainItem {
            header: ItemHeader { span, decorators: Vec::new() },
            name: domain_name.clone(),
            parameters: Vec::new(),
            carrier,
            members: members.clone(),
        });

        let domain_item_id = self.module.alloc_item(domain_stub).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR item arena (imported domain stub)");
            std::process::exit(1);
        });

        // Register each literal suffix entry.
        for (member_index, member) in members.iter().enumerate() {
            if member.kind != DomainMemberKind::Literal {
                continue;
            }
            if member.name.text().starts_with("__stub_") {
                continue;
            }
            insert_site(
                target,
                member.name.text(),
                LiteralSuffixResolution {
                    domain: domain_item_id,
                    member_index,
                },
                span,
            );
        }
    }

    fn resolve_module(&mut self, namespaces: &Namespaces) {
        for item_id in self.module.root_items().to_vec() {
            self.resolve_item(item_id, namespaces, false);
        }
        for item_id in self.module.ambient_items().to_vec() {
            self.resolve_item(item_id, namespaces, true);
        }
    }

    fn normalize_function_signature_annotations(&mut self) {
        let function_ids = self
            .module
            .items()
            .iter()
            .filter_map(|(item_id, item)| matches!(item, Item::Function(_)).then_some(item_id))
            .collect::<Vec<_>>();
        for item_id in function_ids {
            self.normalize_function_signature_annotation(item_id);
        }
    }

    fn normalize_function_signature_annotation(&mut self, item_id: ItemId) {
        let Some((arity, context, annotation, already_split, span)) =
            (match &self.module.items()[item_id] {
                Item::Function(item) => Some((
                    item.parameters.len(),
                    item.context.clone(),
                    item.annotation,
                    item.parameters
                        .iter()
                        .any(|parameter| parameter.annotation.is_some()),
                    item.header.span,
                )),
                _ => None,
            })
        else {
            return;
        };

        let Some(annotation) = annotation else {
            return;
        };
        if arity == 0 || already_split {
            return;
        }

        let Some((constraint_count, parameter_annotations, result_annotation)) =
            self.split_normalized_function_signature_annotation(&context, annotation, arity)
        else {
            self.diagnostics.push(
                Diagnostic::error(
                    "function annotations with parameters must describe the full function signature",
                )
                .with_code(code("invalid-function-signature-annotation"))
                .with_primary_label(
                    span,
                    "expected one parameter type per function parameter before the result type",
                )
                .with_note("use a standalone `type ...` line or a compatible alias such as `type MyFunc = A -> B -> C`"),
            );
            return;
        };

        let Some(Item::Function(function)) = self.module.arenas.items.get_mut(item_id) else {
            return;
        };
        function.context.truncate(constraint_count);
        for (parameter, annotation) in function
            .parameters
            .iter_mut()
            .zip(parameter_annotations.into_iter())
        {
            parameter.annotation = Some(annotation);
        }
        function.annotation = Some(result_annotation);
    }

    fn split_normalized_function_signature_annotation(
        &mut self,
        context: &[TypeId],
        annotation: TypeId,
        arity: usize,
    ) -> Option<(usize, Vec<TypeId>, TypeId)> {
        let maximum_context_parameters = context.len().min(arity);
        for trailing_parameter_count in 0..=maximum_context_parameters {
            let constraint_count = context.len() - trailing_parameter_count;
            let constraints = &context[..constraint_count];
            let leading_parameters = &context[constraint_count..];
            if constraints
                .iter()
                .copied()
                .any(|type_id| !self.is_class_constraint_type(type_id))
            {
                continue;
            }
            let remaining_arity = arity - trailing_parameter_count;
            let mut item_stack = Vec::new();
            let Some((mut parameter_annotations, result_annotation)) = self
                .split_function_signature_annotation(
                    annotation,
                    remaining_arity,
                    &HashMap::new(),
                    &mut item_stack,
                )
            else {
                continue;
            };
            let mut normalized_parameters = leading_parameters.to_vec();
            normalized_parameters.append(&mut parameter_annotations);
            if normalized_parameters.len() == arity {
                return Some((constraint_count, normalized_parameters, result_annotation));
            }
        }
        None
    }

    fn split_function_signature_annotation(
        &mut self,
        type_id: TypeId,
        arity: usize,
        substitutions: &HashMap<TypeParameterId, TypeId>,
        item_stack: &mut Vec<ItemId>,
    ) -> Option<(Vec<TypeId>, TypeId)> {
        if arity == 0 {
            return Some((
                Vec::new(),
                self.instantiate_signature_type(type_id, substitutions)?,
            ));
        }

        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Arrow { parameter, result } => {
                let parameter = self.instantiate_signature_type(parameter, substitutions)?;
                let (mut parameters, result) = self.split_function_signature_annotation(
                    result,
                    arity - 1,
                    substitutions,
                    item_stack,
                )?;
                parameters.insert(0, parameter);
                Some((parameters, result))
            }
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Item(alias_item_id)) => {
                    let alias_item_id = *alias_item_id;
                    if item_stack.contains(&alias_item_id) {
                        return None;
                    }
                    let Item::Type(item) = &self.module.items()[alias_item_id] else {
                        return None;
                    };
                    if !item.parameters.is_empty() {
                        return None;
                    }
                    let TypeItemBody::Alias(alias) = &item.body else {
                        return None;
                    };
                    let alias = *alias;
                    item_stack.push(alias_item_id);
                    let split = self.split_function_signature_annotation(
                        alias,
                        arity,
                        substitutions,
                        item_stack,
                    );
                    let popped = item_stack.pop();
                    debug_assert_eq!(popped, Some(alias_item_id));
                    split
                }
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    let substituted = substitutions.get(parameter).copied()?;
                    self.split_function_signature_annotation(
                        substituted,
                        arity,
                        &HashMap::new(),
                        item_stack,
                    )
                }
                _ => None,
            },
            TypeKind::Apply { callee, arguments } => {
                let TypeKind::Name(reference) = &self.module.types()[callee].kind else {
                    return None;
                };
                let ResolutionState::Resolved(TypeResolution::Item(alias_item_id)) =
                    reference.resolution.as_ref()
                else {
                    return None;
                };
                let alias_item_id = *alias_item_id;
                if item_stack.contains(&alias_item_id) {
                    return None;
                }
                let (parameters, alias) = match &self.module.items()[alias_item_id] {
                    Item::Type(item) => {
                        let TypeItemBody::Alias(alias) = &item.body else {
                            return None;
                        };
                        (item.parameters.clone(), *alias)
                    }
                    _ => return None,
                };
                if parameters.len() != arguments.len() {
                    return None;
                }
                let mut nested_substitutions = HashMap::with_capacity(parameters.len());
                for (parameter, argument) in parameters.iter().zip(arguments.iter()) {
                    nested_substitutions.insert(
                        *parameter,
                        self.instantiate_signature_type(*argument, substitutions)?,
                    );
                }
                item_stack.push(alias_item_id);
                let split = self.split_function_signature_annotation(
                    alias,
                    arity,
                    &nested_substitutions,
                    item_stack,
                );
                let popped = item_stack.pop();
                debug_assert_eq!(popped, Some(alias_item_id));
                split
            }
            TypeKind::Tuple(_) | TypeKind::Record(_) | TypeKind::RecordTransform { .. } => None,
        }
    }

    fn is_class_constraint_type(&self, type_id: TypeId) -> bool {
        match &self.module.types()[type_id].kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                    matches!(self.module.items()[*item_id], Item::Class(_))
                }
                _ => false,
            },
            TypeKind::Apply { callee, .. } => self.is_class_constraint_type(*callee),
            TypeKind::Tuple(_)
            | TypeKind::Record(_)
            | TypeKind::RecordTransform { .. }
            | TypeKind::Arrow { .. } => false,
        }
    }

    fn instantiate_signature_type(
        &mut self,
        type_id: TypeId,
        substitutions: &HashMap<TypeParameterId, TypeId>,
    ) -> Option<TypeId> {
        if substitutions.is_empty() {
            return Some(type_id);
        }

        let ty = self.module.types()[type_id].clone();
        match ty.kind {
            TypeKind::Name(reference) => match reference.resolution.as_ref() {
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter)) => {
                    substitutions.get(parameter).copied().or(Some(type_id))
                }
                _ => Some(type_id),
            },
            TypeKind::Tuple(elements) => {
                let mut changed = false;
                let mut instantiated = Vec::with_capacity(elements.len());
                for element in elements.iter().copied() {
                    let instantiated_element =
                        self.instantiate_signature_type(element, substitutions)?;
                    changed |= instantiated_element != element;
                    instantiated.push(instantiated_element);
                }
                if !changed {
                    return Some(type_id);
                }
                Some(
                    self.alloc_type(TypeNode {
                        span: ty.span,
                        kind: TypeKind::Tuple(
                            AtLeastTwo::from_vec(instantiated)
                                .expect("tuple instantiation preserves arity"),
                        ),
                    }),
                )
            }
            TypeKind::Record(fields) => {
                let mut changed = false;
                let mut instantiated = Vec::with_capacity(fields.len());
                for field in fields {
                    let field_ty = self.instantiate_signature_type(field.ty, substitutions)?;
                    changed |= field_ty != field.ty;
                    instantiated.push(TypeField {
                        span: field.span,
                        label: field.label,
                        ty: field_ty,
                    });
                }
                if !changed {
                    return Some(type_id);
                }
                Some(self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Record(instantiated),
                }))
            }
            TypeKind::RecordTransform { transform, source } => {
                let instantiated_source = self.instantiate_signature_type(source, substitutions)?;
                if instantiated_source == source {
                    return Some(type_id);
                }
                Some(self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::RecordTransform {
                        transform,
                        source: instantiated_source,
                    },
                }))
            }
            TypeKind::Arrow { parameter, result } => {
                let instantiated_parameter =
                    self.instantiate_signature_type(parameter, substitutions)?;
                let instantiated_result = self.instantiate_signature_type(result, substitutions)?;
                if instantiated_parameter == parameter && instantiated_result == result {
                    return Some(type_id);
                }
                Some(self.alloc_type(TypeNode {
                    span: ty.span,
                    kind: TypeKind::Arrow {
                        parameter: instantiated_parameter,
                        result: instantiated_result,
                    },
                }))
            }
            TypeKind::Apply { callee, arguments } => {
                let instantiated_callee = self.instantiate_signature_type(callee, substitutions)?;
                let mut changed = instantiated_callee != callee;
                let mut instantiated_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments.iter().copied() {
                    let argument_ty = self.instantiate_signature_type(argument, substitutions)?;
                    changed |= argument_ty != argument;
                    instantiated_arguments.push(argument_ty);
                }
                if !changed {
                    return Some(type_id);
                }
                Some(self.alloc_type(
                    TypeNode {
                        span: ty.span,
                        kind:
                            TypeKind::Apply {
                                callee: instantiated_callee,
                                arguments:
                                    NonEmpty::from_vec(instantiated_arguments).expect(
                                        "type applications preserve non-empty argument lists",
                                    ),
                            },
                    },
                ))
            }
        }
    }

    fn validate_cluster_normalization(&mut self) {
        let cluster_ids = self
            .module
            .clusters()
            .iter()
            .map(|(cluster_id, _)| cluster_id)
            .collect::<Vec<_>>();
        for cluster_id in cluster_ids {
            self.validate_cluster_ambient_projections(cluster_id);
        }
    }

    fn validate_cluster_ambient_projections(&mut self, cluster_id: crate::ClusterId) {
        let cluster = self.module.clusters()[cluster_id].clone();
        let spine = cluster.normalized_spine();
        for member in spine.apply_arguments() {
            if let Some(span) = self.find_free_ambient_projection(member) {
                self.emit_illegal_cluster_ambient_projection(span, cluster.span);
            }
        }
        if let ApplicativeSpineHead::Expr(finalizer) = spine.pure_head() {
            if let Some(span) = self.find_free_ambient_projection(finalizer) {
                self.emit_illegal_cluster_ambient_projection(span, cluster.span);
            }
        }
    }

    fn emit_illegal_cluster_ambient_projection(
        &mut self,
        span: SourceSpan,
        cluster_span: SourceSpan,
    ) {
        self.diagnostics.push(
            Diagnostic::error(
                "ambient-subject projections such as `.field` are illegal inside `&|>` clusters unless a nested expression provides its own subject",
            )
            .with_code(code("illegal-cluster-ambient-projection"))
            .with_primary_label(
                span,
                "this projection has no ambient subject inside the applicative cluster",
            )
            .with_secondary_label(
                cluster_span,
                "cluster members normalize independently before the finalizer runs",
            )
            .with_note(
                "use an explicit base such as `value.field` or a nested pipe with its own head",
            ),
        );
    }

    fn find_free_ambient_projection(&self, root: ExprId) -> Option<SourceSpan> {
        let mut work = vec![AmbientProjectionWork::Expr {
            expr: root,
            ambient_allowed: false,
        }];
        while let Some(node) = work.pop() {
            match node {
                AmbientProjectionWork::Expr {
                    expr,
                    ambient_allowed,
                } => match &self.module.exprs()[expr].kind {
                    ExprKind::Name(_)
                    | ExprKind::Integer(_)
                    | ExprKind::Float(_)
                    | ExprKind::Decimal(_)
                    | ExprKind::BigInt(_)
                    | ExprKind::SuffixedInteger(_)
                    | ExprKind::AmbientSubject
                    | ExprKind::Regex(_) => {}
                    ExprKind::Text(text) => {
                        for segment in text.segments.iter().rev() {
                            if let TextSegment::Interpolation(interpolation) = segment {
                                work.push(AmbientProjectionWork::Expr {
                                    expr: interpolation.expr,
                                    ambient_allowed,
                                });
                            }
                        }
                    }
                    ExprKind::Tuple(elements) => {
                        for element in elements.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *element,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::List(elements) => {
                        for element in elements.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *element,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Map(map) => {
                        for entry in map.entries.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: entry.value,
                                ambient_allowed,
                            });
                            work.push(AmbientProjectionWork::Expr {
                                expr: entry.key,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Set(elements) => {
                        for element in elements.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *element,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Record(record) => {
                        for field in record.fields.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: field.value,
                                ambient_allowed,
                            });
                        }
                    }
                    ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        ..
                    } if !ambient_allowed => return Some(self.module.exprs()[expr].span),
                    ExprKind::Projection {
                        base: ProjectionBase::Ambient,
                        ..
                    } => {}
                    ExprKind::Projection {
                        base: ProjectionBase::Expr(base),
                        ..
                    } => work.push(AmbientProjectionWork::Expr {
                        expr: *base,
                        ambient_allowed,
                    }),
                    ExprKind::Apply { callee, arguments } => {
                        for argument in arguments.iter().rev() {
                            work.push(AmbientProjectionWork::Expr {
                                expr: *argument,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: *callee,
                            ambient_allowed,
                        });
                    }
                    ExprKind::Unary { expr, .. } => work.push(AmbientProjectionWork::Expr {
                        expr: *expr,
                        ambient_allowed,
                    }),
                    ExprKind::Binary { left, right, .. } => {
                        work.push(AmbientProjectionWork::Expr {
                            expr: *right,
                            ambient_allowed,
                        });
                        work.push(AmbientProjectionWork::Expr {
                            expr: *left,
                            ambient_allowed,
                        });
                    }
                    ExprKind::PatchApply { target, patch } => {
                        for entry in patch.entries.iter().rev() {
                            match entry.instruction.kind {
                                crate::PatchInstructionKind::Replace(expr)
                                | crate::PatchInstructionKind::Store(expr) => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr,
                                        ambient_allowed,
                                    });
                                }
                                crate::PatchInstructionKind::Remove => {}
                            }
                            for segment in entry.selector.segments.iter().rev() {
                                if let crate::PatchSelectorSegment::BracketExpr { expr, .. } =
                                    segment
                                {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed,
                                    });
                                }
                            }
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: *target,
                            ambient_allowed,
                        });
                    }
                    ExprKind::PatchLiteral(patch) => {
                        for entry in patch.entries.iter().rev() {
                            match entry.instruction.kind {
                                crate::PatchInstructionKind::Replace(expr)
                                | crate::PatchInstructionKind::Store(expr) => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr,
                                        ambient_allowed,
                                    });
                                }
                                crate::PatchInstructionKind::Remove => {}
                            }
                            for segment in entry.selector.segments.iter().rev() {
                                if let crate::PatchSelectorSegment::BracketExpr { expr, .. } =
                                    segment
                                {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed,
                                    });
                                }
                            }
                        }
                    }
                    ExprKind::Pipe(pipe) => {
                        for stage in pipe.stages.iter().rev() {
                            match &stage.kind {
                                PipeStageKind::Transform { expr }
                                | PipeStageKind::Gate { expr }
                                | PipeStageKind::Map { expr }
                                | PipeStageKind::Apply { expr }
                                | PipeStageKind::Tap { expr }
                                | PipeStageKind::FanIn { expr }
                                | PipeStageKind::Truthy { expr }
                                | PipeStageKind::Falsy { expr }
                                | PipeStageKind::RecurStart { expr }
                                | PipeStageKind::RecurStep { expr }
                                | PipeStageKind::Validate { expr }
                                | PipeStageKind::Previous { expr }
                                | PipeStageKind::Diff { expr } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed: true,
                                    });
                                }
                                PipeStageKind::Accumulate { seed, step } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *step,
                                        ambient_allowed: true,
                                    });
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *seed,
                                        ambient_allowed: true,
                                    });
                                }
                                PipeStageKind::Case { body, .. } => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *body,
                                        ambient_allowed: true,
                                    });
                                }
                            }
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: pipe.head,
                            ambient_allowed,
                        });
                    }
                    ExprKind::Cluster(_) => {}
                    ExprKind::Markup(node) => work.push(AmbientProjectionWork::Markup {
                        node: *node,
                        ambient_allowed,
                    }),
                },
                AmbientProjectionWork::Markup {
                    node,
                    ambient_allowed,
                } => match &self.module.markup_nodes()[node].kind {
                    MarkupNodeKind::Element(element) => {
                        for child in element.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        for attribute in element.attributes.iter().rev() {
                            match &attribute.value {
                                MarkupAttributeValue::ImplicitTrue => {}
                                MarkupAttributeValue::Expr(expr) => {
                                    work.push(AmbientProjectionWork::Expr {
                                        expr: *expr,
                                        ambient_allowed,
                                    });
                                }
                                MarkupAttributeValue::Text(text) => {
                                    for segment in text.segments.iter().rev() {
                                        if let TextSegment::Interpolation(interpolation) = segment {
                                            work.push(AmbientProjectionWork::Expr {
                                                expr: interpolation.expr,
                                                ambient_allowed,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    MarkupNodeKind::Control(control) => work.push(AmbientProjectionWork::Control {
                        node: *control,
                        ambient_allowed,
                    }),
                },
                AmbientProjectionWork::Control {
                    node,
                    ambient_allowed,
                } => match &self.module.control_nodes()[node] {
                    ControlNode::Show(show) => {
                        for child in show.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        if let Some(keep_mounted) = show.keep_mounted {
                            work.push(AmbientProjectionWork::Expr {
                                expr: keep_mounted,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: show.when,
                            ambient_allowed,
                        });
                    }
                    ControlNode::Each(each) => {
                        if let Some(empty) = each.empty {
                            work.push(AmbientProjectionWork::Control {
                                node: empty,
                                ambient_allowed,
                            });
                        }
                        for child in each.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        if let Some(key) = each.key {
                            work.push(AmbientProjectionWork::Expr {
                                expr: key,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: each.collection,
                            ambient_allowed,
                        });
                    }
                    ControlNode::Empty(empty) => {
                        for child in empty.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                    }
                    ControlNode::Match(match_node) => {
                        for case in match_node.cases.iter().rev() {
                            work.push(AmbientProjectionWork::Control {
                                node: *case,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: match_node.scrutinee,
                            ambient_allowed,
                        });
                    }
                    ControlNode::Case(case) => {
                        for child in case.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                    }
                    ControlNode::Fragment(fragment) => {
                        for child in fragment.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                    }
                    ControlNode::With(with_node) => {
                        for child in with_node.children.iter().rev() {
                            work.push(AmbientProjectionWork::Markup {
                                node: *child,
                                ambient_allowed,
                            });
                        }
                        work.push(AmbientProjectionWork::Expr {
                            expr: with_node.value,
                            ambient_allowed,
                        });
                    }
                },
            }
        }
        None
    }

    fn resolve_item(
        &mut self,
        item_id: ItemId,
        namespaces: &Namespaces,
        prefer_ambient_names: bool,
    ) {
        let item = self.module.items()[item_id].clone();
        for decorator in item.decorators() {
            self.resolve_decorator(*decorator, namespaces, prefer_ambient_names);
        }
        let resolved = match item {
            Item::Type(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.push_type_scope(self.type_parameter_scope(item.parameters.iter().copied()));
                match &item.body {
                    TypeItemBody::Alias(alias) => self.resolve_type(*alias, namespaces, &mut env),
                    TypeItemBody::Sum(variants) => {
                        for variant in variants.iter() {
                            for field in &variant.fields {
                                self.resolve_type(field.ty, namespaces, &mut env);
                            }
                        }
                    }
                }
                Item::Type(item)
            }
            Item::Value(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                if let Some(annotation) = item.annotation {
                    self.resolve_type(annotation, namespaces, &mut env);
                }
                self.resolve_expr(item.body, namespaces, &env);
                Item::Value(item)
            }
            Item::Function(mut item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.enable_implicit_type_parameters();
                for constraint in &item.context {
                    self.resolve_type(*constraint, namespaces, &mut env);
                }
                env.push_term_scope(
                    self.binding_scope(item.parameters.iter().map(|parameter| parameter.binding)),
                );
                for parameter in &item.parameters {
                    if let Some(annotation) = parameter.annotation {
                        self.resolve_type(annotation, namespaces, &mut env);
                    }
                }
                if let Some(annotation) = item.annotation {
                    self.resolve_type(annotation, namespaces, &mut env);
                }
                self.resolve_expr(item.body, namespaces, &env);
                item.type_parameters = env.implicit_type_parameters();
                Item::Function(item)
            }
            Item::Signal(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                if let Some(annotation) = item.annotation {
                    self.resolve_type(annotation, namespaces, &mut env);
                }
                if let Some(body) = item.body {
                    self.resolve_expr(body, namespaces, &env);
                }
                for update in &item.reactive_updates {
                    self.resolve_expr(update.guard, namespaces, &env);
                    self.resolve_expr(update.body, namespaces, &env);
                }
                Item::Signal(item)
            }
            Item::Class(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.push_type_scope(self.type_parameter_scope(item.parameters.iter().copied()));
                for superclass in &item.superclasses {
                    self.resolve_type(*superclass, namespaces, &mut env);
                }
                for constraint in &item.param_constraints {
                    self.resolve_type(*constraint, namespaces, &mut env);
                }
                let mut item = item;
                for member in &mut item.members {
                    let mut member_env = env.clone();
                    member_env.enable_implicit_type_parameters();
                    for constraint in &member.context {
                        self.resolve_type(*constraint, namespaces, &mut member_env);
                    }
                    self.resolve_type(member.annotation, namespaces, &mut member_env);
                    member.type_parameters = member_env.implicit_type_parameters();
                }
                Item::Class(item)
            }
            Item::Domain(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.push_type_scope(self.type_parameter_scope(item.parameters.iter().copied()));
                self.resolve_type(item.carrier, namespaces, &mut env);
                if self.type_contains_item_reference(item.carrier, item_id) {
                    self.emit_error(
                        item.header.span,
                        format!(
                            "domain `{}` cannot use itself in its carrier type",
                            item.name.text()
                        ),
                        code("recursive-domain-carrier"),
                    );
                }
                for member in &item.members {
                    self.resolve_type(member.annotation, namespaces, &mut env);
                    if let Some(body) = member.body {
                        let mut member_env = env.clone();
                        member_env.push_term_scope(self.binding_scope(
                            member.parameters.iter().map(|parameter| parameter.binding),
                        ));
                        self.resolve_expr(body, namespaces, &member_env);
                    }
                }
                Item::Domain(item)
            }
            Item::SourceProviderContract(item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                let item = item;
                for argument in &item.contract.arguments {
                    self.resolve_type(argument.annotation, namespaces, &mut env);
                }
                for option in &item.contract.options {
                    self.resolve_type(option.annotation, namespaces, &mut env);
                }
                for operation in &item.contract.operations {
                    self.resolve_type(operation.annotation, namespaces, &mut env);
                }
                for command in &item.contract.commands {
                    self.resolve_type(command.annotation, namespaces, &mut env);
                }
                Item::SourceProviderContract(item)
            }
            Item::Use(item) => Item::Use(item),
            Item::Export(mut item) => {
                item.resolution = self.resolve_export_target(&item.target, namespaces);
                Item::Export(item)
            }
            Item::Hoist(item) => Item::Hoist(item),
            Item::Instance(mut item) => {
                let mut env = ResolveEnv::default();
                if prefer_ambient_names {
                    env.set_prefer_ambient_names();
                }
                env.enable_implicit_type_parameters();
                self.resolve_type_reference(&mut item.class, namespaces, &mut env);
                for argument in item.arguments.iter() {
                    self.resolve_type(*argument, namespaces, &mut env);
                }
                for context in &item.context {
                    self.resolve_type(*context, namespaces, &mut env);
                }
                for member in &item.members {
                    if let Some(annotation) = member.annotation {
                        self.resolve_type(annotation, namespaces, &mut env);
                    }
                    let mut member_env = env.clone();
                    member_env.push_term_scope(self.binding_scope(
                        member.parameters.iter().map(|parameter| parameter.binding),
                    ));
                    self.resolve_expr(member.body, namespaces, &member_env);
                }
                item.type_parameters = env.implicit_type_parameters();
                Item::Instance(item)
            }
        };
        *self
            .module
            .arenas
            .items
            .get_mut(item_id)
            .expect("resolved item id should still exist") = resolved;
    }

    fn resolve_decorator(
        &mut self,
        decorator_id: DecoratorId,
        namespaces: &Namespaces,
        prefer_ambient_names: bool,
    ) {
        let decorator = self.module.decorators()[decorator_id].clone();
        let mut env = ResolveEnv::default();
        if prefer_ambient_names {
            env.set_prefer_ambient_names();
        }
        match &decorator.payload {
            DecoratorPayload::Bare => {}
            DecoratorPayload::Call(call) => {
                for argument in &call.arguments {
                    self.resolve_expr(*argument, namespaces, &env);
                }
                if let Some(options) = call.options {
                    self.resolve_expr(options, namespaces, &env);
                }
            }
            DecoratorPayload::RecurrenceWakeup(wakeup) => {
                self.resolve_expr(wakeup.witness, namespaces, &env);
            }
            DecoratorPayload::Source(source) => {
                for argument in &source.arguments {
                    self.resolve_expr(*argument, namespaces, &env);
                }
                if let Some(options) = source.options {
                    self.resolve_expr(options, namespaces, &env);
                }
            }
            DecoratorPayload::Test(_) | DecoratorPayload::Debug(_) => {}
            DecoratorPayload::Deprecated(deprecated) => {
                if let Some(message) = deprecated.message {
                    self.resolve_expr(message, namespaces, &env);
                }
                if let Some(options) = deprecated.options {
                    self.resolve_expr(options, namespaces, &env);
                }
            }
            DecoratorPayload::Mock(mock) => {
                self.resolve_expr(mock.target, namespaces, &env);
                self.resolve_expr(mock.replacement, namespaces, &env);
            }
        }
        *self
            .module
            .arenas
            .decorators
            .get_mut(decorator_id)
            .expect("resolved decorator id should still exist") = decorator;
    }

    fn resolve_expr(&mut self, expr_id: ExprId, namespaces: &Namespaces, env: &ResolveEnv) {
        let expr = self.module.exprs()[expr_id].clone();
        let resolved = match expr.kind {
            ExprKind::Name(mut reference) => {
                self.resolve_term_reference(&mut reference, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Name(reference),
                }
            }
            ExprKind::Integer(_)
            | ExprKind::Float(_)
            | ExprKind::Decimal(_)
            | ExprKind::BigInt(_)
            | ExprKind::AmbientSubject
            | ExprKind::Regex(_) => expr,
            ExprKind::Text(text) => {
                self.resolve_text_literal(&text, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Text(text),
                }
            }
            ExprKind::SuffixedInteger(mut literal) => {
                literal.resolution = self.resolve_literal_suffix(&literal.suffix, namespaces);
                Expr {
                    span: expr.span,
                    kind: ExprKind::SuffixedInteger(literal),
                }
            }
            ExprKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.resolve_expr(*element, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Tuple(elements),
                }
            }
            ExprKind::List(elements) => {
                for element in &elements {
                    self.resolve_expr(*element, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::List(elements),
                }
            }
            ExprKind::Map(map) => {
                for entry in &map.entries {
                    self.resolve_expr(entry.key, namespaces, env);
                    self.resolve_expr(entry.value, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Map(map),
                }
            }
            ExprKind::Set(elements) => {
                for element in &elements {
                    self.resolve_expr(*element, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Set(elements),
                }
            }
            ExprKind::Record(record) => {
                for field in &record.fields {
                    self.resolve_expr(field.value, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Record(record),
                }
            }
            ExprKind::Projection { base, path } => {
                if let ProjectionBase::Expr(base) = base {
                    self.resolve_expr(base, namespaces, env);
                    Expr {
                        span: expr.span,
                        kind: ExprKind::Projection {
                            base: ProjectionBase::Expr(base),
                            path,
                        },
                    }
                } else {
                    Expr {
                        span: expr.span,
                        kind: ExprKind::Projection { base, path },
                    }
                }
            }
            ExprKind::Apply { callee, arguments } => {
                self.resolve_expr(callee, namespaces, env);
                for argument in arguments.iter() {
                    self.resolve_expr(*argument, namespaces, env);
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Apply { callee, arguments },
                }
            }
            ExprKind::Unary {
                operator,
                expr: inner,
            } => {
                self.resolve_expr(inner, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Unary {
                        operator,
                        expr: inner,
                    },
                }
            }
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.resolve_expr(left, namespaces, env);
                self.resolve_expr(right, namespaces, env);
                Expr {
                    span: expr.span,
                    kind: ExprKind::Binary {
                        left,
                        operator,
                        right,
                    },
                }
            }
            ExprKind::PatchApply { target, patch } => {
                self.resolve_expr(target, namespaces, env);
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            self.resolve_expr(*expr, namespaces, env);
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            self.resolve_expr(expr, namespaces, env);
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::PatchApply { target, patch },
                }
            }
            ExprKind::PatchLiteral(patch) => {
                for entry in &patch.entries {
                    for segment in &entry.selector.segments {
                        if let crate::PatchSelectorSegment::BracketExpr { expr, .. } = segment {
                            self.resolve_expr(*expr, namespaces, env);
                        }
                    }
                    match entry.instruction.kind {
                        crate::PatchInstructionKind::Replace(expr)
                        | crate::PatchInstructionKind::Store(expr) => {
                            self.resolve_expr(expr, namespaces, env);
                        }
                        crate::PatchInstructionKind::Remove => {}
                    }
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::PatchLiteral(patch),
                }
            }
            ExprKind::Pipe(pipe) => {
                self.resolve_expr(pipe.head, namespaces, env);
                let mut pipe_env = env.clone();
                for stage in pipe.stages.iter() {
                    let mut stage_env = pipe_env.clone();
                    if stage.supports_memos()
                        && let Some(binding) = stage.subject_memo
                    {
                        stage_env.push_term_scope(self.binding_scope([binding]));
                    }
                    match &stage.kind {
                        PipeStageKind::Transform { expr }
                        | PipeStageKind::Gate { expr }
                        | PipeStageKind::Map { expr }
                        | PipeStageKind::Apply { expr }
                        | PipeStageKind::Tap { expr }
                        | PipeStageKind::FanIn { expr }
                        | PipeStageKind::Truthy { expr }
                        | PipeStageKind::Falsy { expr }
                        | PipeStageKind::RecurStart { expr }
                        | PipeStageKind::RecurStep { expr }
                        | PipeStageKind::Validate { expr }
                        | PipeStageKind::Previous { expr }
                        | PipeStageKind::Diff { expr } => {
                            self.resolve_expr(*expr, namespaces, &stage_env)
                        }
                        PipeStageKind::Accumulate { seed, step } => {
                            self.resolve_expr(*seed, namespaces, &stage_env);
                            self.resolve_expr(*step, namespaces, &stage_env);
                        }
                        PipeStageKind::Case { pattern, body } => {
                            let bindings = self.resolve_pattern(*pattern, namespaces, &stage_env);
                            let mut branch_env = stage_env.clone();
                            branch_env.push_term_scope(self.binding_scope(bindings));
                            self.resolve_expr(*body, namespaces, &branch_env);
                        }
                    }
                    if stage.supports_memos() {
                        if let Some(binding) = stage.subject_memo {
                            pipe_env.push_term_scope(self.binding_scope([binding]));
                        }
                        if let Some(binding) = stage.result_memo {
                            pipe_env.push_term_scope(self.binding_scope([binding]));
                        }
                    }
                }
                Expr {
                    span: expr.span,
                    kind: ExprKind::Pipe(pipe),
                }
            }
            ExprKind::Cluster(cluster_id) => {
                self.resolve_cluster(cluster_id, namespaces, env);
                expr
            }
            ExprKind::Markup(node_id) => {
                self.resolve_markup_node(node_id, namespaces, env);
                expr
            }
        };
        *self
            .module
            .arenas
            .exprs
            .get_mut(expr_id)
            .expect("resolved expr id should still exist") = resolved;
    }

    fn resolve_cluster(
        &mut self,
        cluster_id: crate::ClusterId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        let cluster = self.module.clusters()[cluster_id].clone();
        let spine = cluster.normalized_spine();
        for member in spine.apply_arguments() {
            self.resolve_expr(member, namespaces, env);
        }
        if let ApplicativeSpineHead::Expr(expr) = spine.pure_head() {
            self.resolve_expr(expr, namespaces, env);
        }
    }

    fn resolve_markup_node(
        &mut self,
        node_id: MarkupNodeId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        let node = self.module.markup_nodes()[node_id].clone();
        match node.kind {
            MarkupNodeKind::Element(element) => {
                for attribute in &element.attributes {
                    match &attribute.value {
                        MarkupAttributeValue::Expr(expr) => {
                            self.resolve_expr(*expr, namespaces, env)
                        }
                        MarkupAttributeValue::Text(text) => {
                            self.resolve_text_literal(text, namespaces, env)
                        }
                        MarkupAttributeValue::ImplicitTrue => {}
                    }
                }
                for child in &element.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            MarkupNodeKind::Control(control_id) => {
                self.resolve_control_node(control_id, namespaces, env)
            }
        }
    }

    fn resolve_control_node(
        &mut self,
        control_id: ControlNodeId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        let control = self.module.control_nodes()[control_id].clone();
        match control {
            ControlNode::Show(node) => {
                self.resolve_expr(node.when, namespaces, env);
                if let Some(expr) = node.keep_mounted {
                    self.resolve_expr(expr, namespaces, env);
                }
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            ControlNode::Each(node) => {
                self.resolve_expr(node.collection, namespaces, env);
                let mut child_env = env.clone();
                child_env.push_term_scope(self.binding_scope([node.binding]));
                if let Some(key) = node.key {
                    self.resolve_expr(key, namespaces, &child_env);
                }
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, &child_env);
                }
                if let Some(empty) = node.empty {
                    self.resolve_control_node(empty, namespaces, env);
                }
            }
            ControlNode::Match(node) => {
                self.resolve_expr(node.scrutinee, namespaces, env);
                for case in node.cases.iter() {
                    self.resolve_control_node(*case, namespaces, env);
                }
            }
            ControlNode::Empty(node) => {
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            ControlNode::Case(node) => {
                let bindings = self.resolve_pattern(node.pattern, namespaces, env);
                let mut child_env = env.clone();
                child_env.push_term_scope(self.binding_scope(bindings));
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, &child_env);
                }
            }
            ControlNode::Fragment(node) => {
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, env);
                }
            }
            ControlNode::With(node) => {
                self.resolve_expr(node.value, namespaces, env);
                let mut child_env = env.clone();
                child_env.push_term_scope(self.binding_scope([node.binding]));
                for child in &node.children {
                    self.resolve_markup_node(*child, namespaces, &child_env);
                }
            }
        }
    }

    fn resolve_pattern(
        &mut self,
        pattern_id: PatternId,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) -> Vec<BindingId> {
        let pattern = self.module.patterns()[pattern_id].clone();
        let mut bindings = Vec::new();
        let resolved = match pattern.kind {
            PatternKind::Wildcard | PatternKind::Integer(_) => pattern,
            PatternKind::Text(text) => {
                self.resolve_text_literal(&text, namespaces, env);
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Text(text),
                }
            }
            PatternKind::Binding(binding) => {
                bindings.push(binding.binding);
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Binding(binding),
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements.iter() {
                    bindings.extend(self.resolve_pattern(*element, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Tuple(elements),
                }
            }
            PatternKind::List { elements, rest } => {
                for element in &elements {
                    bindings.extend(self.resolve_pattern(*element, namespaces, env));
                }
                if let Some(rest) = rest {
                    bindings.extend(self.resolve_pattern(rest, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::List { elements, rest },
                }
            }
            PatternKind::Record(fields) => {
                for field in &fields {
                    bindings.extend(self.resolve_pattern(field.pattern, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Record(fields),
                }
            }
            PatternKind::Constructor {
                mut callee,
                arguments,
            } => {
                self.resolve_term_reference(&mut callee, namespaces, env);
                for argument in &arguments {
                    bindings.extend(self.resolve_pattern(*argument, namespaces, env));
                }
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::Constructor { callee, arguments },
                }
            }
            PatternKind::UnresolvedName(mut reference) => {
                self.resolve_term_reference(&mut reference, namespaces, env);
                Pattern {
                    span: pattern.span,
                    kind: PatternKind::UnresolvedName(reference),
                }
            }
        };
        *self
            .module
            .arenas
            .patterns
            .get_mut(pattern_id)
            .expect("resolved pattern id should still exist") = resolved;
        bindings
    }

    fn resolve_text_literal(
        &mut self,
        text: &TextLiteral,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        for segment in &text.segments {
            if let TextSegment::Interpolation(interpolation) = segment {
                self.resolve_expr(interpolation.expr, namespaces, env);
            }
        }
    }

    fn resolve_type(&mut self, type_id: TypeId, namespaces: &Namespaces, env: &mut ResolveEnv) {
        let ty = self.module.types()[type_id].clone();
        let resolved = match ty.kind {
            TypeKind::Name(mut reference) => {
                self.resolve_type_reference(&mut reference, namespaces, env);
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Name(reference),
                }
            }
            TypeKind::Tuple(elements) => {
                for element in elements.iter() {
                    self.resolve_type(*element, namespaces, env);
                }
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Tuple(elements),
                }
            }
            TypeKind::Record(fields) => {
                for field in &fields {
                    self.resolve_type(field.ty, namespaces, env);
                }
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Record(fields),
                }
            }
            TypeKind::RecordTransform { transform, source } => {
                self.resolve_type(source, namespaces, env);
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::RecordTransform { transform, source },
                }
            }
            TypeKind::Arrow { parameter, result } => {
                self.resolve_type(parameter, namespaces, env);
                self.resolve_type(result, namespaces, env);
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Arrow { parameter, result },
                }
            }
            TypeKind::Apply { callee, arguments } => {
                self.resolve_type(callee, namespaces, env);
                for argument in arguments.iter() {
                    self.resolve_type(*argument, namespaces, env);
                }
                TypeNode {
                    span: ty.span,
                    kind: TypeKind::Apply { callee, arguments },
                }
            }
        };
        *self
            .module
            .arenas
            .types
            .get_mut(type_id)
            .expect("resolved type id should still exist") = resolved;
    }

    fn resolve_term_reference(
        &mut self,
        reference: &mut TermReference,
        namespaces: &Namespaces,
        env: &ResolveEnv,
    ) {
        if reference.path.segments().len() != 1 {
            self.emit_error(
                reference.span(),
                format!(
                    "ordinary term reference `{}` is not supported in Milestone 2",
                    path_text(&reference.path)
                ),
                code("unsupported-qualified-term-ref"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        let name = reference.path.segments().first().text();
        if let Some(binding) = env.lookup_term(name) {
            reference.resolution = ResolutionState::Resolved(TermResolution::Local(binding));
            return;
        }
        if env.prefer_ambient_names() {
            match lookup_item(&namespaces.ambient_term_items, name) {
                LookupResult::Unique(item) => {
                    reference.resolution = ResolutionState::Resolved(TermResolution::Item(item));
                    return;
                }
                LookupResult::Ambiguous => {
                    self.emit_error(
                        reference.span(),
                        format!("ambient term `{name}` is ambiguous"),
                        code("ambiguous-term-name"),
                    );
                    reference.resolution = ResolutionState::Unresolved;
                    return;
                }
                LookupResult::Missing => {}
            }
            match lookup_item(&namespaces.ambient_class_terms, name) {
                LookupResult::Unique(resolution) => {
                    reference.resolution =
                        ResolutionState::Resolved(TermResolution::ClassMember(resolution));
                    return;
                }
                LookupResult::Ambiguous => {
                    if let Some(candidates) = namespaces.ambient_class_terms.get(name)
                        && let Ok(candidates) = crate::NonEmpty::from_vec(
                            candidates
                                .iter()
                                .map(|site| site.value)
                                .collect::<Vec<crate::hir::ClassMemberResolution>>(),
                        )
                    {
                        reference.resolution = ResolutionState::Resolved(
                            TermResolution::AmbiguousClassMembers(candidates),
                        );
                        return;
                    }
                }
                LookupResult::Missing => {}
            }
            if let Some(builtin) = builtin_term(name) {
                reference.resolution = ResolutionState::Resolved(TermResolution::Builtin(builtin));
                return;
            }
        }
        let term_lookup = lookup_item(&namespaces.term_items, name);
        let ambient_term_lookup = lookup_item(&namespaces.ambient_term_items, name);
        let domain_candidates = namespaces
            .domain_terms
            .get(name)
            .map(|candidates| {
                candidates
                    .iter()
                    .map(|candidate| candidate.value)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if matches!(term_lookup, LookupResult::Ambiguous) {
            self.emit_error(
                reference.span(),
                format!("term `{name}` is ambiguous in this module"),
                code("ambiguous-term-name"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        if let LookupResult::Unique(item) = term_lookup {
            if !domain_candidates.is_empty() {
                self.emit_error(
                    reference.span(),
                    format!("term `{name}` is ambiguous in this module"),
                    code("ambiguous-term-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            reference.resolution = ResolutionState::Resolved(TermResolution::Item(item));
            return;
        }
        if let LookupResult::Ambiguous = ambient_term_lookup {
            self.emit_error(
                reference.span(),
                format!("ambient term `{name}` is ambiguous"),
                code("ambiguous-term-name"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        if let LookupResult::Unique(item) = ambient_term_lookup {
            reference.resolution = ResolutionState::Resolved(TermResolution::Item(item));
            return;
        }
        let import_lookup = lookup_item(&namespaces.term_imports, name);
        if !domain_candidates.is_empty() {
            if !matches!(import_lookup, LookupResult::Missing) {
                self.emit_error(
                    reference.span(),
                    format!("term `{name}` is ambiguous in this module"),
                    code("ambiguous-term-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            if domain_candidates.len() == 1 {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::DomainMember(domain_candidates[0]));
                return;
            }
            if let Ok(candidates) = crate::NonEmpty::from_vec(domain_candidates) {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(candidates));
                return;
            }
        }
        match import_lookup {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match &import_binding.metadata {
                    ImportBindingMetadata::BuiltinTerm(builtin) => {
                        ResolutionState::Resolved(TermResolution::Builtin(*builtin))
                    }
                    ImportBindingMetadata::IntrinsicValue { value, .. } => {
                        ResolutionState::Resolved(TermResolution::IntrinsicValue(value.clone()))
                    }
                    ImportBindingMetadata::AmbientValue { name } => {
                        match lookup_item(&namespaces.ambient_term_items, name) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TermResolution::Item(item))
                            }
                            _ => ResolutionState::Resolved(TermResolution::Import(import)),
                        }
                    }
                    _ => ResolutionState::Resolved(TermResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("imported term `{name}` is ambiguous"),
                    code("ambiguous-import-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        // Hoisted names (from `hoist` declarations) are consulted after explicit
        // `use` imports.  Unique → resolved as a normal import.  Multiple
        // candidates (same name from different hoisted modules) → deferred to
        // type-directed disambiguation at the type-checking layer.
        match lookup_item(&namespaces.hoisted_term_imports, name) {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match &import_binding.metadata {
                    ImportBindingMetadata::BuiltinTerm(builtin) => {
                        ResolutionState::Resolved(TermResolution::Builtin(*builtin))
                    }
                    ImportBindingMetadata::IntrinsicValue { value, .. } => {
                        ResolutionState::Resolved(TermResolution::IntrinsicValue(value.clone()))
                    }
                    ImportBindingMetadata::AmbientValue { name } => {
                        match lookup_item(&namespaces.ambient_term_items, name) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TermResolution::Item(item))
                            }
                            _ => ResolutionState::Resolved(TermResolution::Import(import)),
                        }
                    }
                    _ => ResolutionState::Resolved(TermResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                if let Some(candidates) = namespaces.hoisted_term_imports.get(name)
                    && let Ok(candidates) = crate::NonEmpty::from_vec(
                        candidates.iter().map(|site| site.value).collect::<Vec<ImportId>>(),
                    )
                {
                    reference.resolution = ResolutionState::Resolved(
                        TermResolution::AmbiguousHoistedImports(candidates),
                    );
                    return;
                }
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.class_terms, name) {
            LookupResult::Unique(resolution) => {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::ClassMember(resolution));
                return;
            }
            LookupResult::Ambiguous => {
                if let Some(candidates) = namespaces.class_terms.get(name)
                    && let Ok(candidates) = crate::NonEmpty::from_vec(
                        candidates
                            .iter()
                            .map(|site| site.value)
                            .collect::<Vec<crate::hir::ClassMemberResolution>>(),
                    )
                {
                    reference.resolution = ResolutionState::Resolved(
                        TermResolution::AmbiguousClassMembers(candidates),
                    );
                    return;
                }
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.ambient_class_terms, name) {
            LookupResult::Unique(resolution) => {
                reference.resolution =
                    ResolutionState::Resolved(TermResolution::ClassMember(resolution));
                return;
            }
            LookupResult::Ambiguous => {
                if let Some(candidates) = namespaces.ambient_class_terms.get(name)
                    && let Ok(candidates) = crate::NonEmpty::from_vec(
                        candidates
                            .iter()
                            .map(|site| site.value)
                            .collect::<Vec<crate::hir::ClassMemberResolution>>(),
                    )
                {
                    reference.resolution = ResolutionState::Resolved(
                        TermResolution::AmbiguousClassMembers(candidates),
                    );
                    return;
                }
            }
            LookupResult::Missing => {}
        }
        if let Some(builtin) = builtin_term(name) {
            reference.resolution = ResolutionState::Resolved(TermResolution::Builtin(builtin));
            return;
        }
        {
            let mut candidates: Vec<&str> = Vec::new();
            candidates.extend(
                env.term_scopes
                    .iter()
                    .flat_map(|scope| scope.keys().map(|k| k.as_str())),
            );
            candidates.extend(namespaces.term_items.keys().map(|k| k.as_str()));
            candidates.extend(namespaces.ambient_term_items.keys().map(|k| k.as_str()));
            candidates.extend(namespaces.term_imports.keys().map(|k| k.as_str()));
            let mut diag = Diagnostic::error(format!("unknown term `{name}`"))
                .with_code(code("unresolved-term-name"))
                .with_primary_label(
                    reference.span(),
                    "reported during Milestone 2 HIR lowering",
                );
            if let Some(suggestion) = closest_name(name, &candidates) {
                diag = diag.with_help(format!("did you mean `{suggestion}`?"));
            }
            self.diagnostics.push(diag);
        }
        reference.resolution = ResolutionState::Unresolved;
    }

    fn resolve_type_reference(
        &mut self,
        reference: &mut TypeReference,
        namespaces: &Namespaces,
        env: &mut ResolveEnv,
    ) {
        if reference.path.segments().len() != 1 {
            self.emit_error(
                reference.span(),
                format!(
                    "ordinary type reference `{}` is not supported in Milestone 2",
                    path_text(&reference.path)
                ),
                code("unsupported-qualified-type-ref"),
            );
            reference.resolution = ResolutionState::Unresolved;
            return;
        }
        let name = reference.path.segments().first().text();
        if let Some(parameter) = env.lookup_type(name) {
            reference.resolution =
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter));
            return;
        }
        if env.prefer_ambient_names() {
            match lookup_item(&namespaces.ambient_type_items, name) {
                LookupResult::Unique(item) => {
                    reference.resolution = ResolutionState::Resolved(TypeResolution::Item(item));
                    return;
                }
                LookupResult::Ambiguous => {
                    self.emit_error(
                        reference.span(),
                        format!("ambient type `{name}` is ambiguous"),
                        code("ambiguous-type-name"),
                    );
                    reference.resolution = ResolutionState::Unresolved;
                    return;
                }
                LookupResult::Missing => {}
            }
            if let Some(builtin) = builtin_type(name) {
                reference.resolution = ResolutionState::Resolved(TypeResolution::Builtin(builtin));
                return;
            }
        }
        match lookup_item(&namespaces.type_items, name) {
            LookupResult::Unique(item) => {
                reference.resolution = ResolutionState::Resolved(TypeResolution::Item(item));
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("type `{name}` is ambiguous in this module"),
                    code("ambiguous-type-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.ambient_type_items, name) {
            LookupResult::Unique(item) => {
                reference.resolution = ResolutionState::Resolved(TypeResolution::Item(item));
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("ambient type `{name}` is ambiguous"),
                    code("ambiguous-type-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        match lookup_item(&namespaces.type_imports, name) {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match import_binding.metadata {
                    ImportBindingMetadata::BuiltinType(builtin) => {
                        ResolutionState::Resolved(TypeResolution::Builtin(builtin))
                    }
                    ImportBindingMetadata::AmbientType => {
                        match lookup_item(
                            &namespaces.ambient_type_items,
                            import_binding.imported_name.text(),
                        ) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TypeResolution::Item(item))
                            }
                            LookupResult::Ambiguous => {
                                self.emit_error(
                                    reference.span(),
                                    format!(
                                        "ambient type `{}` is ambiguous",
                                        import_binding.imported_name.text()
                                    ),
                                    code("ambiguous-type-name"),
                                );
                                ResolutionState::Unresolved
                            }
                            LookupResult::Missing => {
                                self.emit_error(
                                    reference.span(),
                                    format!(
                                        "import `{}` resolved without an ambient type target",
                                        import_binding.imported_name.text()
                                    ),
                                    code("invalid-import-resolution"),
                                );
                                ResolutionState::Unresolved
                            }
                        }
                    }
                    _ => ResolutionState::Resolved(TypeResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    reference.span(),
                    format!("imported type `{name}` is ambiguous"),
                    code("ambiguous-import-name"),
                );
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        // Hoisted type imports (from `hoist` declarations): consulted after
        // explicit `use` type imports but before builtins.
        match lookup_item(&namespaces.hoisted_type_imports, name) {
            LookupResult::Unique(import) => {
                let import_binding = &self.module.imports()[import];
                reference.resolution = match import_binding.metadata {
                    ImportBindingMetadata::BuiltinType(builtin) => {
                        ResolutionState::Resolved(TypeResolution::Builtin(builtin))
                    }
                    ImportBindingMetadata::AmbientType => {
                        match lookup_item(
                            &namespaces.ambient_type_items,
                            import_binding.imported_name.text(),
                        ) {
                            LookupResult::Unique(item) => {
                                ResolutionState::Resolved(TypeResolution::Item(item))
                            }
                            _ => ResolutionState::Resolved(TypeResolution::Import(import)),
                        }
                    }
                    _ => ResolutionState::Resolved(TypeResolution::Import(import)),
                };
                return;
            }
            LookupResult::Ambiguous => {
                // Multiple hoisted modules export the same type name.  Report an
                // error and suggest narrowing with `hiding`.
                if let Some(candidates) = namespaces.hoisted_type_imports.get(name) {
                    let modules = candidates
                        .iter()
                        .filter_map(|site| {
                            self.module
                                .imports()
                                .get(site.value)
                                .map(|b| b.imported_name.text().to_owned())
                        })
                        .collect::<Vec<_>>()
                        .join("`, `");
                    self.emit_error(
                        reference.span(),
                        format!(
                            "hoisted type `{name}` is ambiguous — it is exported by multiple \
                             hoisted modules (`{modules}`); add a `hiding` clause to the relevant \
                             `hoist` declarations to resolve the conflict",
                        ),
                        code("ambiguous-hoisted-type"),
                    );
                }
                reference.resolution = ResolutionState::Unresolved;
                return;
            }
            LookupResult::Missing => {}
        }
        if let Some(builtin) = builtin_type(name) {
            reference.resolution = ResolutionState::Resolved(TypeResolution::Builtin(builtin));
            return;
        }
        if is_implicit_type_parameter_candidate(name) && env.allow_implicit_type_parameters() {
            let parameter = env.bind_implicit_type_parameter(name, reference.span(), self);
            reference.resolution =
                ResolutionState::Resolved(TypeResolution::TypeParameter(parameter));
            return;
        }
        self.emit_error(
            reference.span(),
            format!("unknown type `{name}`"),
            code("unresolved-type-name"),
        );
        reference.resolution = ResolutionState::Unresolved;
    }

    fn resolve_export_target(
        &mut self,
        target: &NamePath,
        namespaces: &Namespaces,
    ) -> ResolutionState<ExportResolution> {
        if let Some(resolution) =
            self.resolve_export_item_target(target, &namespaces.term_items, &namespaces.type_items)
        {
            return resolution;
        }
        if let Some(resolution) = self.resolve_export_item_target(
            target,
            &namespaces.ambient_term_items,
            &namespaces.ambient_type_items,
        ) {
            return resolution;
        }

        let name = target.segments().first().text();

        // Allow re-exporting imported names (e.g. intrinsics forwarded through
        // a module's own `use` declaration).
        if let Some(resolution) = self.resolve_export_import_target(
            target,
            &namespaces.term_imports,
            &namespaces.type_imports,
        ) {
            return resolution;
        }

        if let Some(builtin) = builtin_term(name) {
            return ResolutionState::Resolved(ExportResolution::BuiltinTerm(builtin));
        }
        if let Some(builtin) = builtin_type(name) {
            return ResolutionState::Resolved(ExportResolution::BuiltinType(builtin));
        }

        self.emit_error(
            target.span(),
            format!("cannot export unknown item `{}`", path_text(target)),
            code("unknown-export-target"),
        );
        ResolutionState::Unresolved
    }

    fn resolve_export_import_target(
        &mut self,
        target: &NamePath,
        term_imports: &HashMap<String, Vec<NamedSite<ImportId>>>,
        type_imports: &HashMap<String, Vec<NamedSite<ImportId>>>,
    ) -> Option<ResolutionState<ExportResolution>> {
        let name = target.segments().first().text();
        let mut candidates: Vec<ImportId> = Vec::new();

        match lookup_item(term_imports, name) {
            LookupResult::Unique(import_id) => candidates.push(import_id),
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }
        match lookup_item(type_imports, name) {
            LookupResult::Unique(import_id) => {
                if !candidates.contains(&import_id) {
                    candidates.push(import_id);
                }
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }

        match candidates.as_slice() {
            [import_id] => Some(ResolutionState::Resolved(ExportResolution::Import(
                *import_id,
            ))),
            [] => None,
            _ => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                Some(ResolutionState::Unresolved)
            }
        }
    }

    fn resolve_export_item_target(
        &mut self,
        target: &NamePath,
        term_items: &HashMap<String, Vec<NamedSite<ItemId>>>,
        type_items: &HashMap<String, Vec<NamedSite<ItemId>>>,
    ) -> Option<ResolutionState<ExportResolution>> {
        let name = target.segments().first().text();
        let mut candidates = Vec::new();

        match lookup_item(term_items, name) {
            LookupResult::Unique(item) => candidates.push(item),
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }

        match lookup_item(type_items, name) {
            LookupResult::Unique(item) => {
                if !candidates.contains(&item) {
                    candidates.push(item);
                }
            }
            LookupResult::Ambiguous => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                return Some(ResolutionState::Unresolved);
            }
            LookupResult::Missing => {}
        }

        match candidates.as_slice() {
            [item] => Some(ResolutionState::Resolved(ExportResolution::Item(*item))),
            [] => None,
            _ => {
                self.emit_error(
                    target.span(),
                    format!("export `{}` is ambiguous", path_text(target)),
                    code("ambiguous-export"),
                );
                Some(ResolutionState::Unresolved)
            }
        }
    }

    fn resolve_literal_suffix(
        &mut self,
        suffix: &Name,
        namespaces: &Namespaces,
    ) -> ResolutionState<LiteralSuffixResolution> {
        if suffix.text().chars().count() < 2 {
            self.emit_error(
                suffix.span(),
                format!(
                    "literal suffix `{}` is too short; domain literal suffixes must be at least two characters long",
                    suffix.text()
                ),
                code("literal-suffix-too-short"),
            );
            return ResolutionState::Unresolved;
        }
        let Some(candidates) = namespaces.literal_suffixes.get(suffix.text()) else {
            self.emit_error(
                suffix.span(),
                format!("unknown literal suffix `{}`", suffix.text()),
                code("unknown-literal-suffix"),
            );
            return ResolutionState::Unresolved;
        };

        if candidates.len() > 1 {
            let mut diagnostic =
                Diagnostic::error(format!("literal suffix `{}` is ambiguous", suffix.text()))
                    .with_code(code("ambiguous-literal-suffix"))
                    .with_primary_label(
                        suffix.span(),
                        "this suffixed literal matches multiple domain literal declarations",
                    );
            for candidate in candidates.iter().take(3) {
                diagnostic = diagnostic
                    .with_secondary_label(candidate.span, "matching literal suffix declared here");
            }
            self.diagnostics.push(diagnostic);
            return ResolutionState::Unresolved;
        }

        ResolutionState::Resolved(candidates[0].value)
    }

    fn type_contains_item_reference(&self, root: TypeId, target: ItemId) -> bool {
        let mut stack = vec![root];
        while let Some(type_id) = stack.pop() {
            let ty = &self.module.types()[type_id];
            match &ty.kind {
                TypeKind::Name(reference) => {
                    if matches!(
                        reference.resolution,
                        ResolutionState::Resolved(TypeResolution::Item(item_id)) if item_id == target
                    ) {
                        return true;
                    }
                }
                TypeKind::Tuple(elements) => {
                    stack.extend(elements.iter().copied());
                }
                TypeKind::Record(fields) => {
                    stack.extend(fields.iter().map(|field| field.ty));
                }
                TypeKind::RecordTransform { source, .. } => {
                    stack.push(*source);
                }
                TypeKind::Arrow { parameter, result } => {
                    stack.push(*parameter);
                    stack.push(*result);
                }
                TypeKind::Apply { callee, arguments } => {
                    stack.push(*callee);
                    stack.extend(arguments.iter().copied());
                }
            }
        }
        false
    }

    fn binding_scope<I>(&self, bindings: I) -> HashMap<String, BindingId>
    where
        I: IntoIterator<Item = BindingId>,
    {
        bindings
            .into_iter()
            .map(|binding| {
                let binding_name = self.module.bindings()[binding].name.text().to_owned();
                (binding_name, binding)
            })
            .collect()
    }

    fn type_parameter_scope<I>(&self, parameters: I) -> HashMap<String, TypeParameterId>
    where
        I: IntoIterator<Item = TypeParameterId>,
    {
        parameters
            .into_iter()
            .map(|parameter| {
                let parameter_name = self.module.type_parameters()[parameter]
                    .name
                    .text()
                    .to_owned();
                (parameter_name, parameter)
            })
            .collect()
    }

    fn placeholder_expr(&mut self, span: SourceSpan) -> ExprId {
        self.alloc_expr(Expr {
            span,
            kind: ExprKind::Name(TermReference::unresolved(
                self.make_path(&[self.make_name("invalid", span)]),
            )),
        })
    }

    fn placeholder_type(&mut self, span: SourceSpan) -> TypeId {
        self.alloc_type(TypeNode {
            span,
            kind: TypeKind::Name(TypeReference {
                path: self.make_path(&[self.make_name("Unit", span)]),
                resolution: ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Unit)),
            }),
        })
    }

    fn placeholder_pattern(&mut self, span: SourceSpan) -> PatternId {
        self.alloc_pattern(Pattern {
            span,
            kind: PatternKind::Wildcard,
        })
    }

    fn make_name(&self, text: &str, span: SourceSpan) -> Name {
        Name::new(text.to_owned(), span).expect("non-empty lowered names should always be valid")
    }

    fn make_path(&self, names: &[Name]) -> NamePath {
        NamePath::from_vec(names.to_vec())
            .expect("non-empty same-file paths should always be valid")
    }

    fn emit_error(
        &mut self,
        span: SourceSpan,
        message: impl Into<String>,
        error_code: DiagnosticCode,
    ) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code(error_code)
                .with_primary_label(span, "reported during Milestone 2 HIR lowering"),
        );
    }

    fn emit_warning(
        &mut self,
        span: SourceSpan,
        message: impl Into<String>,
        warning_code: DiagnosticCode,
    ) {
        self.diagnostics.push(
            Diagnostic::warning(message)
                .with_code(warning_code)
                .with_primary_label(span, "reported during Milestone 2 HIR lowering"),
        );
    }

    fn emit_arena_overflow(&mut self, arena_name: &str) {
        self.diagnostics.push(
            Diagnostic::error(format!(
                "program too large: arena capacity exceeded ({})",
                arena_name
            ))
            .with_code(code("arena-overflow")),
        );
    }

    fn alloc_expr(&mut self, expr: Expr) -> ExprId {
        self.module.alloc_expr(expr).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR expr arena");
            std::process::exit(1);
        })
    }

    fn alloc_pattern(&mut self, pattern: Pattern) -> PatternId {
        self.module.alloc_pattern(pattern).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR pattern arena");
            std::process::exit(1);
        })
    }

    fn alloc_type(&mut self, ty: TypeNode) -> TypeId {
        self.module.alloc_type(ty).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR type arena");
            std::process::exit(1);
        })
    }

    fn alloc_decorator(&mut self, decorator: Decorator) -> DecoratorId {
        self.module.alloc_decorator(decorator).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR decorator arena");
            std::process::exit(1);
        })
    }

    fn alloc_markup_node(&mut self, node: MarkupNode) -> MarkupNodeId {
        self.module.alloc_markup_node(node).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR markup arena");
            std::process::exit(1);
        })
    }

    fn alloc_control(&mut self, control: ControlNode) -> ControlNodeId {
        self.module.alloc_control_node(control).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR control arena");
            std::process::exit(1);
        })
    }

    fn wrap_control(&mut self, control: ControlNode) -> MarkupNodeId {
        let span = control.span();
        let control_id = self.alloc_control(control);
        self.alloc_markup_node(MarkupNode {
            span,
            kind: MarkupNodeKind::Control(control_id),
        })
    }

    fn alloc_cluster(&mut self, cluster: ApplicativeCluster) -> crate::ClusterId {
        self.module.alloc_cluster(cluster).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR cluster arena");
            std::process::exit(1);
        })
    }

    fn alloc_binding(&mut self, binding: Binding) -> BindingId {
        self.module.alloc_binding(binding).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR binding arena");
            std::process::exit(1);
        })
    }

    fn alloc_type_parameter(&mut self, parameter: TypeParameter) -> TypeParameterId {
        self.module
            .alloc_type_parameter(parameter)
            .unwrap_or_else(|_| {
                self.emit_arena_overflow("HIR type parameter arena");
                std::process::exit(1);
            })
    }

    fn alloc_import(&mut self, import: ImportBinding) -> ImportId {
        self.module.alloc_import(import).unwrap_or_else(|_| {
            self.emit_arena_overflow("HIR import arena");
            std::process::exit(1);
        })
    }
}

/// Build a string label from an `ImportValueType`, used for generating unique synthetic
/// binding names for auto-imported instance members.
fn import_value_type_label(ty: &ImportValueType) -> String {
    match ty {
        ImportValueType::Primitive(builtin) => format!("{builtin:?}"),
        ImportValueType::Tuple(elements) => {
            let parts: Vec<_> = elements.iter().map(import_value_type_label).collect();
            format!("Tuple_{}", parts.join("_"))
        }
        ImportValueType::Record(fields) => {
            let parts: Vec<_> = fields
                .iter()
                .map(|f| format!("{}_{}", f.name, import_value_type_label(&f.ty)))
                .collect();
            format!("Record_{}", parts.join("_"))
        }
        ImportValueType::Arrow { parameter, result } => {
            format!(
                "Arrow_{}_{}",
                import_value_type_label(parameter),
                import_value_type_label(result)
            )
        }
        ImportValueType::List(elem) => format!("List_{}", import_value_type_label(elem)),
        ImportValueType::Map { key, value } => {
            format!(
                "Map_{}_{}",
                import_value_type_label(key),
                import_value_type_label(value)
            )
        }
        ImportValueType::Set(elem) => format!("Set_{}", import_value_type_label(elem)),
        ImportValueType::Option(elem) => format!("Option_{}", import_value_type_label(elem)),
        ImportValueType::Result { error, value } => {
            format!(
                "Result_{}_{}",
                import_value_type_label(error),
                import_value_type_label(value)
            )
        }
        ImportValueType::Validation { error, value } => {
            format!(
                "Validation_{}_{}",
                import_value_type_label(error),
                import_value_type_label(value)
            )
        }
        ImportValueType::Signal(elem) => format!("Signal_{}", import_value_type_label(elem)),
        ImportValueType::Task { error, value } => {
            format!(
                "Task_{}_{}",
                import_value_type_label(error),
                import_value_type_label(value)
            )
        }
        ImportValueType::TypeVariable { name, .. } => name.clone(),
        ImportValueType::Named {
            type_name,
            arguments,
        } => {
            if arguments.is_empty() {
                type_name.clone()
            } else {
                let args: Vec<_> = arguments.iter().map(import_value_type_label).collect();
                format!("{}_{}", type_name, args.join("_"))
            }
        }
    }
}

impl ResolveEnv {
    fn push_term_scope(&mut self, scope: HashMap<String, BindingId>) {
        self.term_scopes.push(scope);
    }

    fn push_type_scope(&mut self, scope: HashMap<String, TypeParameterId>) {
        self.type_scopes.push(scope);
    }

    fn enable_implicit_type_parameters(&mut self) {
        self.allow_implicit_type_parameters = true;
        if self.type_scopes.is_empty() {
            self.type_scopes.push(HashMap::new());
        }
    }

    fn set_prefer_ambient_names(&mut self) {
        self.prefer_ambient_names = true;
    }

    fn lookup_term(&self, name: &str) -> Option<BindingId> {
        self.term_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn lookup_type(&self, name: &str) -> Option<TypeParameterId> {
        self.type_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn allow_implicit_type_parameters(&self) -> bool {
        self.allow_implicit_type_parameters
    }

    fn prefer_ambient_names(&self) -> bool {
        self.prefer_ambient_names
    }

    fn bind_implicit_type_parameter(
        &mut self,
        name: &str,
        span: SourceSpan,
        lowerer: &mut Lowerer,
    ) -> TypeParameterId {
        if let Some(parameter) = self.lookup_type(name) {
            return parameter;
        }
        if self.type_scopes.is_empty() {
            self.type_scopes.push(HashMap::new());
        }
        let parameter = lowerer.alloc_type_parameter(TypeParameter {
            span,
            name: lowerer.make_name(name, span),
        });
        self.type_scopes
            .last_mut()
            .expect("implicit type parameter scope should exist")
            .insert(name.to_owned(), parameter);
        self.implicit_type_parameters.push(parameter);
        parameter
    }

    fn implicit_type_parameters(&self) -> Vec<TypeParameterId> {
        self.implicit_type_parameters.clone()
    }
}

fn is_implicit_type_parameter_candidate(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
}

impl LoweredMarkup {
    fn span(&self, lowerer: &Lowerer) -> SourceSpan {
        match self {
            Self::Renderable(id) => lowerer.module.markup_nodes()[*id].span,
            Self::Empty(id) | Self::Case(id) => lowerer.module.control_nodes()[*id].span(),
        }
    }
}

fn lower_unary_operator(operator: syn::UnaryOperator) -> UnaryOperator {
    match operator {
        syn::UnaryOperator::Not => UnaryOperator::Not,
    }
}

fn lower_binary_operator(operator: syn::BinaryOperator) -> BinaryOperator {
    match operator {
        syn::BinaryOperator::Add => BinaryOperator::Add,
        syn::BinaryOperator::Subtract => BinaryOperator::Subtract,
        syn::BinaryOperator::Multiply => BinaryOperator::Multiply,
        syn::BinaryOperator::Divide => BinaryOperator::Divide,
        syn::BinaryOperator::Modulo => BinaryOperator::Modulo,
        syn::BinaryOperator::GreaterThan => BinaryOperator::GreaterThan,
        syn::BinaryOperator::LessThan => BinaryOperator::LessThan,
        syn::BinaryOperator::GreaterThanOrEqual => BinaryOperator::GreaterThanOrEqual,
        syn::BinaryOperator::LessThanOrEqual => BinaryOperator::LessThanOrEqual,
        syn::BinaryOperator::Equals => BinaryOperator::Equals,
        syn::BinaryOperator::NotEquals => BinaryOperator::NotEquals,
        syn::BinaryOperator::And => BinaryOperator::And,
        syn::BinaryOperator::Or => BinaryOperator::Or,
    }
}

fn insert_named(
    map: &mut HashMap<String, Vec<NamedSite<ItemId>>>,
    name: &str,
    item_id: ItemId,
    span: SourceSpan,
    diagnostics: &mut Vec<Diagnostic>,
    error_code: DiagnosticCode,
    subject: &str,
) {
    let entry = map.entry(name.to_owned()).or_default();
    if let Some(previous) = entry.first().copied() {
        diagnostics.push(
            Diagnostic::error(format!("duplicate {subject} name `{name}`"))
                .with_code(error_code)
                .with_primary_label(span, "this declaration reuses an existing top-level name")
                .with_secondary_label(previous.span, "previous declaration here"),
        );
    }
    entry.push(NamedSite {
        value: item_id,
        span,
    });
}

fn insert_site<T: Copy>(
    map: &mut HashMap<String, Vec<NamedSite<T>>>,
    name: &str,
    value: T,
    span: SourceSpan,
) {
    map.entry(name.to_owned())
        .or_default()
        .push(NamedSite { value, span });
}

fn lookup_item<T: Copy>(map: &HashMap<String, Vec<NamedSite<T>>>, name: &str) -> LookupResult<T> {
    match map.get(name) {
        Some(values) if values.len() == 1 => LookupResult::Unique(values[0].value),
        Some(_) => LookupResult::Ambiguous,
        None => LookupResult::Missing,
    }
}

fn is_source_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "source"
}

fn is_test_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "test"
}

fn is_debug_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "debug"
}

fn is_deprecated_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "deprecated"
}

fn is_mock_decorator(path: &NamePath) -> bool {
    path.segments().len() == 1 && path.segments().first().text() == "mock"
}

fn recurrence_wakeup_decorator_kind(path: &NamePath) -> Option<RecurrenceWakeupDecoratorKind> {
    match path_text(path).as_str() {
        "recur.timer" => Some(RecurrenceWakeupDecoratorKind::Timer),
        "recur.backoff" => Some(RecurrenceWakeupDecoratorKind::Backoff),
        _ => None,
    }
}

fn is_recurrence_wakeup_decorator_name(path: &syn::QualifiedName) -> bool {
    matches!(path.as_dotted().as_str(), "recur.timer" | "recur.backoff")
}

fn path_text(path: &NamePath) -> String {
    path.segments()
        .iter()
        .map(|segment| segment.text())
        .collect::<Vec<_>>()
        .join(".")
}

fn domain_member_surface_name(name: &syn::DomainMemberName) -> String {
    match name {
        syn::DomainMemberName::Signature(syn::ClassMemberName::Identifier(identifier)) => {
            identifier.text.clone()
        }
        syn::DomainMemberName::Signature(syn::ClassMemberName::Operator(operator)) => {
            format!("({})", operator.text)
        }
        syn::DomainMemberName::Literal(identifier) => format!("literal {}", identifier.text),
    }
}

fn domain_member_surface_key(name: &syn::DomainMemberName) -> String {
    match name {
        syn::DomainMemberName::Signature(syn::ClassMemberName::Identifier(identifier)) => {
            format!("method:{}", identifier.text)
        }
        syn::DomainMemberName::Signature(syn::ClassMemberName::Operator(operator)) => {
            format!("operator:{}", operator.text)
        }
        syn::DomainMemberName::Literal(identifier) => format!("literal:{}", identifier.text),
    }
}

fn builtin_term(name: &str) -> Option<BuiltinTerm> {
    match name {
        "True" => Some(BuiltinTerm::True),
        "False" => Some(BuiltinTerm::False),
        "None" => Some(BuiltinTerm::None),
        "Some" => Some(BuiltinTerm::Some),
        "Ok" => Some(BuiltinTerm::Ok),
        "Err" => Some(BuiltinTerm::Err),
        "Valid" => Some(BuiltinTerm::Valid),
        "Invalid" => Some(BuiltinTerm::Invalid),
        _ => None,
    }
}

fn builtin_type(name: &str) -> Option<BuiltinType> {
    match name {
        "Int" => Some(BuiltinType::Int),
        "Float" => Some(BuiltinType::Float),
        "Decimal" => Some(BuiltinType::Decimal),
        "BigInt" => Some(BuiltinType::BigInt),
        "Bool" => Some(BuiltinType::Bool),
        "Text" => Some(BuiltinType::Text),
        "Unit" => Some(BuiltinType::Unit),
        "Bytes" => Some(BuiltinType::Bytes),
        "List" => Some(BuiltinType::List),
        "Map" => Some(BuiltinType::Map),
        "Set" => Some(BuiltinType::Set),
        "Option" => Some(BuiltinType::Option),
        "Result" => Some(BuiltinType::Result),
        "Validation" => Some(BuiltinType::Validation),
        "Signal" => Some(BuiltinType::Signal),
        "Task" => Some(BuiltinType::Task),
        _ => None,
    }
}

fn is_known_module(module: &str) -> bool {
    matches!(
        module,
        "aivi.network"
            | "aivi.defaults"
            | "aivi.random"
            | "aivi.stdio"
            | "aivi.fs"
            | "aivi.db"
            | "aivi.text"
            | "aivi.time"
            | "aivi.env"
            | "aivi.i18n"
            | "aivi.log"
            | "aivi.regex"
            | "aivi.http"
            | "aivi.bigint"
            | "aivi.nonEmpty"
            | "aivi.matrix"
            | "aivi.option"
            | "aivi.list"
    )
}

fn known_import_metadata(module: &str, member: &str) -> Option<ImportBindingMetadata> {
    match (module, member) {
        ("aivi.network", "http") | ("aivi.network", "socket") => {
            Some(ImportBindingMetadata::Value {
                ty: ImportValueType::Primitive(BuiltinType::Text),
            })
        }
        ("aivi.network", "Request") => Some(ImportBindingMetadata::TypeConstructor {
            kind: Kind::constructor(1),
            fields: None,
        }),
        ("aivi.network", "Channel") => Some(ImportBindingMetadata::TypeConstructor {
            kind: Kind::constructor(2),
            fields: None,
        }),
        ("aivi.defaults", "defaultText") => Some(ImportBindingMetadata::Value {
            ty: ImportValueType::Primitive(BuiltinType::Text),
        }),
        ("aivi.defaults", "defaultInt") => Some(ImportBindingMetadata::Value {
            ty: ImportValueType::Primitive(BuiltinType::Int),
        }),
        ("aivi.defaults", "defaultBool") => Some(ImportBindingMetadata::Value {
            ty: ImportValueType::Primitive(BuiltinType::Bool),
        }),
        ("aivi.defaults", "Option") => Some(ImportBindingMetadata::Bundle(
            ImportBundleKind::BuiltinOption,
        )),
        ("aivi.option", "getOrElse") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_option_getOrElse".into(),
        }),
        ("aivi.list", "length") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_length".into(),
        }),
        ("aivi.list", "head") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_head".into(),
        }),
        ("aivi.list", "tail") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_tail".into(),
        }),
        ("aivi.list", "any") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_any".into(),
        }),
        ("aivi.list", "at") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_listAt".into(),
        }),
        ("aivi.list", "replaceAt") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_listReplace".into(),
        }),
        ("aivi.stdio", "stdoutWrite") => Some(intrinsic_import_value(
            IntrinsicValue::StdoutWrite,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.stdio", "stderrWrite") => Some(intrinsic_import_value(
            IntrinsicValue::StderrWrite,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.fs", "createDirAll") => Some(intrinsic_import_value(
            IntrinsicValue::FsCreateDirAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.fs", "deleteFile") => Some(intrinsic_import_value(
            IntrinsicValue::FsDeleteFile,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Unit),
                ),
            ),
        )),
        ("aivi.fs", "writeText") => Some(intrinsic_import_value(
            IntrinsicValue::FsWriteText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Unit),
                    ),
                ),
            ),
        )),
        ("aivi.fs", "exists") => Some(intrinsic_import_value(
            IntrinsicValue::FsExists,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.random", "RandomError") => Some(ImportBindingMetadata::TypeConstructor {
            kind: Kind::constructor(0),
            fields: None,
        }),
        ("aivi.db", "paramBool") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamBool,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bool),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramInt") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramFloat") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamFloat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramDecimal") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamDecimal,
            arrow_import_type(
                primitive_import_type(BuiltinType::Decimal),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramBigInt") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamBigInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramText") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "paramBytes") => Some(intrinsic_import_value(
            IntrinsicValue::DbParamBytes,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                db_param_import_type(),
            ),
        )),
        ("aivi.db", "statement") => Some(intrinsic_import_value(
            IntrinsicValue::DbStatement,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    list_import_type(db_param_import_type()),
                    db_statement_import_type(),
                ),
            ),
        )),
        // Float math intrinsics
        ("aivi.core.float", "floor") => Some(intrinsic_import_value(
            IntrinsicValue::FloatFloor,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "ceil") => Some(intrinsic_import_value(
            IntrinsicValue::FloatCeil,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "round") => Some(intrinsic_import_value(
            IntrinsicValue::FloatRound,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "sqrt") => Some(intrinsic_import_value(
            IntrinsicValue::FloatSqrt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "abs") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAbs,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "toInt") => Some(intrinsic_import_value(
            IntrinsicValue::FloatToInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.core.float", "fromInt") => Some(intrinsic_import_value(
            IntrinsicValue::FloatFromInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "toText") => Some(intrinsic_import_value(
            IntrinsicValue::FloatToText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.core.float", "parseText") => Some(intrinsic_import_value(
            IntrinsicValue::FloatParseText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        // Path intrinsics — synchronous, operate on Text path strings
        ("aivi.path", "parent") => Some(intrinsic_import_value(
            IntrinsicValue::PathParent,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "filename") => Some(intrinsic_import_value(
            IntrinsicValue::PathFilename,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "stem") => Some(intrinsic_import_value(
            IntrinsicValue::PathStem,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "extension") => Some(intrinsic_import_value(
            IntrinsicValue::PathExtension,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.path", "join") => Some(intrinsic_import_value(
            IntrinsicValue::PathJoin,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        ("aivi.path", "isAbsolute") => Some(intrinsic_import_value(
            IntrinsicValue::PathIsAbsolute,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Bool),
            ),
        )),
        ("aivi.path", "normalize") => Some(intrinsic_import_value(
            IntrinsicValue::PathNormalize,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        // Bytes intrinsics — synchronous operations on the Bytes type
        ("aivi.core.bytes", "length") => Some(intrinsic_import_value(
            IntrinsicValue::BytesLength,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.core.bytes", "get") => Some(intrinsic_import_value(
            IntrinsicValue::BytesGet,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Bytes),
                    option_import_type(primitive_import_type(BuiltinType::Int)),
                ),
            ),
        )),
        ("aivi.core.bytes", "slice") => Some(intrinsic_import_value(
            IntrinsicValue::BytesSlice,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Bytes),
                        primitive_import_type(BuiltinType::Bytes),
                    ),
                ),
            ),
        )),
        ("aivi.core.bytes", "append") => Some(intrinsic_import_value(
            IntrinsicValue::BytesAppend,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Bytes),
                    primitive_import_type(BuiltinType::Bytes),
                ),
            ),
        )),
        ("aivi.core.bytes", "fromText") => Some(intrinsic_import_value(
            IntrinsicValue::BytesFromText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Bytes),
            ),
        )),
        ("aivi.core.bytes", "toText") => Some(intrinsic_import_value(
            IntrinsicValue::BytesToText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bytes),
                option_import_type(primitive_import_type(BuiltinType::Text)),
            ),
        )),
        ("aivi.core.bytes", "repeat") => Some(intrinsic_import_value(
            IntrinsicValue::BytesRepeat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    primitive_import_type(BuiltinType::Bytes),
                ),
            ),
        )),
        ("aivi.core.bytes", "empty") => Some(intrinsic_import_value(
            IntrinsicValue::BytesEmpty,
            primitive_import_type(BuiltinType::Bytes),
        )),
        // JSON intrinsics — async tasks, executed via serde_json in CLI
        ("aivi.data.json", "validate") => Some(intrinsic_import_value(
            IntrinsicValue::JsonValidate,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.data.json", "get") => Some(intrinsic_import_value(
            IntrinsicValue::JsonGet,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.data.json", "at") => Some(intrinsic_import_value(
            IntrinsicValue::JsonAt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.data.json", "keys") => Some(intrinsic_import_value(
            IntrinsicValue::JsonKeys,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    list_import_type(primitive_import_type(BuiltinType::Text)),
                ),
            ),
        )),
        ("aivi.data.json", "pretty") => Some(intrinsic_import_value(
            IntrinsicValue::JsonPretty,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        ("aivi.data.json", "minify") => Some(intrinsic_import_value(
            IntrinsicValue::JsonMinify,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                task_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        // XDG base directory intrinsics — synchronous, no I/O cost beyond env-var reads
        ("aivi.desktop.xdg", "dataHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgDataHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "configHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgConfigHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "cacheHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgCacheHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "stateHome") => Some(intrinsic_import_value(
            IntrinsicValue::XdgStateHome,
            primitive_import_type(BuiltinType::Text),
        )),
        ("aivi.desktop.xdg", "runtimeDir") => Some(intrinsic_import_value(
            IntrinsicValue::XdgRuntimeDir,
            option_import_type(primitive_import_type(BuiltinType::Text)),
        )),
        ("aivi.desktop.xdg", "dataDirs") => Some(intrinsic_import_value(
            IntrinsicValue::XdgDataDirs,
            list_import_type(primitive_import_type(BuiltinType::Text)),
        )),
        ("aivi.desktop.xdg", "configDirs") => Some(intrinsic_import_value(
            IntrinsicValue::XdgConfigDirs,
            list_import_type(primitive_import_type(BuiltinType::Text)),
        )),
        // Text intrinsics
        ("aivi.text", "length") => Some(intrinsic_import_value(
            IntrinsicValue::TextLength,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.text", "byteLen") => Some(intrinsic_import_value(
            IntrinsicValue::TextByteLen,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.text", "slice") => Some(intrinsic_import_value(
            IntrinsicValue::TextSlice,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.text", "find") => Some(intrinsic_import_value(
            IntrinsicValue::TextFind,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    option_import_type(primitive_import_type(BuiltinType::Int)),
                ),
            ),
        )),
        ("aivi.text", "contains") => Some(intrinsic_import_value(
            IntrinsicValue::TextContains,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.text", "startsWith") => Some(intrinsic_import_value(
            IntrinsicValue::TextStartsWith,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.text", "endsWith") => Some(intrinsic_import_value(
            IntrinsicValue::TextEndsWith,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.text", "toUpper") => Some(intrinsic_import_value(
            IntrinsicValue::TextToUpper,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "toLower") => Some(intrinsic_import_value(
            IntrinsicValue::TextToLower,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "trim") => Some(intrinsic_import_value(
            IntrinsicValue::TextTrim,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "trimStart") => Some(intrinsic_import_value(
            IntrinsicValue::TextTrimStart,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "trimEnd") => Some(intrinsic_import_value(
            IntrinsicValue::TextTrimEnd,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "replace") => Some(intrinsic_import_value(
            IntrinsicValue::TextReplace,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.text", "replaceAll") => Some(intrinsic_import_value(
            IntrinsicValue::TextReplaceAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.text", "split") => Some(intrinsic_import_value(
            IntrinsicValue::TextSplit,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    list_import_type(primitive_import_type(BuiltinType::Text)),
                ),
            ),
        )),
        ("aivi.text", "repeat") => Some(intrinsic_import_value(
            IntrinsicValue::TextRepeat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    primitive_import_type(BuiltinType::Text),
                ),
            ),
        )),
        ("aivi.text", "fromInt") => Some(intrinsic_import_value(
            IntrinsicValue::TextFromInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "parseInt") => Some(intrinsic_import_value(
            IntrinsicValue::TextParseInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Int)),
            ),
        )),
        ("aivi.text", "fromBool") => Some(intrinsic_import_value(
            IntrinsicValue::TextFromBool,
            arrow_import_type(
                primitive_import_type(BuiltinType::Bool),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "parseBool") => Some(intrinsic_import_value(
            IntrinsicValue::TextParseBool,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::Bool)),
            ),
        )),
        ("aivi.text", "concat") => Some(intrinsic_import_value(
            IntrinsicValue::TextConcat,
            arrow_import_type(
                list_import_type(primitive_import_type(BuiltinType::Text)),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        // Float transcendental intrinsics
        ("aivi.core.float", "sin") => Some(intrinsic_import_value(
            IntrinsicValue::FloatSin,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "cos") => Some(intrinsic_import_value(
            IntrinsicValue::FloatCos,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "tan") => Some(intrinsic_import_value(
            IntrinsicValue::FloatTan,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "asin") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAsin,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "acos") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAcos,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "atan") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAtan,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "atan2") => Some(intrinsic_import_value(
            IntrinsicValue::FloatAtan2,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Float),
                    primitive_import_type(BuiltinType::Float),
                ),
            ),
        )),
        ("aivi.core.float", "exp") => Some(intrinsic_import_value(
            IntrinsicValue::FloatExp,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "log") => Some(intrinsic_import_value(
            IntrinsicValue::FloatLog,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "log2") => Some(intrinsic_import_value(
            IntrinsicValue::FloatLog2,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "log10") => Some(intrinsic_import_value(
            IntrinsicValue::FloatLog10,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                option_import_type(primitive_import_type(BuiltinType::Float)),
            ),
        )),
        ("aivi.core.float", "pow") => Some(intrinsic_import_value(
            IntrinsicValue::FloatPow,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Float),
                    option_import_type(primitive_import_type(BuiltinType::Float)),
                ),
            ),
        )),
        ("aivi.core.float", "hypot") => Some(intrinsic_import_value(
            IntrinsicValue::FloatHypot,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Float),
                    primitive_import_type(BuiltinType::Float),
                ),
            ),
        )),
        ("aivi.core.float", "trunc") => Some(intrinsic_import_value(
            IntrinsicValue::FloatTrunc,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        ("aivi.core.float", "frac") => Some(intrinsic_import_value(
            IntrinsicValue::FloatFrac,
            arrow_import_type(
                primitive_import_type(BuiltinType::Float),
                primitive_import_type(BuiltinType::Float),
            ),
        )),
        // Time intrinsics
        ("aivi.time", "nowMs") => Some(intrinsic_import_value(
            IntrinsicValue::TimeNowMs,
            task_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.time", "monotonicMs") => Some(intrinsic_import_value(
            IntrinsicValue::TimeMonotonicMs,
            task_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Int),
            ),
        )),
        ("aivi.time", "format") => Some(intrinsic_import_value(
            IntrinsicValue::TimeFormat,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        ("aivi.time", "parse") => Some(intrinsic_import_value(
            IntrinsicValue::TimeParse,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Int),
                    ),
                ),
            ),
        )),
        // Regex intrinsics
        ("aivi.regex", "isMatch") => Some(intrinsic_import_value(
            IntrinsicValue::RegexIsMatch,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        primitive_import_type(BuiltinType::Bool),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "find") => Some(intrinsic_import_value(
            IntrinsicValue::RegexFind,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Int)),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "findText") => Some(intrinsic_import_value(
            IntrinsicValue::RegexFindText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        option_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "findAll") => Some(intrinsic_import_value(
            IntrinsicValue::RegexFindAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    task_import_type(
                        primitive_import_type(BuiltinType::Text),
                        list_import_type(primitive_import_type(BuiltinType::Text)),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "replace") => Some(intrinsic_import_value(
            IntrinsicValue::RegexReplace,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        task_import_type(
                            primitive_import_type(BuiltinType::Text),
                            primitive_import_type(BuiltinType::Text),
                        ),
                    ),
                ),
            ),
        )),
        ("aivi.regex", "replaceAll") => Some(intrinsic_import_value(
            IntrinsicValue::RegexReplaceAll,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Text),
                        task_import_type(
                            primitive_import_type(BuiltinType::Text),
                            primitive_import_type(BuiltinType::Text),
                        ),
                    ),
                ),
            ),
        )),
        // I18n intrinsics
        ("aivi.i18n", "tr") => Some(intrinsic_import_value(
            IntrinsicValue::I18nTranslate,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.i18n", "trn") => Some(intrinsic_import_value(
            IntrinsicValue::I18nTranslatePlural,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Text),
                    arrow_import_type(
                        primitive_import_type(BuiltinType::Int),
                        primitive_import_type(BuiltinType::Text),
                    ),
                ),
            ),
        )),
        // BigInt intrinsics (pure/synchronous)
        ("aivi.bigint", "fromInt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntFromInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::Int),
                primitive_import_type(BuiltinType::BigInt),
            ),
        )),
        ("aivi.bigint", "fromText") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntFromText,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                option_import_type(primitive_import_type(BuiltinType::BigInt)),
            ),
        )),
        ("aivi.bigint", "toInt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntToInt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                option_import_type(primitive_import_type(BuiltinType::Int)),
            ),
        )),
        ("aivi.bigint", "toText") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntToText,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.bigint", "add") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntAdd,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "sub") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntSub,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "mul") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntMul,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "div") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntDiv,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    option_import_type(primitive_import_type(BuiltinType::BigInt)),
                ),
            ),
        )),
        ("aivi.bigint", "mod") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntMod,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    option_import_type(primitive_import_type(BuiltinType::BigInt)),
                ),
            ),
        )),
        ("aivi.bigint", "pow") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntPow,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::Int),
                    primitive_import_type(BuiltinType::BigInt),
                ),
            ),
        )),
        ("aivi.bigint", "neg") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntNeg,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                primitive_import_type(BuiltinType::BigInt),
            ),
        )),
        ("aivi.bigint", "abs") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntAbs,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                primitive_import_type(BuiltinType::BigInt),
            ),
        )),
        ("aivi.bigint", "cmp") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntCmp,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Int),
                ),
            ),
        )),
        ("aivi.bigint", "eq") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntEq,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.bigint", "gt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntGt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        ("aivi.bigint", "lt") => Some(intrinsic_import_value(
            IntrinsicValue::BigIntLt,
            arrow_import_type(
                primitive_import_type(BuiltinType::BigInt),
                arrow_import_type(
                    primitive_import_type(BuiltinType::BigInt),
                    primitive_import_type(BuiltinType::Bool),
                ),
            ),
        )),
        // NonEmptyList ambient types and values
        ("aivi.nonEmpty", "NonEmptyList") => Some(ImportBindingMetadata::AmbientType),
        ("aivi.nonEmpty", "singleton") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_singleton".into(),
        }),
        ("aivi.nonEmpty", "head") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_head".into(),
        }),
        ("aivi.nonEmpty", "cons") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_cons".into(),
        }),
        ("aivi.nonEmpty", "length") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_length".into(),
        }),
        ("aivi.nonEmpty", "toList") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_toList".into(),
        }),
        ("aivi.nonEmpty", "init") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_init".into(),
        }),
        ("aivi.nonEmpty", "fromHeadTail") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_fromHeadTail".into(),
        }),
        ("aivi.nonEmpty", "last") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_last".into(),
        }),
        ("aivi.nonEmpty", "mapNel") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_mapNel".into(),
        }),
        ("aivi.nonEmpty", "appendNel") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_appendNel".into(),
        }),
        ("aivi.nonEmpty", "fromList") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_nel_fromList".into(),
        }),
        // Option ambient values
        ("aivi.option", "map") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_option_map".into(),
        }),
        // List ambient values
        ("aivi.list", "contains") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_contains".into(),
        }),
        ("aivi.list", "map") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_map".into(),
        }),
        ("aivi.list", "flatMap") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_flatMap".into(),
        }),
        ("aivi.list", "filter") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_filter".into(),
        }),
        ("aivi.list", "count") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_count".into(),
        }),
        ("aivi.list", "sum") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_sum".into(),
        }),
        ("aivi.list", "maximum") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_maximum".into(),
        }),
        // Text ambient values
        ("aivi.text", "join") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_text_join".into(),
        }),
        ("aivi.text", "lower") => Some(intrinsic_import_value(
            IntrinsicValue::TextToLower,
            arrow_import_type(
                primitive_import_type(BuiltinType::Text),
                primitive_import_type(BuiltinType::Text),
            ),
        )),
        ("aivi.text", "isEmpty") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_text_isEmpty".into(),
        }),
        ("aivi.text", "nonEmpty") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_text_nonEmpty".into(),
        }),
        ("aivi.list", "find") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_find".into(),
        }),
        ("aivi.list", "take") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_take".into(),
        }),
        ("aivi.list", "sortBy") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_sortBy".into(),
        }),
        // Matrix ambient types and values
        ("aivi.matrix", "Matrix") => Some(ImportBindingMetadata::AmbientType),
        ("aivi.matrix", "MatrixError") => Some(ImportBindingMetadata::AmbientType),
        ("aivi.matrix", "indices") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_list_range".into(),
        }),
        ("aivi.matrix", "fromRows") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_fromRows".into(),
        }),
        ("aivi.matrix", "at") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_at".into(),
        }),
        ("aivi.matrix", "replaceAt") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_replaceAt".into(),
        }),
        ("aivi.matrix", "rows") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_rows".into(),
        }),
        ("aivi.matrix", "width") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_width".into(),
        }),
        ("aivi.matrix", "height") => Some(ImportBindingMetadata::AmbientValue {
            name: "__aivi_matrix_height".into(),
        }),
        _ => None,
    }
}

fn intrinsic_import_value(value: IntrinsicValue, ty: ImportValueType) -> ImportBindingMetadata {
    ImportBindingMetadata::IntrinsicValue { value, ty }
}

fn primitive_import_type(builtin: BuiltinType) -> ImportValueType {
    ImportValueType::Primitive(builtin)
}

fn record_import_type(fields: Vec<ImportRecordField>) -> ImportValueType {
    ImportValueType::Record(fields)
}

fn record_import_field(name: &str, ty: ImportValueType) -> ImportRecordField {
    ImportRecordField {
        name: name.into(),
        ty,
    }
}

fn arrow_import_type(parameter: ImportValueType, result: ImportValueType) -> ImportValueType {
    ImportValueType::Arrow {
        parameter: Box::new(parameter),
        result: Box::new(result),
    }
}

fn task_import_type(error: ImportValueType, value: ImportValueType) -> ImportValueType {
    ImportValueType::Task {
        error: Box::new(error),
        value: Box::new(value),
    }
}

fn option_import_type(element: ImportValueType) -> ImportValueType {
    ImportValueType::Option(Box::new(element))
}

fn list_import_type(element: ImportValueType) -> ImportValueType {
    ImportValueType::List(Box::new(element))
}

fn db_param_import_type() -> ImportValueType {
    record_import_type(vec![
        record_import_field("kind", primitive_import_type(BuiltinType::Text)),
        record_import_field(
            "bool",
            option_import_type(primitive_import_type(BuiltinType::Bool)),
        ),
        record_import_field(
            "int",
            option_import_type(primitive_import_type(BuiltinType::Int)),
        ),
        record_import_field(
            "float",
            option_import_type(primitive_import_type(BuiltinType::Float)),
        ),
        record_import_field(
            "decimal",
            option_import_type(primitive_import_type(BuiltinType::Decimal)),
        ),
        record_import_field(
            "bigInt",
            option_import_type(primitive_import_type(BuiltinType::BigInt)),
        ),
        record_import_field(
            "text",
            option_import_type(primitive_import_type(BuiltinType::Text)),
        ),
        record_import_field(
            "bytes",
            option_import_type(primitive_import_type(BuiltinType::Bytes)),
        ),
    ])
}

fn db_statement_import_type() -> ImportValueType {
    record_import_type(vec![
        record_import_field("sql", primitive_import_type(BuiltinType::Text)),
        record_import_field("arguments", list_import_type(db_param_import_type())),
    ])
}

fn surface_exprs_equal(left: &syn::Expr, right: &syn::Expr) -> bool {
    match (&left.kind, &right.kind) {
        (syn::ExprKind::Group(left), _) => surface_exprs_equal(left, right),
        (_, syn::ExprKind::Group(right)) => surface_exprs_equal(left, right),
        (syn::ExprKind::Name(left), syn::ExprKind::Name(right)) => left.text == right.text,
        (syn::ExprKind::Integer(left), syn::ExprKind::Integer(right)) => left.raw == right.raw,
        (syn::ExprKind::Float(left), syn::ExprKind::Float(right)) => left.raw == right.raw,
        (syn::ExprKind::Decimal(left), syn::ExprKind::Decimal(right)) => left.raw == right.raw,
        (syn::ExprKind::BigInt(left), syn::ExprKind::BigInt(right)) => left.raw == right.raw,
        (syn::ExprKind::SuffixedInteger(left), syn::ExprKind::SuffixedInteger(right)) => {
            left.literal.raw == right.literal.raw && left.suffix.text == right.suffix.text
        }
        (syn::ExprKind::Text(left), syn::ExprKind::Text(right)) => {
            left.segments.len() == right.segments.len()
                && left
                    .segments
                    .iter()
                    .zip(&right.segments)
                    .all(|(left, right)| match (left, right) {
                        (syn::TextSegment::Text(left), syn::TextSegment::Text(right)) => {
                            left.raw == right.raw
                        }
                        (
                            syn::TextSegment::Interpolation(left),
                            syn::TextSegment::Interpolation(right),
                        ) => surface_exprs_equal(&left.expr, &right.expr),
                        _ => false,
                    })
        }
        (syn::ExprKind::Regex(left), syn::ExprKind::Regex(right)) => left.raw == right.raw,
        (syn::ExprKind::Tuple(left), syn::ExprKind::Tuple(right))
        | (syn::ExprKind::List(left), syn::ExprKind::List(right))
        | (syn::ExprKind::Set(left), syn::ExprKind::Set(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| surface_exprs_equal(left, right))
        }
        (syn::ExprKind::Map(left), syn::ExprKind::Map(right)) => {
            left.entries.len() == right.entries.len()
                && left
                    .entries
                    .iter()
                    .zip(&right.entries)
                    .all(|(left, right)| {
                        surface_exprs_equal(&left.key, &right.key)
                            && surface_exprs_equal(&left.value, &right.value)
                    })
        }
        (syn::ExprKind::Record(left), syn::ExprKind::Record(right)) => {
            left.fields.len() == right.fields.len()
                && left.fields.iter().zip(&right.fields).all(|(left, right)| {
                    left.label.text == right.label.text
                        && match (&left.value, &right.value) {
                            (Some(left), Some(right)) => surface_exprs_equal(left, right),
                            (None, None) => true,
                            (None, Some(value)) | (Some(value), None) => {
                                matches!(
                                    &value.kind,
                                    syn::ExprKind::Name(identifier)
                                        if identifier.text == left.label.text
                                )
                            }
                        }
                })
        }
        (syn::ExprKind::SubjectPlaceholder, syn::ExprKind::SubjectPlaceholder) => true,
        (syn::ExprKind::AmbientProjection(left), syn::ExprKind::AmbientProjection(right)) => {
            left.fields.len() == right.fields.len()
                && left
                    .fields
                    .iter()
                    .zip(&right.fields)
                    .all(|(left, right)| left.text == right.text)
        }
        (
            syn::ExprKind::Range {
                start: left_start,
                end: left_end,
            },
            syn::ExprKind::Range {
                start: right_start,
                end: right_end,
            },
        ) => {
            surface_exprs_equal(left_start, right_start) && surface_exprs_equal(left_end, right_end)
        }
        (
            syn::ExprKind::Projection {
                base: left_base,
                path: left_path,
            },
            syn::ExprKind::Projection {
                base: right_base,
                path: right_path,
            },
        ) => {
            surface_exprs_equal(left_base, right_base)
                && left_path.fields.len() == right_path.fields.len()
                && left_path
                    .fields
                    .iter()
                    .zip(&right_path.fields)
                    .all(|(left, right)| left.text == right.text)
        }
        (
            syn::ExprKind::Apply {
                callee: left_callee,
                arguments: left_arguments,
            },
            syn::ExprKind::Apply {
                callee: right_callee,
                arguments: right_arguments,
            },
        ) => {
            surface_exprs_equal(left_callee, right_callee)
                && left_arguments.len() == right_arguments.len()
                && left_arguments
                    .iter()
                    .zip(right_arguments)
                    .all(|(left, right)| surface_exprs_equal(left, right))
        }
        (
            syn::ExprKind::Unary {
                operator: left_operator,
                expr: left_expr,
            },
            syn::ExprKind::Unary {
                operator: right_operator,
                expr: right_expr,
            },
        ) => left_operator == right_operator && surface_exprs_equal(left_expr, right_expr),
        (
            syn::ExprKind::Binary {
                left: left_left,
                operator: left_operator,
                right: left_right,
            },
            syn::ExprKind::Binary {
                left: right_left,
                operator: right_operator,
                right: right_right,
            },
        ) => {
            left_operator == right_operator
                && surface_exprs_equal(left_left, right_left)
                && surface_exprs_equal(left_right, right_right)
        }
        (syn::ExprKind::ResultBlock(left), syn::ExprKind::ResultBlock(right)) => {
            left.bindings.len() == right.bindings.len()
                && left
                    .bindings
                    .iter()
                    .zip(&right.bindings)
                    .all(|(left, right)| {
                        left.name.text == right.name.text
                            && surface_exprs_equal(&left.expr, &right.expr)
                    })
                && match (&left.tail, &right.tail) {
                    (Some(left), Some(right)) => surface_exprs_equal(left, right),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

fn code(name: &'static str) -> DiagnosticCode {
    DiagnosticCode::new("hir", name)
}

/// Iterative Levenshtein distance between two strings (character-level).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j].min(dp[i][j - 1]).min(dp[i - 1][j - 1])
            };
        }
    }
    dp[m][n]
}

/// Return the closest candidate within Levenshtein distance 2 of `target`, or `None`.
fn closest_name<'a>(target: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .filter_map(|&c| {
            let d = levenshtein(target, c);
            if d <= 2 { Some((d, c)) } else { None }
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use aivi_base::{Severity, SourceDatabase};
    use aivi_syntax::parse_module;
    use aivi_typing::{BuiltinSourceProvider, Kind};

    use super::{lower_module, path_text};
    use crate::{
        ApplicativeSpineHead, BuiltinTerm, BuiltinType, ClusterFinalizer, ClusterPresentation,
        DecoratorPayload, DomainMemberKind, ExportResolution, ExprKind, HoistKindFilter,
        ImportBindingMetadata,
        ImportBundleKind, ImportValueType, IntrinsicValue, Item, LiteralSuffixResolution,
        PipeStageKind, ReactiveUpdateBodyMode, RecordRowTransform, RecurrenceWakeupDecoratorKind,
        ResolutionState, SourceProviderRef, TermResolution, TextSegment, TypeItemBody, TypeKind,
        TypeResolution, ValidationMode, exports,
    };

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
            .join("frontend")
    }

    fn lower_text(path: &str, text: &str) -> super::LoweringResult {
        let mut sources = SourceDatabase::new();
        let file_id = sources.add_file(path, text);
        let parsed = parse_module(&sources[file_id]);
        assert!(
            !parsed.has_errors(),
            "fixture {path} should parse before HIR lowering: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        lower_module(&parsed.module)
    }

    fn lower_fixture(path: &str) -> super::LoweringResult {
        let text =
            fs::read_to_string(fixture_root().join(path)).expect("fixture should be readable");
        lower_text(path, &text)
    }

    #[test]
    fn did_you_mean_suggestion_for_misspelled_binding() {
        // "couner" is 2 edits from "counter" — should trigger a "did you mean" hint.
        let result = lower_text(
            "did-you-mean.aivi",
            "value counter = 42\nvalue result = couner\n",
        );
        assert!(result.has_errors(), "expected an error for unknown term");
        let has_help = result.diagnostics().iter().any(|d| {
            d.help.iter().any(|h| h.contains("counter"))
        });
        assert!(
            has_help,
            "expected a 'did you mean `counter`?' hint, got diagnostics: {:?}",
            result.diagnostics()
        );
    }

    #[test]
    fn no_did_you_mean_for_very_different_binding() {
        // "xyz" has no close match to "counter" — no suggestion should appear.
        let result = lower_text(
            "no-suggestion.aivi",
            "value counter = 42\nvalue result = xyz\n",
        );
        assert!(result.has_errors(), "expected an error for unknown term");
        let has_help = result
            .diagnostics()
            .iter()
            .any(|d| !d.help.is_empty());
        assert!(
            !has_help,
            "expected no 'did you mean' hint for very different name, got: {:?}",
            result.diagnostics()
        );
    }

    fn find_ambient_named_item<'a>(module: &'a crate::Module, name: &str) -> &'a Item {
        module
            .ambient_items()
            .iter()
            .map(|item_id| &module.items()[*item_id])
            .find(|item| match item {
                Item::Type(item) => item.name.text() == name,
                Item::Value(item) => item.name.text() == name,
                Item::Function(item) => item.name.text() == name,
                Item::Signal(item) => item.name.text() == name,
                Item::Class(item) => item.name.text() == name,
                Item::Domain(item) => item.name.text() == name,
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
            | Item::Hoist(_) => false,
            })
            .unwrap_or_else(|| panic!("expected to find ambient item `{name}`"))
    }

    fn find_named_item<'a>(module: &'a crate::Module, name: &str) -> &'a Item {
        module
            .root_items()
            .iter()
            .map(|item_id| &module.items()[*item_id])
            .find(|item| match item {
                Item::Type(item) => item.name.text() == name,
                Item::Value(item) => item.name.text() == name,
                Item::Function(item) => item.name.text() == name,
                Item::Signal(item) => item.name.text() == name,
                Item::Class(item) => item.name.text() == name,
                Item::Domain(item) => item.name.text() == name,
                Item::SourceProviderContract(_)
                | Item::Instance(_)
                | Item::Use(_)
                | Item::Export(_)
            | Item::Hoist(_) => false,
            })
            .unwrap_or_else(|| panic!("expected to find named item `{name}`"))
    }

    fn find_signal<'a>(module: &'a crate::Module, name: &str) -> &'a crate::SignalItem {
        match find_named_item(module, name) {
            Item::Signal(item) => item,
            other => panic!("expected `{name}` to be a signal item, found {other:?}"),
        }
    }

    fn find_value<'a>(module: &'a crate::Module, name: &str) -> &'a crate::ValueItem {
        match find_named_item(module, name) {
            Item::Value(item) => item,
            other => panic!("expected `{name}` to be a value item, found {other:?}"),
        }
    }

    fn signal_dependency_names(module: &crate::Module, item: &crate::SignalItem) -> Vec<String> {
        item.signal_dependencies
            .iter()
            .map(|item_id| match &module.items()[*item_id] {
                Item::Signal(signal) => signal.name.text().to_owned(),
                other => {
                    panic!("expected signal dependency to point at a signal item, found {other:?}")
                }
            })
            .collect()
    }

    fn signal_item_names(module: &crate::Module, dependencies: &[crate::ItemId]) -> Vec<String> {
        dependencies
            .iter()
            .map(|item_id| match &module.items()[*item_id] {
                Item::Signal(signal) => signal.name.text().to_owned(),
                other => {
                    panic!("expected source dependency to point at a signal item, found {other:?}")
                }
            })
            .collect()
    }

    #[test]
    fn lowers_valid_fixture_corpus() {
        for path in [
            "milestone-2/valid/local-top-level-refs/main.aivi",
            "milestone-2/valid/use-member-imports/main.aivi",
            "milestone-2/valid/use-member-import-aliases/main.aivi",
            "milestone-2/valid/source-provider-contract-declarations/main.aivi",
            "milestone-2/valid/custom-source-provider-wakeup/main.aivi",
            "milestone-2/valid/custom-source-recurrence-wakeup/main.aivi",
            "milestone-2/valid/source-decorator-signals/main.aivi",
            "milestone-2/valid/source-option-contract-parameters/main.aivi",
            "milestone-2/valid/source-option-imported-binding-match/main.aivi",
            "milestone-2/valid/source-option-constructor-applications/main.aivi",
            "milestone-2/valid/applicative-clusters/main.aivi",
            "milestone-2/valid/markup-control-nodes/main.aivi",
            "milestone-2/valid/class-declarations/main.aivi",
            "milestone-2/valid/instance-declarations/main.aivi",
            "milestone-2/valid/domain-declarations/main.aivi",
            "milestone-2/valid/domain-member-resolution/main.aivi",
            "milestone-2/valid/domain-literal-suffixes/main.aivi",
            "milestone-2/valid/type-kinds/main.aivi",
            "milestone-2/valid/pipe-branch-and-join/main.aivi",
            "milestone-2/valid/pipe-fanout-carriers/main.aivi",
            "milestone-2/valid/result-block/main.aivi",
            "milestone-2/valid/pipe-accumulate-signal-wakeup/main.aivi",
            "milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi",
            "milestone-1/valid/records/record_shorthand_and_elision.aivi",
            "milestone-1/valid/sources/source_declarations.aivi",
            "milestone-1/valid/strings/text_and_regex.aivi",
            "milestone-1/valid/top-level/declarations.aivi",
            "milestone-1/valid/pipes/pipe_algebra.aivi",
            "milestone-1/valid/pipes/applicative_clusters.aivi",
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report.is_ok(),
                "expected {path} to validate as resolved HIR, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn lowers_record_row_transform_aliases_into_explicit_hir_types() {
        let lowered = lower_text(
            "record-row-transforms.aivi",
            concat!(
                "type User = { id: Int, name: Text, nickname: Option Text, createdAt: Text }\n",
                "type Public = Pick (id, name) User\n",
                "type Patch = User |> Omit (createdAt) |> Optional (name, nickname)\n",
                "type Snake = Rename { createdAt: created_at } User\n",
            ),
        );
        assert!(
            !lowered.has_errors(),
            "record row transform aliases should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let module = lowered.module();

        let Item::Type(public) = find_named_item(module, "Public") else {
            panic!("expected `Public` to be a type item");
        };
        let TypeItemBody::Alias(public_alias) = public.body else {
            panic!("expected `Public` to be an alias");
        };
        match &module.types()[public_alias].kind {
            TypeKind::RecordTransform { transform, source } => {
                assert!(matches!(transform, RecordRowTransform::Pick(labels) if labels.len() == 2));
                assert!(matches!(
                    &module.types()[*source].kind,
                    TypeKind::Name(reference)
                        if reference.path.to_string() == "User"
                ));
            }
            other => panic!("expected `Public` to lower to a record transform, found {other:?}"),
        }

        let Item::Type(patch) = find_named_item(module, "Patch") else {
            panic!("expected `Patch` to be a type item");
        };
        let TypeItemBody::Alias(patch_alias) = patch.body else {
            panic!("expected `Patch` to be an alias");
        };
        match &module.types()[patch_alias].kind {
            TypeKind::RecordTransform { transform, source } => {
                assert!(matches!(
                    transform,
                    RecordRowTransform::Optional(labels) if labels.len() == 2
                ));
                assert!(matches!(
                    &module.types()[*source].kind,
                    TypeKind::RecordTransform {
                        transform: RecordRowTransform::Omit(labels),
                        ..
                    } if labels.len() == 1
                ));
            }
            other => {
                panic!("expected `Patch` to lower to a nested record transform, found {other:?}")
            }
        }

        let Item::Type(snake) = find_named_item(module, "Snake") else {
            panic!("expected `Snake` to be a type item");
        };
        let TypeItemBody::Alias(snake_alias) = snake.body else {
            panic!("expected `Snake` to be an alias");
        };
        match &module.types()[snake_alias].kind {
            TypeKind::RecordTransform {
                transform: RecordRowTransform::Rename(renames),
                ..
            } => {
                assert_eq!(renames.len(), 1);
                assert_eq!(renames[0].from.text(), "createdAt");
                assert_eq!(renames[0].to.text(), "created_at");
            }
            other => panic!("expected `Snake` to lower to a rename transform, found {other:?}"),
        }
    }

    #[test]
    fn lowering_reports_malformed_record_row_transform_shapes() {
        let lowered = lower_text(
            "invalid-record-row-transform.aivi",
            concat!(
                "type User = { id: Int, name: Text }\n",
                "type BrokenPick = Pick id User\n",
                "type BrokenRename = Rename (id) User\n",
            ),
        );
        let codes = lowered
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        assert!(
            codes.contains(&super::code("invalid-record-row-transform")),
            "expected malformed transforms to report invalid-record-row-transform, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn lowers_single_source_signal_merge_arms() {
        let lowered = lower_text(
            "single_source_merge.aivi",
            r#"signal ready : Signal Bool
signal left : Signal Int
signal right : Signal Int

signal total : Signal Int = ready
  ||> True => left + right
  ||> _ => 0
"#,
        );
        assert!(
            !lowered.has_errors(),
            "expected signal merge to lower cleanly, got diagnostics: {:?}",
            lowered.diagnostics()
        );

        let module = lowered.module();
        let total = find_signal(module, "total");
        // Default arm becomes the seed body, so only 1 reactive update.
        assert_eq!(total.reactive_updates.len(), 1);
        assert!(total.body.is_some(), "default arm should become the signal seed body");
        assert_eq!(
            total.reactive_updates[0].body_mode,
            ReactiveUpdateBodyMode::OptionalPayload
        );
    }

    #[test]
    fn lowers_merge_rejects_unknown_source_signal() {
        let lowered = lower_text(
            "merge_unknown_source.aivi",
            r#"signal total : Signal Int = nonexistent
  ||> True => 42
  ||> _ => 0
"#,
        );
        assert!(
            lowered.has_errors(),
            "expected unknown merge source to fail lowering"
        );
        assert!(lowered.diagnostics().iter().any(|diagnostic| {
            diagnostic.severity == Severity::Error
                && diagnostic
                    .message
                    .contains("must name a previously declared signal")
        }));
    }

    #[test]
    fn lowers_multi_source_signal_merge_arms() {
        let lowered = lower_text(
            "multi_source_merge.aivi",
            r#"type Direction = Up | Down
type Event = Turn Direction | Tick

signal event = Turn Down

signal heading : Signal Direction = event
  ||> Turn dir => dir
  ||> _ => Up

signal tickSeen : Signal Bool = event
  ||> Tick => True
  ||> _ => False
"#,
        );
        assert!(
            !lowered.has_errors(),
            "expected multi-source signal merge to lower cleanly, got diagnostics: {:?}",
            lowered.diagnostics()
        );

        let module = lowered.module();
        let heading = find_signal(module, "heading");
        let tick_seen = find_signal(module, "tickSeen");
        // Default arm becomes seed, so only 1 reactive update each.
        assert_eq!(heading.reactive_updates.len(), 1);
        assert_eq!(tick_seen.reactive_updates.len(), 1);
        assert!(heading.body.is_some(), "default arm should become heading's seed");
        assert!(tick_seen.body.is_some(), "default arm should become tickSeen's seed");
        assert_eq!(
            heading.reactive_updates[0].body_mode,
            ReactiveUpdateBodyMode::OptionalPayload
        );
    }

    #[test]
    fn lowers_source_pattern_signal_merge_arms() {
        let lowered = lower_text(
            "source_pattern_merge.aivi",
            r#"type Key = Key Text
type Event = Tick | Turn Text

signal keyDown : Signal Key

signal event : Signal Event = keyDown
  ||> (Key "ArrowUp") => Turn "up"
  ||> _ => Tick
"#,
        );
        assert!(
            !lowered.has_errors(),
            "expected source-pattern merge to lower cleanly, got diagnostics: {:?}",
            lowered.diagnostics()
        );

        let module = lowered.module();
        let event = find_signal(module, "event");
        // Default arm becomes seed, so only 1 reactive update.
        assert_eq!(event.reactive_updates.len(), 1);
        assert!(event.body.is_some(), "default arm should become event's seed");
        assert_eq!(
            event.reactive_updates[0].body_mode,
            ReactiveUpdateBodyMode::OptionalPayload
        );
        assert!(matches!(
            module.exprs()[event.reactive_updates[0].guard].kind,
            ExprKind::Pipe(_)
        ));
    }

    #[test]
    fn lower_injects_ambient_typeclass_prelude() {
        let lowered = lower_text("ambient-prelude.aivi", "value answer:Int = 42\n");
        assert!(
            !lowered.has_errors(),
            "ambient prelude should lower cleanly, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        let module = lowered.module();
        assert!(
            module.ambient_items().len() >= 10,
            "expected ambient prelude items to be injected"
        );
        assert!(
            matches!(find_ambient_named_item(module, "Ordering"), Item::Type(_)),
            "expected ambient Ordering type to be present"
        );
        assert!(
            matches!(find_ambient_named_item(module, "Default"), Item::Class(_)),
            "expected ambient Default class to be present"
        );
        let Item::Class(traversable) = find_ambient_named_item(module, "Traversable") else {
            panic!("expected ambient Traversable class");
        };
        let traverse = traversable
            .members
            .iter()
            .find(|member| member.name.text() == "traverse")
            .expect("Traversable should expose traverse");
        assert_eq!(
            traverse.context.len(),
            1,
            "expected traverse to keep its Applicative constraint"
        );
        let Item::Class(applicative) = find_ambient_named_item(module, "Applicative") else {
            panic!("expected ambient Applicative class");
        };
        assert!(
            !applicative.superclasses.is_empty(),
            "Applicative should retain its superclass edge"
        );
    }

    #[test]
    fn ambient_prelude_prefers_builtin_names_over_user_shadowing() {
        let lowered = lower_text(
            "ambient-shadow-bool.aivi",
            r#"
type Bool = True | False

value answer:Int = 42
"#,
        );
        assert!(
            !lowered.has_errors(),
            "fixture should lower cleanly before validation: {:?}",
            lowered.diagnostics()
        );

        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "ambient prelude should validate even when the user shadows builtin Bool: {:?}",
            report.diagnostics()
        );

        let Item::Function(any_step) =
            find_ambient_named_item(lowered.module(), "__aivi_list_anyStep")
        else {
            panic!("expected `__aivi_list_anyStep` to lower as an ambient function");
        };
        let found_annotation = any_step.parameters[1]
            .annotation
            .expect("ambient helper parameter should retain its annotation");
        assert!(matches!(
            lowered.module().types()[found_annotation].kind,
            TypeKind::Name(ref reference)
                if matches!(
                    reference.resolution,
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Bool))
                )
        ));
    }

    #[test]
    fn reports_invalid_fixture_corpus_but_keeps_structural_hir() {
        for path in [
            "milestone-2/invalid/duplicate-top-level-names/main.aivi",
            "milestone-2/invalid/duplicate-source-provider-contract/main.aivi",
            "milestone-2/invalid/unknown-imported-names/main.aivi",
            "milestone-2/invalid/unknown-decorator/main.aivi",
            "milestone-2/invalid/unresolved-names/main.aivi",
            "milestone-2/invalid/misplaced-control-branches/main.aivi",
            "milestone-2/invalid/source-decorator-non-signal/main.aivi",
            "milestone-2/invalid/unknown-import-module/main.aivi",
            "milestone-2/invalid/domain-recursive-carrier/main.aivi",
            "milestone-2/invalid/ambiguous-domain-literal-suffix/main.aivi",
            "milestone-2/invalid/unpaired-truthy-falsy/main.aivi",
            "milestone-2/invalid/fanin-without-map/main.aivi",
            "milestone-2/invalid/cluster-ambient-projection/main.aivi",
            "milestone-2/invalid/orphan-recur-step/main.aivi",
            "milestone-2/invalid/unfinished-recurrence/main.aivi",
            "milestone-2/invalid/recurrence-continuation/main.aivi",
            "milestone-2/invalid/interpolated-pattern-text/main.aivi",
            "milestone-1/invalid/cluster_unfinished_gate.aivi",
            "milestone-1/invalid/source_unknown_option.aivi",
            "milestone-2/invalid/source-duplicate-option/main.aivi",
            "milestone-2/invalid/source-provider-without-variant/main.aivi",
            "milestone-2/invalid/source-legacy-quantity-option/main.aivi",
        ] {
            let lowered = lower_fixture(path);
            assert!(
                lowered.has_errors(),
                "expected {path} to fail HIR lowering, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered.module().validate(ValidationMode::Structural);
            assert!(
                report.is_ok(),
                "expected {path} to keep structurally valid HIR, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_kind_invalid_fixtures() {
        for (path, code_name) in [
            (
                "milestone-2/invalid/overapplied-type-constructor/main.aivi",
                "invalid-type-application",
            ),
            (
                "milestone-2/invalid/underapplied-domain-constructor/main.aivi",
                "expected-kind-mismatch",
            ),
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before kind validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
                "expected {path} to report {code_name}, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_recurrence_target_invalid_fixtures() {
        for (path, code_name) in [
            (
                "milestone-2/invalid/unknown-recurrence-target/main.aivi",
                "unknown-recurrence-target",
            ),
            (
                "milestone-2/invalid/unsupported-recurrence-target/main.aivi",
                "unsupported-recurrence-target",
            ),
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before recurrence target validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
                "expected {path} to report {code_name}, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_source_contract_invalid_fixtures() {
        for (path, code_name) in [
            (
                "milestone-2/invalid/source-contract-missing-type/main.aivi",
                "missing-source-contract-type",
            ),
            (
                "milestone-2/invalid/source-contract-arity-mismatch/main.aivi",
                "source-contract-type-arity",
            ),
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before source contract validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
                "expected {path} to report {code_name}, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_source_option_type_invalid_fixtures() {
        for path in [
            "milestone-2/invalid/source-option-type-mismatch/main.aivi",
            "milestone-2/invalid/source-option-contract-parameter-signal-mismatch/main.aivi",
            "milestone-2/invalid/source-option-imported-binding-mismatch/main.aivi",
            "milestone-2/invalid/source-option-constructor-mismatch/main.aivi",
            "milestone-2/invalid/source-option-constructor-application-mismatch/main.aivi",
            "milestone-2/invalid/source-option-list-element-mismatch/main.aivi",
            "milestone-2/invalid/custom-source-provider-option-type-mismatch/main.aivi",
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before source option value validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report.diagnostics().iter().any(|diagnostic| diagnostic.code
                    == Some(super::code("source-option-type-mismatch"))),
                "expected {path} to report source-option-type-mismatch, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_custom_source_provider_contract_invalid_fixtures() {
        for (path, code_name) in [
            (
                "milestone-2/invalid/custom-source-provider-unknown-option/main.aivi",
                "unknown-source-option",
            ),
            (
                "milestone-2/invalid/custom-source-provider-argument-count-mismatch/main.aivi",
                "source-argument-count-mismatch",
            ),
            (
                "milestone-2/invalid/custom-source-provider-argument-type-mismatch/main.aivi",
                "source-argument-type-mismatch",
            ),
            (
                "milestone-2/invalid/custom-source-provider-unsupported-schema-type/main.aivi",
                "unsupported-source-provider-contract-type",
            ),
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before custom provider contract validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(super::code(code_name))),
                "expected {path} to report {code_name}, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_recurrence_wakeup_invalid_fixtures() {
        for path in ["milestone-2/invalid/missing-recurrence-wakeup/main.aivi"] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before recurrence wakeup validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code
                        == Some(super::code("missing-recurrence-wakeup"))),
                "expected {path} to report missing-recurrence-wakeup, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_rejects_bodyful_source_signals() {
        for path in [
            "milestone-2/invalid/custom-source-recurrence-missing-wakeup/main.aivi",
            "milestone-2/invalid/request-recurrence-missing-wakeup/main.aivi",
        ] {
            let lowered = lower_fixture(path);
            assert!(
                !lowered.has_errors(),
                "expected {path} to lower cleanly before source validation, got diagnostics: {:?}",
                lowered.diagnostics()
            );
            let report = lowered
                .module()
                .validate(ValidationMode::RequireResolvedNames);
            assert!(
                report.diagnostics().iter().any(|diagnostic| diagnostic.code
                    == Some(super::code("source-signals-must-be-bodyless"))),
                "expected {path} to report source-signals-must-be-bodyless, got diagnostics: {:?}",
                report.diagnostics()
            );
        }
    }

    #[test]
    fn resolved_validation_accepts_request_sources_with_retry_policy_and_accumulate() {
        let lowered = lower_text(
            "request_source_with_retry_and_scan.aivi",
            r#"
type HttpError =
  | Timeout

type User = {
    id: Int
}

domain Retry over Int = {
    literal times : Int -> Retry
}
fun keepCount:Int = response:(Result HttpError (List User)) current:Int=>    current

@source http.get "/users" with {
    retry: 3times
}
signal responses : Signal (Result HttpError (List User))

signal retried : Signal Int =
    responses
     +|> 0 keepCount
"#,
        );
        assert!(
            !lowered.has_errors(),
            "request source with retry and accumulate should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "request source with retry and accumulate should validate cleanly, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn resolved_validation_accepts_reactive_source_option_payloads() {
        let lowered = lower_text(
            "reactive_source_option_payloads.aivi",
            r#"
domain Duration over Int = {
    literal sec : Int -> Duration
}
signal enabled : Signal Bool =
    True

signal jitterValue : Signal Duration =
    5sec

@source timer.every 120 with {
    immediate: enabled,
    activeWhen: enabled,
    jitter: jitterValue
}
signal tick : Signal Unit
"#,
        );
        assert!(
            !lowered.has_errors(),
            "reactive source option payloads should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "reactive source option payloads should validate cleanly, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn resolved_validation_accepts_custom_sources_feeding_accumulate_signals() {
        let lowered = lower_fixture("milestone-2/valid/custom-source-recurrence-wakeup/main.aivi");
        assert!(
            !lowered.has_errors(),
            "custom source accumulate fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "custom source accumulate fixture should validate cleanly, got diagnostics: {:?}",
            report.diagnostics()
        );

        let update_events = find_signal(lowered.module(), "updateEvents");
        let metadata = update_events
            .source_metadata
            .as_ref()
            .expect("bodyless custom source signal should still carry source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Custom("custom.feed".into())
        );
        assert!(
            metadata.is_reactive,
            "reactive custom source arguments should mark the source metadata as reactive"
        );
        assert_eq!(
            metadata.custom_contract, None,
            "surface lowering should not invent custom provider contract metadata"
        );
        assert_eq!(
            signal_dependency_names(lowered.module(), update_events),
            vec!["refresh".to_owned()],
            "custom source metadata should still track provider-independent reactive dependencies"
        );
        let updates = find_signal(lowered.module(), "updates");
        assert_eq!(
            signal_dependency_names(lowered.module(), updates),
            vec!["updateEvents".to_owned()],
            "accumulate-derived signals should depend on the raw source signal rather than provider inputs"
        );
    }

    #[test]
    fn resolves_provider_contract_declarations_onto_source_use_sites() {
        let lowered =
            lower_fixture("milestone-2/valid/source-provider-contract-declarations/main.aivi");
        assert!(
            !lowered.has_errors(),
            "provider contract declaration fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let contract = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::SourceProviderContract(item) => Some(item),
                _ => None,
            })
            .expect("expected to find provider contract item");
        assert_eq!(
            contract.provider,
            SourceProviderRef::Custom("custom.feed".into())
        );
        assert_eq!(
            contract.contract.recurrence_wakeup,
            Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger)
        );
        assert_eq!(contract.contract.arguments.len(), 1);
        assert_eq!(contract.contract.arguments[0].name.text(), "path");
        assert_eq!(contract.contract.options.len(), 2);
        assert_eq!(contract.contract.options[0].name.text(), "timeout");
        assert_eq!(contract.contract.options[1].name.text(), "mode");
        assert_eq!(contract.contract.operations.len(), 1);
        assert_eq!(contract.contract.operations[0].name.text(), "read");
        assert_eq!(contract.contract.commands.len(), 1);
        assert_eq!(contract.contract.commands[0].name.text(), "delete");

        let updates = find_signal(lowered.module(), "updates");
        let metadata = updates
            .source_metadata
            .as_ref()
            .expect("source-backed signal should keep source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Custom("custom.feed".into())
        );
        assert_eq!(
            metadata.custom_contract,
            Some(crate::CustomSourceContractMetadata {
                recurrence_wakeup: Some(
                    crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger
                ),
                arguments: contract.contract.arguments.clone(),
                options: contract.contract.options.clone(),
                operations: contract.contract.operations.clone(),
                commands: contract.contract.commands.clone(),
            }),
            "same-module provider declarations should resolve onto matching custom @source use sites"
        );
    }

    #[test]
    fn duplicate_provider_contracts_do_not_attach_ambiguous_use_site_metadata() {
        let lowered = lower_text(
            "duplicate_provider_contract_use_site.aivi",
            r#"
provider custom.feed
    wakeup: timer

provider custom.feed
    wakeup: backoff

@source custom.feed
signal updates : Signal Int
"#,
        );
        assert!(
            lowered.has_errors(),
            "duplicate provider contract test should still report lowering errors"
        );

        let updates = find_signal(lowered.module(), "updates");
        let metadata = updates
            .source_metadata
            .as_ref()
            .expect("custom source should still carry source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Custom("custom.feed".into())
        );
        assert_eq!(
            metadata.custom_contract, None,
            "ambiguous provider contract lookup must not attach arbitrary custom wakeup metadata"
        );
    }

    #[test]
    fn provider_contract_resolution_is_order_independent_within_module() {
        let lowered = lower_text(
            "provider_contract_resolution_order.aivi",
            r#"
@source custom.feed
signal updates : Signal Int

provider custom.feed
    wakeup: timer
"#,
        );
        assert!(
            !lowered.has_errors(),
            "same-module provider declarations should resolve regardless of source order: {:?}",
            lowered.diagnostics()
        );

        let updates = find_signal(lowered.module(), "updates");
        let metadata = updates
            .source_metadata
            .as_ref()
            .expect("custom source should still carry source metadata");
        assert_eq!(
            metadata.custom_contract,
            Some(crate::CustomSourceContractMetadata {
                recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::Timer),
                arguments: Vec::new(),
                options: Vec::new(),
                operations: Vec::new(),
                commands: Vec::new(),
            }),
            "provider contract resolution should use the module namespace rather than declaration order"
        );
    }

    #[test]
    fn provider_contract_declarations_report_builtin_keys_and_invalid_fields() {
        let lowered = lower_text(
            "provider_contract_errors.aivi",
            r#"
provider http.get
    wakeup: surprise
    mode: manual
    wakeup: timer
"#,
        );
        let codes = lowered
            .diagnostics()
            .iter()
            .filter_map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        assert!(
            codes.contains(&super::code("builtin-source-provider-contract")),
            "expected built-in provider contract diagnostic, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        assert!(
            codes.contains(&super::code("unknown-source-provider-contract-wakeup")),
            "expected unknown wakeup diagnostic, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        assert!(
            codes.contains(&super::code("unknown-source-provider-contract-field")),
            "expected unknown field diagnostic, got diagnostics: {:?}",
            lowered.diagnostics()
        );
        assert!(
            codes.contains(&super::code("duplicate-source-provider-contract-field")),
            "expected duplicate wakeup diagnostic, got diagnostics: {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn provider_contract_declarations_report_duplicate_schema_names() {
        let lowered = lower_text(
            "provider_contract_duplicate_schemas.aivi",
            r#"
provider custom.feed
    argument path: Text
    argument path: Int
    option timeout: Text
    option timeout: Bool
"#,
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .filter(|diagnostic| {
                    diagnostic.code == Some(super::code("duplicate-source-provider-contract-field"))
                })
                .count()
                >= 2,
            "expected duplicate schema diagnostics, got diagnostics: {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn provider_contract_declarations_report_duplicate_capability_member_names() {
        let lowered = lower_text(
            "provider_contract_duplicate_capability_members.aivi",
            r#"
provider custom.feed
    operation read : Text -> Signal Text
    command read : Text -> Task Text Unit
"#,
        );
        assert!(
            lowered.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(super::code("duplicate-source-provider-contract-field"))
            }),
            "expected duplicate capability member diagnostic, got diagnostics: {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn provider_contract_metadata_allows_nonreactive_recurrence() {
        let lowered = lower_fixture("milestone-2/valid/custom-source-provider-wakeup/main.aivi");
        assert!(
            !lowered.has_errors(),
            "provider-declared custom source wakeup fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let retry_events = find_signal(lowered.module(), "retryEvents");
        let metadata = retry_events
            .source_metadata
            .as_ref()
            .expect("provider-defined source signal should carry source metadata");
        assert!(
            !metadata.is_reactive,
            "provider-declared recurrence fixture should stay non-reactive"
        );
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Custom("custom.feed".into())
        );
        assert_eq!(
            metadata.custom_contract,
            Some(crate::CustomSourceContractMetadata {
                recurrence_wakeup: Some(
                    crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger
                ),
                arguments: Vec::new(),
                options: Vec::new(),
                operations: Vec::new(),
                commands: Vec::new(),
            }),
            "matching provider contracts should populate custom wakeup metadata before validation"
        );

        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "resolved custom provider metadata should unblock recurrence validation, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn manual_custom_source_contract_metadata_rejects_builtin_providers() {
        let lowered = lower_fixture("milestone-1/valid/sources/source_declarations.aivi");
        assert!(
            !lowered.has_errors(),
            "built-in source fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let mut module = lowered.module().clone();
        let signal_id = module
            .root_items()
            .iter()
            .copied()
            .find(|item_id| {
                matches!(
                    &module.items()[*item_id],
                    Item::Signal(item) if item.name.text() == "tick"
                )
            })
            .expect("expected to find `tick` signal item");
        let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
            panic!("expected `tick` item to stay a signal");
        };
        signal
            .source_metadata
            .as_mut()
            .expect("built-in source should carry source metadata")
            .custom_contract = Some(crate::CustomSourceContractMetadata {
            recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
            arguments: Vec::new(),
            options: Vec::new(),
            operations: Vec::new(),
            commands: Vec::new(),
        });

        let report = module.validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(super::code("invalid-custom-source-wakeup"))),
            "built-in sources should reject injected custom contract metadata, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn manual_custom_source_contract_metadata_rejects_invalid_provider_shapes() {
        let lowered =
            lower_fixture("milestone-2/invalid/source-provider-without-variant/main.aivi");
        assert!(
            lowered.has_errors(),
            "invalid provider fixture should still report a lowering error"
        );

        let mut module = lowered.module().clone();
        let signal_id = module
            .root_items()
            .iter()
            .copied()
            .find(|item_id| {
                matches!(
                    &module.items()[*item_id],
                    Item::Signal(item) if item.name.text() == "users"
                )
            })
            .expect("expected to find `users` signal item");
        let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
            panic!("expected `users` item to stay a signal");
        };
        signal
            .source_metadata
            .as_mut()
            .expect("invalid provider shape should still preserve source metadata")
            .custom_contract = Some(crate::CustomSourceContractMetadata {
            recurrence_wakeup: Some(crate::CustomSourceRecurrenceWakeup::ProviderDefinedTrigger),
            arguments: Vec::new(),
            options: Vec::new(),
            operations: Vec::new(),
            commands: Vec::new(),
        });

        let report = module.validate(ValidationMode::Structural);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(super::code("invalid-custom-source-wakeup"))),
            "malformed provider paths should reject injected custom contract metadata, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn resolved_validation_accepts_explicit_recurrence_wakeup_fixture() {
        let lowered = lower_fixture("milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi");
        assert!(
            !lowered.has_errors(),
            "explicit recurrence wakeup fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "explicit recurrence wakeup fixture should validate cleanly, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn lowers_recurrence_wakeup_decorators_into_typed_payloads() {
        let lowered = lower_fixture("milestone-2/valid/pipe-explicit-recurrence-wakeups/main.aivi");
        assert!(
            !lowered.has_errors(),
            "explicit recurrence wakeup fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let polled = find_signal(lowered.module(), "polled");
        let polled_decorator = &lowered.module().decorators()[polled.header.decorators[0]];
        match &polled_decorator.payload {
            DecoratorPayload::RecurrenceWakeup(wakeup) => {
                assert_eq!(wakeup.kind, RecurrenceWakeupDecoratorKind::Timer);
                assert!(matches!(
                    lowered.module().exprs()[wakeup.witness].kind,
                    ExprKind::Integer(_) | ExprKind::SuffixedInteger(_)
                ));
            }
            other => panic!(
                "expected `polled` to carry a typed recurrence wakeup decorator, found {other:?}"
            ),
        }

        let Item::Value(retried) = find_named_item(lowered.module(), "retried") else {
            panic!("expected `retried` to be a value item");
        };
        let retried_decorator = &lowered.module().decorators()[retried.header.decorators[0]];
        match &retried_decorator.payload {
            DecoratorPayload::RecurrenceWakeup(wakeup) => {
                assert_eq!(wakeup.kind, RecurrenceWakeupDecoratorKind::Backoff);
                assert!(matches!(
                    lowered.module().exprs()[wakeup.witness].kind,
                    ExprKind::Integer(_) | ExprKind::SuffixedInteger(_)
                ));
            }
            other => panic!(
                "expected `retried` to carry a typed recurrence wakeup decorator, found {other:?}"
            ),
        }
    }

    #[test]
    fn recurrence_wakeup_decorators_reject_invalid_shapes_and_source_mix() {
        let lowered = lower_text(
            "invalid_recurrence_wakeup_decorators.aivi",
            r#"
domain Duration over Int = {
    literal sec : Int -> Duration
}
domain Retry over Int = {
    literal times : Int -> Retry
}
fun step = x=>    x

@recur.timer
signal bare : Signal Int =
    0
     @|> step
     <|@ step

@source http.get "/users"
@recur.backoff 3times
signal mixed : Signal Int =
    0
     @|> step
     <|@ step

@recur.timer 5sec
@recur.backoff 3times
value duplicate : Task Int Int =
    0
     @|> step
     <|@ step
"#,
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(super::code("invalid-recurrence-wakeup-decorator"))),
            "expected invalid recurrence wakeup shape diagnostic, got {:?}",
            lowered.diagnostics()
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(super::code("invalid-source-recurrence-wakeup"))),
            "expected source/non-source recurrence wakeup conflict diagnostic, got {:?}",
            lowered.diagnostics()
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(super::code("duplicate-recurrence-wakeup-decorator"))),
            "expected duplicate recurrence wakeup decorator diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn preserves_bodyless_source_signals_and_provider_paths() {
        let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
        assert!(
            !lowered.has_errors(),
            "source-decorator fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let users = find_signal(lowered.module(), "users");
        assert!(
            users.body.is_none(),
            "source-backed signals should stay bodyless in HIR"
        );
        let metadata = users
            .source_metadata
            .as_ref()
            .expect("source-backed signal should carry source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Builtin(BuiltinSourceProvider::HttpGet),
            "source metadata should preserve built-in provider identity"
        );
        assert_eq!(
            metadata.custom_contract, None,
            "built-in providers should never attach custom-provider contract hooks"
        );
        assert!(
            metadata.is_reactive,
            "interpolated source arguments should mark the source as reactive"
        );
        assert_eq!(
            metadata.signal_dependencies.len(),
            1,
            "source metadata should capture the static signal dependency set"
        );
        assert_eq!(
            users.signal_dependencies, metadata.signal_dependencies,
            "source-backed signals should expose the same dependency set at the signal boundary"
        );
        let users_decorator = lowered.module().decorators()[users.header.decorators[0]].clone();
        match users_decorator.payload {
            DecoratorPayload::Source(source) => {
                assert_eq!(
                    source.provider.as_ref().map(path_text).as_deref(),
                    Some("http.get"),
                    "@source provider path should be preserved exactly"
                );
            }
            other => panic!("expected source decorator payload, found {other:?}"),
        }

        let tick = find_signal(lowered.module(), "tick");
        assert!(
            tick.body.is_none(),
            "bodyless timer source signal should stay bodyless"
        );
        let metadata = tick
            .source_metadata
            .as_ref()
            .expect("timer source should still carry source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Builtin(BuiltinSourceProvider::TimerEvery)
        );
        assert_eq!(
            metadata.custom_contract, None,
            "built-in source metadata should not use the custom-provider wakeup hook"
        );
        assert!(
            !metadata.is_reactive,
            "non-reactive source arguments should stay non-reactive"
        );
        assert!(
            metadata.signal_dependencies.is_empty(),
            "non-reactive sources should not record signal dependencies"
        );
        assert_eq!(
            tick.signal_dependencies, metadata.signal_dependencies,
            "non-reactive source signals should expose an empty dependency set"
        );
    }

    #[test]
    fn classifies_source_lifecycle_dependency_roles() {
        let lowered = lower_text(
            "source_lifecycle_dependency_roles.aivi",
            r#"
domain Duration over Int = {
    literal sec : Int -> Duration
}
provider custom.feed
    argument path: Text
    option activeWhen: Signal Bool

signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval : Signal Duration = 5sec
signal path = "/tmp/demo.txt"

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: pollInterval
}
signal users : Signal Int

@source custom.feed path with {
    activeWhen: enabled
}
signal updates : Signal Int
"#,
        );
        assert!(
            !lowered.has_errors(),
            "source lifecycle dependency role fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let users = find_signal(lowered.module(), "users");
        let metadata = users
            .source_metadata
            .as_ref()
            .expect("built-in source should carry source metadata");
        assert_eq!(
            signal_dependency_names(lowered.module(), users),
            vec![
                "apiHost".to_owned(),
                "refresh".to_owned(),
                "enabled".to_owned(),
                "pollInterval".to_owned()
            ]
        );
        assert_eq!(
            metadata.lifecycle_dependencies.merged(),
            metadata.signal_dependencies,
            "lifecycle dependency roles should merge back into the overall source dependency set"
        );
        assert_eq!(
            signal_item_names(
                lowered.module(),
                &metadata.lifecycle_dependencies.reconfiguration
            ),
            vec!["apiHost".to_owned(), "pollInterval".to_owned()]
        );
        assert_eq!(
            signal_item_names(
                lowered.module(),
                &metadata.lifecycle_dependencies.explicit_triggers
            ),
            vec!["refresh".to_owned()]
        );
        assert_eq!(
            signal_item_names(
                lowered.module(),
                &metadata.lifecycle_dependencies.active_when
            ),
            vec!["enabled".to_owned()]
        );

        let updates = find_signal(lowered.module(), "updates");
        let metadata = updates
            .source_metadata
            .as_ref()
            .expect("custom source should carry source metadata");
        assert_eq!(
            signal_item_names(
                lowered.module(),
                &metadata.lifecycle_dependencies.reconfiguration
            ),
            vec!["enabled".to_owned(), "path".to_owned()]
        );
        assert!(
            metadata.lifecycle_dependencies.explicit_triggers.is_empty(),
            "custom sources must not invent built-in trigger roles"
        );
        assert!(
            metadata.lifecycle_dependencies.active_when.is_empty(),
            "custom sources must not invent built-in activeWhen roles"
        );
    }

    #[test]
    fn manual_source_lifecycle_metadata_inconsistency_is_rejected() {
        let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
        assert!(
            !lowered.has_errors(),
            "source lifecycle validation fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let mut module = lowered.module().clone();
        let signal_id = module
            .root_items()
            .iter()
            .copied()
            .find(|item_id| {
                matches!(&module.items()[*item_id], Item::Signal(item) if item.name.text() == "users")
            })
            .expect("expected to find `users` signal item");
        let Some(Item::Signal(signal)) = module.arenas.items.get_mut(signal_id) else {
            panic!("expected `users` item to stay a signal");
        };
        signal
            .source_metadata
            .as_mut()
            .expect("source-backed signal should carry source metadata")
            .lifecycle_dependencies
            .reconfiguration
            .clear();

        let report = module.validate(ValidationMode::Structural);
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(super::code("inconsistent-source-lifecycle-dependencies"))
            }),
            "inconsistent source lifecycle dependency roles should be rejected, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn lowers_structured_text_interpolation_in_source_arguments() {
        let lowered = lower_fixture("milestone-2/valid/source-decorator-signals/main.aivi");
        assert!(
            !lowered.has_errors(),
            "source-decorator fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let users = find_signal(lowered.module(), "users");
        let users_decorator = &lowered.module().decorators()[users.header.decorators[0]];
        let DecoratorPayload::Source(source) = &users_decorator.payload else {
            panic!("expected source decorator payload");
        };
        let argument = source.arguments[0];
        let ExprKind::Text(text) = &lowered.module().exprs()[argument].kind else {
            panic!("expected interpolated text argument");
        };
        assert_eq!(text.segments.len(), 2);
        match &text.segments[0] {
            TextSegment::Interpolation(interpolation) => {
                let ExprKind::Name(reference) = &lowered.module().exprs()[interpolation.expr].kind
                else {
                    panic!("expected interpolation hole to lower as a name expression");
                };
                assert_eq!(
                    path_text(&reference.path),
                    "apiHost",
                    "interpolation should preserve the embedded expression"
                );
                assert!(
                    matches!(
                        reference.resolution,
                        ResolutionState::Resolved(TermResolution::Item(_))
                    ),
                    "interpolation names should resolve like ordinary expressions"
                );
            }
            other => panic!("expected leading interpolation segment, got {other:?}"),
        }
        match &text.segments[1] {
            TextSegment::Text(fragment) => assert_eq!(&*fragment.raw, "/users"),
            other => panic!("expected trailing text segment, got {other:?}"),
        }
    }

    #[test]
    fn tracks_signal_dependencies_for_ordinary_derived_signals() {
        let lowered = lower_fixture("milestone-2/valid/applicative-clusters/main.aivi");
        assert!(
            !lowered.has_errors(),
            "applicative cluster fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let validated_user = find_signal(lowered.module(), "validatedUser");
        assert_eq!(
            signal_dependency_names(lowered.module(), validated_user),
            vec![
                "nameText".to_owned(),
                "emailText".to_owned(),
                "ageValue".to_owned(),
            ],
            "derived signals should collect the union of referenced signal dependencies"
        );
        assert!(
            validated_user.source_metadata.is_none(),
            "ordinary derived signals should not carry source metadata"
        );

        let name_pair = find_signal(lowered.module(), "namePair");
        assert_eq!(
            signal_dependency_names(lowered.module(), name_pair),
            vec!["firstName".to_owned(), "lastName".to_owned()],
            "applicative derived signals should keep deterministic dependency ordering"
        );

        let local_refs = lower_fixture("milestone-2/valid/local-top-level-refs/main.aivi");
        assert!(
            !local_refs.has_errors(),
            "local top-level refs fixture should lower cleanly: {:?}",
            local_refs.diagnostics()
        );
        let next_refresh = find_signal(local_refs.module(), "nextRefresh");
        assert_eq!(
            signal_dependency_names(local_refs.module(), next_refresh),
            vec!["refreshMs".to_owned()],
            "value references must not leak into signal dependency metadata"
        );
    }

    #[test]
    fn tracks_signal_dependencies_through_helper_bodies() {
        let lowered = lower_text(
            "signal-helper-dependencies.aivi",
            r#"signal direction : Signal Int = 1
signal tick : Signal Int = 0
fun stepOnTick:Int = tick:Int => direction
signal game : Signal Int = stepOnTick tick
"#,
        );
        assert!(
            !lowered.has_errors(),
            "helper-body dependency example should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let game = find_signal(lowered.module(), "game");
        assert_eq!(
            signal_dependency_names(lowered.module(), game),
            vec!["direction".to_owned(), "tick".to_owned()],
            "signal dependency metadata should include signal reads hidden behind helper bodies"
        );
    }

    #[test]
    fn tracks_signal_dependencies_through_signal_record_projections() {
        let lowered = lower_text(
            "signal-projection-dependencies.aivi",
            "type Game = { score: Int }\n\
             type State = { game: Game, seenRestartCount: Int }\n\
             signal state : Signal State = { game: { score: 0 }, seenRestartCount: 0 }\n\
             signal game : Signal Game = state.game\n\
             signal score : Signal Int = state.game.score\n",
        );
        assert!(
            !lowered.has_errors(),
            "signal projection dependency example should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "signal projection dependency example should validate cleanly: {:?}",
            report.diagnostics()
        );

        let game = find_signal(lowered.module(), "game");
        assert_eq!(
            signal_dependency_names(lowered.module(), game),
            vec!["state".to_owned()],
            "projecting a field out of a signal record should keep the upstream signal dependency"
        );

        let score = find_signal(lowered.module(), "score");
        assert_eq!(
            signal_dependency_names(lowered.module(), score),
            vec!["state".to_owned()],
            "nested signal record projections should still trace back to the original upstream signal"
        );
    }

    #[test]
    fn normalizes_expression_headed_clusters_into_spines() {
        let lowered = lower_text(
            "expression-headed-clusters.aivi",
            "type NamePair = NamePair Text Text\n\
             signal firstName = \"Ada\"\n\
             signal lastName = \"Lovelace\"\n\
             signal headedPair =\n\
              firstName\n\
               &|> lastName\n\
                |> NamePair\n\
             signal headedTuple =\n\
              firstName\n\
               &|> lastName\n",
        );
        assert!(
            !lowered.has_errors(),
            "expression-headed clusters should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let headed_pair = find_signal(lowered.module(), "headedPair");
        let pair_body = headed_pair
            .body
            .expect("headedPair should lower to a cluster expression");
        let ExprKind::Cluster(pair_cluster_id) = &lowered.module().exprs()[pair_body].kind else {
            panic!("expected headedPair to lower as a cluster expression");
        };
        let pair_cluster = &lowered.module().clusters()[*pair_cluster_id];
        assert_eq!(
            pair_cluster.presentation,
            ClusterPresentation::ExpressionHeaded,
            "expression-headed surface form should stay visible in HIR"
        );
        let pair_spine = pair_cluster.normalized_spine();
        let pair_arguments = pair_spine
            .apply_arguments()
            .map(|expr_id| match &lowered.module().exprs()[expr_id].kind {
                ExprKind::Name(reference) => path_text(&reference.path),
                other => {
                    panic!("expected normalized cluster argument to stay a name, found {other:?}")
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            pair_arguments,
            vec!["firstName".to_owned(), "lastName".to_owned()],
            "normalized applicative spines should preserve cluster member order"
        );
        match pair_spine.pure_head() {
            ApplicativeSpineHead::Expr(expr_id) => match &lowered.module().exprs()[expr_id].kind {
                ExprKind::Name(reference) => assert_eq!(path_text(&reference.path), "NamePair"),
                other => panic!("expected explicit spine head to stay a name, found {other:?}"),
            },
            other => panic!("expected explicit applicative head, found {other:?}"),
        }

        let headed_tuple = find_signal(lowered.module(), "headedTuple");
        let tuple_body = headed_tuple
            .body
            .expect("headedTuple should lower to a cluster expression");
        let ExprKind::Cluster(tuple_cluster_id) = &lowered.module().exprs()[tuple_body].kind else {
            panic!("expected headedTuple to lower as a cluster expression");
        };
        match lowered.module().clusters()[*tuple_cluster_id]
            .normalized_spine()
            .pure_head()
        {
            ApplicativeSpineHead::TupleConstructor(arity) => assert_eq!(arity.get(), 2),
            other => panic!("expected implicit tuple applicative head, found {other:?}"),
        }
    }

    #[test]
    fn allows_nested_pipe_subjects_inside_clusters() {
        let lowered = lower_text(
            "nested-cluster-pipe-subject.aivi",
            "type NamePair = NamePair Text Text\n\
             signal firstName = \"Ada\"\n\
             signal lastName = \"Lovelace\"\n\
             signal ok =\n\
              firstName\n\
               &|> (lastName |> .display)\n\
                |> NamePair\n",
        );
        assert!(
            !lowered.has_errors(),
            "nested pipes with their own heads should remain legal inside clusters: {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn rejects_interpolated_pattern_text() {
        let lowered = lower_text(
            "interpolated-pattern-text.aivi",
            "value subject = \"Ada\"\nvalue result = subject ||> \"{subject}\" -> 1\n",
        );
        assert!(
            lowered.has_errors(),
            "interpolated pattern text should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("interpolated-pattern-text"))),
            "expected interpolated-pattern-text diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "invalid interpolated-pattern-text fixture should keep structural HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_unfinished_cluster_continuations() {
        let lowered = lower_fixture("milestone-1/invalid/cluster_unfinished_gate.aivi");
        assert!(
            lowered.has_errors(),
            "unfinished applicative clusters should be rejected"
        );
        assert!(
            lowered.diagnostics().iter().any(
                |diagnostic| diagnostic.code == Some(super::code("illegal-unfinished-cluster"))
            ),
            "expected illegal-unfinished-cluster diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "unfinished cluster errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_ambient_projections_inside_clusters() {
        let lowered = lower_fixture("milestone-2/invalid/cluster-ambient-projection/main.aivi");
        assert!(
            lowered.has_errors(),
            "ambient projections should be rejected inside applicative clusters"
        );
        assert!(
            lowered.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(super::code("illegal-cluster-ambient-projection"))
            }),
            "expected illegal-cluster-ambient-projection diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "cluster ambient-projection errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_duplicate_source_options() {
        let lowered = lower_fixture("milestone-2/invalid/source-duplicate-option/main.aivi");
        assert!(
            lowered.has_errors(),
            "duplicate source options should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-source-option"))),
            "expected duplicate-source-option diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "duplicate source options should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_source_provider_without_variant() {
        let lowered =
            lower_fixture("milestone-2/invalid/source-provider-without-variant/main.aivi");
        assert!(
            lowered.has_errors(),
            "source providers without variants should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("invalid-source-provider"))),
            "expected invalid-source-provider diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "source provider shape errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
        let users = find_signal(lowered.module(), "users");
        let metadata = users
            .source_metadata
            .as_ref()
            .expect("invalid source provider fixture should still preserve source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::InvalidShape("http".into())
        );
    }

    #[test]
    fn lowers_source_capability_handles_into_existing_source_and_task_paths() {
        let lowered = lower_text(
            "source_capability_handles.aivi",
            r#"
type FsSource = Unit
type FsError = Text

signal projectRoot : Signal Text = "/tmp/demo"

@source fs projectRoot
signal files : FsSource

signal config : Signal (Result FsError Text) = files.read "config.json"
value cleanup = files.delete "cache.txt"
"#,
        );
        assert!(
            !lowered.has_errors(),
            "capability handle lowering should not introduce front-end diagnostics: {:?}",
            lowered.diagnostics()
        );

        let files = find_signal(lowered.module(), "files");
        assert!(
            files.is_source_capability_handle,
            "bodyless `@source fs` anchors should lower as capability handles"
        );
        assert!(
            files.source_metadata.is_none(),
            "capability handles must not produce executable source metadata"
        );
        assert!(
            crate::exports::exports(lowered.module())
                .find("files")
                .is_none(),
            "capability handles are compile-time anchors and should not be exported as runtime signals"
        );

        let config = find_signal(lowered.module(), "config");
        assert!(
            config.body.is_none(),
            "signal capability operations should lower into bodyless source bindings"
        );
        let metadata = config
            .source_metadata
            .as_ref()
            .expect("lowered capability source should carry ordinary source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Builtin(aivi_typing::BuiltinSourceProvider::FsRead)
        );
        assert_eq!(
            signal_dependency_names(lowered.module(), config),
            vec!["projectRoot".to_owned()],
            "synthesized provider arguments should depend on the inherited handle root, not the handle anchor itself"
        );

        let cleanup = find_value(lowered.module(), "cleanup");
        let ExprKind::Apply { callee, arguments } = &lowered.module().exprs()[cleanup.body].kind
        else {
            panic!("expected cleanup to lower into an intrinsic application");
        };
        let ExprKind::Name(reference) = &lowered.module().exprs()[*callee].kind else {
            panic!("expected cleanup callee to be a resolved intrinsic");
        };
        assert_eq!(
            reference.resolution,
            ResolutionState::Resolved(TermResolution::IntrinsicValue(IntrinsicValue::FsDeleteFile))
        );
        let joined_path = *arguments.first();
        let ExprKind::Apply {
            callee: join_callee,
            arguments: join_arguments,
        } = &lowered.module().exprs()[joined_path].kind
        else {
            panic!("expected cleanup path to lower through a synthesized path join");
        };
        let ExprKind::Name(join_reference) = &lowered.module().exprs()[*join_callee].kind else {
            panic!("expected path join callee to be a resolved intrinsic");
        };
        assert_eq!(
            join_reference.resolution,
            ResolutionState::Resolved(TermResolution::IntrinsicValue(IntrinsicValue::PathJoin))
        );
        assert_eq!(
            join_arguments.len(),
            2,
            "path joins should combine the inherited handle root with the member path"
        );
    }

    #[test]
    fn lowers_custom_source_capability_operations_into_member_qualified_custom_sources() {
        let lowered = lower_text(
            "custom_source_capability_operations.aivi",
            r#"
type FeedSource = Unit

signal root = "/tmp/demo"
signal enabled = True

provider custom.feed
    argument path: Text
    option activeWhen: Signal Bool
    operation read : Text -> Signal Int
    command delete : Text -> Task Text Unit

@source custom.feed root with {
    activeWhen: enabled
}
signal feed : FeedSource

signal config : Signal Int = feed.read "config"
"#,
        );
        assert!(
            !lowered.has_errors(),
            "custom capability operations should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let feed = find_signal(lowered.module(), "feed");
        assert!(
            feed.is_source_capability_handle,
            "bodyless custom @source anchors should lower as capability handles"
        );

        let config = find_signal(lowered.module(), "config");
        assert!(
            config.body.is_none(),
            "custom capability operations should lower into bodyless source bindings"
        );
        let metadata = config
            .source_metadata
            .as_ref()
            .expect("lowered custom capability operation should carry source metadata");
        assert_eq!(
            metadata.provider,
            SourceProviderRef::Custom("custom.feed.read".into())
        );
        assert_eq!(
            signal_dependency_names(lowered.module(), config),
            vec!["root".to_owned(), "enabled".to_owned()],
            "custom capability operations should depend on inherited arguments/options, not the handle anchor"
        );
        let contract = metadata
            .custom_contract
            .as_ref()
            .expect("member-qualified custom sources should attach a derived contract");
        assert_eq!(
            contract.arguments.len(),
            2,
            "derived custom source contracts should include both provider arguments and member arguments"
        );
        assert_eq!(
            contract.options.len(),
            1,
            "member-qualified custom sources should preserve provider options"
        );

        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "member-qualified custom sources should validate against the derived contract, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn custom_source_capability_operations_require_signal_bindings() {
        let lowered = lower_text(
            "custom_source_capability_operation_value.aivi",
            r#"
type FeedSource = Unit

provider custom.feed
    operation read : Text -> Signal Int

@source custom.feed
signal feed : FeedSource

value load = feed.read "config"
"#,
        );
        assert!(
            lowered.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(super::code("invalid-source-capability-value-member"))
            }),
            "custom capability operations should reject `value` bindings, got diagnostics: {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn lowers_custom_source_capability_commands_into_typed_runtime_imports() {
        let lowered = lower_text(
            "custom_source_capability_command_value.aivi",
            r#"
type FeedSource = Unit

value mode = "sync"

provider custom.feed
    argument root: Text
    option mode: Text
    command delete : Text -> Task Text Unit

@source custom.feed "/tmp/demo" with {
    mode: mode
}
signal feed : FeedSource

value cleanup : Task Text Unit = feed.delete "config"
"#,
        );
        assert!(
            !lowered.has_errors(),
            "custom capability commands should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let cleanup = find_value(lowered.module(), "cleanup");
        let ExprKind::Apply { callee, arguments } = &lowered.module().exprs()[cleanup.body].kind
        else {
            panic!("custom capability command values should lower into a runtime application");
        };
        let ExprKind::Name(reference) = &lowered.module().exprs()[*callee].kind else {
            panic!("custom capability command callee should lower to a synthetic import");
        };
        let ResolutionState::Resolved(TermResolution::Import(import_id)) = reference.resolution
        else {
            panic!("custom capability command callee should resolve through a synthetic import");
        };
        assert_eq!(
            arguments.len(),
            3,
            "custom capability commands should apply inherited provider args, handle options, and member args"
        );
        let import = &lowered.module().imports()[import_id];
        let ImportBindingMetadata::IntrinsicValue {
            value: IntrinsicValue::CustomCapabilityCommand(spec),
            ..
        } = &import.metadata
        else {
            panic!(
                "custom capability command imports should carry the shared custom command intrinsic"
            );
        };
        assert_eq!(spec.provider_key.as_ref(), "custom.feed");
        assert_eq!(spec.command.as_ref(), "delete");
        assert_eq!(
            spec.provider_arguments.as_ref(),
            &["root".into()],
            "custom capability commands should preserve provider argument names"
        );
        assert_eq!(
            spec.options.as_ref(),
            &["mode".into()],
            "custom capability commands should preserve captured handle option names"
        );
        assert_eq!(
            spec.arguments.as_ref(),
            &["arg1".into()],
            "custom capability commands should synthesize stable member argument names"
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "custom capability commands should validate against the synthetic import type, got diagnostics: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_unknown_source_options_for_known_providers() {
        let lowered = lower_fixture("milestone-1/invalid/source_unknown_option.aivi");
        assert!(
            lowered.has_errors(),
            "unknown source options on known providers should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("unknown-source-option"))),
            "expected unknown-source-option diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "unknown source option errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_legacy_quantity_source_option_names() {
        let lowered = lower_fixture("milestone-2/invalid/source-legacy-quantity-option/main.aivi");
        assert!(
            lowered.has_errors(),
            "legacy quantity option names should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("unknown-source-option"))),
            "expected unknown-source-option diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "legacy quantity option errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_orphan_recur_steps() {
        let lowered = lower_fixture("milestone-2/invalid/orphan-recur-step/main.aivi");
        assert!(
            lowered.has_errors(),
            "orphan recurrence steps should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("orphan-recur-step"))),
            "expected orphan-recur-step diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "orphan recurrence step errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_unfinished_recurrence_suffixes() {
        let lowered = lower_fixture("milestone-2/invalid/unfinished-recurrence/main.aivi");
        assert!(
            lowered.has_errors(),
            "unfinished recurrence suffixes should be rejected"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("unfinished-recurrence"))),
            "expected unfinished-recurrence diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "unfinished recurrence errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn rejects_recurrence_suffix_continuations() {
        let lowered = lower_fixture("milestone-2/invalid/recurrence-continuation/main.aivi");
        assert!(
            lowered.has_errors(),
            "recurrence suffix continuations should be rejected"
        );
        assert!(
            lowered.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == Some(super::code("illegal-recurrence-continuation"))
            }),
            "expected illegal-recurrence-continuation diagnostic, got {:?}",
            lowered.diagnostics()
        );
        let report = lowered.module().validate(ValidationMode::Structural);
        assert!(
            report.is_ok(),
            "recurrence continuation errors should keep structurally valid HIR: {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn does_not_double_report_followup_recurrence_starts() {
        let lowered = lower_text(
            "duplicate-recurrence-starts.aivi",
            "fun step = x => x\nvalue broken = 0 @|> step @|> step <|@ step\n",
        );
        assert!(
            lowered.has_errors(),
            "duplicate recurrence starts should still be rejected"
        );
        let unfinished = lowered
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code == Some(super::code("unfinished-recurrence")))
            .count();
        let illegal = lowered
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(super::code("illegal-recurrence-continuation"))
            })
            .count();
        assert_eq!(
            unfinished,
            1,
            "expected exactly one unfinished-recurrence diagnostic, got {:?}",
            lowered.diagnostics()
        );
        assert_eq!(
            illegal,
            1,
            "expected exactly one illegal-recurrence-continuation diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn exposes_trailing_recurrence_suffix_views() {
        let lowered = lower_text(
            "recurrence-suffix-view.aivi",
            r#"fun keep = x => x
fun start = x => x
fun step = x => x
signal retried = 0 |> keep | keep @|> start <|@ step <|@ step
"#,
        );
        assert!(
            !lowered.has_errors(),
            "valid recurrence suffixes should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let retried = find_signal(lowered.module(), "retried");
        let body = retried
            .body
            .expect("retried should lower to a pipe expression");
        let ExprKind::Pipe(pipe) = &lowered.module().exprs()[body].kind else {
            panic!("expected retried to lower as a pipe expression");
        };
        let recurrence = pipe
            .recurrence_suffix()
            .expect("lowered pipe should satisfy the structural recurrence invariant")
            .expect("retried should include a recurrence suffix");

        assert_eq!(
            recurrence.prefix_stage_count(),
            2,
            "prefix stages should stay separate from the recurrence suffix"
        );
        let prefix_kinds = recurrence
            .prefix_stages()
            .map(|stage| match &stage.kind {
                PipeStageKind::Transform { .. } => "transform",
                PipeStageKind::Tap { .. } => "tap",
                other => panic!("expected only non-recurrent prefix stages, found {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(prefix_kinds, vec!["transform", "tap"]);
        match &lowered.module().exprs()[recurrence.start_expr()].kind {
            ExprKind::Name(reference) => assert_eq!(path_text(&reference.path), "start"),
            other => panic!("expected recurrence start expression to stay a name, found {other:?}"),
        }
        assert_eq!(recurrence.step_count(), 2);
        let step_names = recurrence
            .step_exprs()
            .map(|expr_id| match &lowered.module().exprs()[expr_id].kind {
                ExprKind::Name(reference) => path_text(&reference.path),
                other => {
                    panic!("expected recurrence step expression to stay a name, found {other:?}")
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(step_names, vec!["step".to_owned(), "step".to_owned()]);
    }

    #[test]
    fn allows_recurrence_guards_before_steps() {
        let lowered = lower_text(
            "recurrence-guard-view.aivi",
            r#"domain Duration over Int = {
    literal sec : Int -> Duration
}
type Cursor = { hasNext: Bool }
fun keep:Cursor = cursor:Cursor => cursor
value seed:Cursor = { hasNext: True }
@recur.timer 1sec
signal cursor : Signal Cursor =
 seed
  @|> keep
  ?|> .hasNext
  <|@ keep
"#,
        );
        assert!(
            !lowered.has_errors(),
            "recurrence guards should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let cursor = find_signal(lowered.module(), "cursor");
        let body = cursor
            .body
            .expect("cursor should lower to a pipe expression");
        let ExprKind::Pipe(pipe) = &lowered.module().exprs()[body].kind else {
            panic!("expected cursor to lower as a pipe expression");
        };
        let recurrence = pipe
            .recurrence_suffix()
            .expect("lowered pipe should satisfy the structural recurrence invariant")
            .expect("cursor should include a recurrence suffix");

        assert_eq!(recurrence.guard_stage_count(), 1);
        assert_eq!(recurrence.step_count(), 1);
    }

    #[test]
    fn allows_fanout_filters_before_join() {
        let lowered = lower_text(
            "fanout-filter-before-join.aivi",
            r#"type User = { email: Text }
fun keepText:Bool = email:Text => True
fun joinEmails:Text = items:List Text => "joined"
value users:List User = [{ email: "ada@example.com" }]
value joinedEmails:Text =
 users
  *|> .email
  ?|> keepText
  <|* joinEmails
"#,
        );
        assert!(
            !lowered.has_errors(),
            "fan-out filters before `<|*` should lower cleanly: {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn normalizes_standalone_function_signature_lines_into_parameter_annotations() {
        let lowered = lower_text(
            "standalone-function-signature.aivi",
            "type List Text -> Text\n\
             func joinEmails = items=> \"joined\"\n",
        );
        assert!(
            !lowered.has_errors(),
            "standalone function signature lines should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let function = match find_named_item(lowered.module(), "joinEmails") {
            Item::Function(item) => item,
            other => panic!("expected `joinEmails` to lower as a function item, found {other:?}"),
        };
        assert!(
            function.context.is_empty(),
            "plain parameter types should not remain in the class-constraint context"
        );
        let parameter_annotation = function.parameters[0]
            .annotation
            .expect("parameter annotation should be reconstructed from the signature line");
        let result_annotation = function
            .annotation
            .expect("result annotation should remain attached to the function");

        match &lowered.module().types()[parameter_annotation].kind {
            TypeKind::Apply { callee, arguments } => {
                assert_eq!(arguments.len(), 1);
                match &lowered.module().types()[*callee].kind {
                    TypeKind::Name(reference) => assert!(
                        matches!(
                            reference.resolution.as_ref(),
                            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::List))
                        ),
                        "expected builtin `List` callee resolution, found {:?}",
                        reference.resolution.as_ref()
                    ),
                    other => {
                        panic!("expected `List` callee in parameter annotation, found {other:?}")
                    }
                }
                let argument = arguments
                    .iter()
                    .next()
                    .copied()
                    .expect("list annotation should keep its element type");
                match &lowered.module().types()[argument].kind {
                    TypeKind::Name(reference) => assert!(
                        matches!(
                            reference.resolution.as_ref(),
                            ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Text))
                        ),
                        "expected builtin `Text` element resolution, found {:?}",
                        reference.resolution.as_ref()
                    ),
                    other => panic!("expected `Text` list element annotation, found {other:?}"),
                }
            }
            other => panic!("expected `List Text` parameter annotation, found {other:?}"),
        }

        match &lowered.module().types()[result_annotation].kind {
            TypeKind::Name(reference) => assert!(
                matches!(
                    reference.resolution.as_ref(),
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Text))
                ),
                "expected builtin `Text` result resolution, found {:?}",
                reference.resolution.as_ref()
            ),
            other => panic!("expected `Text` result annotation, found {other:?}"),
        }
    }

    #[test]
    fn preserves_real_class_constraints_while_normalizing_function_signatures() {
        let lowered = lower_text(
            "standalone-function-signature-with-constraint.aivi",
            "type Setoid Text => Text -> Bool\n\
             func same = text=> True\n",
        );
        assert!(
            !lowered.has_errors(),
            "class constraints should remain intact while normalizing function signatures: {:?}",
            lowered.diagnostics()
        );

        let function = match find_named_item(lowered.module(), "same") {
            Item::Function(item) => item,
            other => panic!("expected `same` to lower as a function item, found {other:?}"),
        };
        assert_eq!(
            function.context.len(),
            1,
            "the Setoid constraint should stay in the function context"
        );
        let constraint = function.context[0];
        match &lowered.module().types()[constraint].kind {
            TypeKind::Apply { callee, arguments } => {
                assert_eq!(arguments.len(), 1);
                match &lowered.module().types()[*callee].kind {
                    TypeKind::Name(reference) => match reference.resolution.as_ref() {
                        ResolutionState::Resolved(TypeResolution::Item(item_id)) => {
                            assert!(matches!(lowered.module().items()[*item_id], Item::Class(_)));
                        }
                        other => panic!("expected class resolution for `Setoid`, found {other:?}"),
                    },
                    other => panic!("expected `Setoid` callee in constraint, found {other:?}"),
                }
            }
            other => panic!("expected `Setoid Text` class constraint, found {other:?}"),
        }
        assert!(
            function.parameters[0].annotation.is_some(),
            "the function parameter should still receive its normalized annotation"
        );
        assert!(
            function.annotation.is_some(),
            "the result annotation should still be present after normalization"
        );
    }

    #[test]
    fn normalizes_legacy_constraint_arrow_for_applied_parameter_types() {
        let lowered = lower_text(
            "standalone-function-signature-legacy-constraint-arrow.aivi",
            "type List Text => Text\n\
             func joinEmails = items=> \"joined\"\n",
        );
        assert!(
            !lowered.has_errors(),
            "legacy formatter output should still normalize into parameter annotations: {:?}",
            lowered.diagnostics()
        );

        let function = match find_named_item(lowered.module(), "joinEmails") {
            Item::Function(item) => item,
            other => panic!("expected `joinEmails` to lower as a function item, found {other:?}"),
        };
        assert!(
            function.context.is_empty(),
            "legacy `=>` output should not leave applied parameter types in the class-constraint context"
        );
        assert!(
            function.parameters[0].annotation.is_some(),
            "legacy formatter output should still reconstruct the parameter annotation"
        );
        assert!(
            function.annotation.is_some(),
            "legacy formatter output should keep the result annotation"
        );
    }

    #[test]
    fn snake_demo_legacy_signature_lines_keep_all_parameter_annotations() {
        // Inline source that tests the "legacy signature lines" format where function
        // parameters carry type annotations. This format must survive HIR lowering.
        let lowered = lower_text(
            "demos/snake.aivi",
            r#"
fun bodyOrFoodGlyph:Text = isHead:Bool isBody:Bool isFood:Bool =>
    isHead | isBody | isFood | "."

fun cellGlyph:Text = row:Int col:Int =>
    bodyOrFoodGlyph False False False

fun rowTextStep:Text = row:Int acc:Text col:Int =>
    "{acc}{cellGlyph row col}"

fun rowText:Text = row:Int =>
    ""

fun boardTextStep:Text = acc:Text row:Int =>
    "{acc}{rowText row}"
"#,
        );
        assert!(
            !lowered.has_errors(),
            "snake demo should still lower cleanly after signature normalization: {:?}",
            lowered.diagnostics()
        );

        for name in [
            "bodyOrFoodGlyph",
            "cellGlyph",
            "rowTextStep",
            "rowText",
            "boardTextStep",
        ] {
            let function = match find_named_item(lowered.module(), name) {
                Item::Function(item) => item,
                other => panic!("expected `{name}` to lower as a function item, found {other:?}"),
            };
            assert!(
                function
                    .parameters
                    .iter()
                    .all(|parameter| parameter.annotation.is_some()),
                "expected `{name}` to keep all parameter annotations, found {:?}",
                function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.annotation.is_some())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn lowers_trailing_clusters_with_implicit_tuple_finalizers() {
        let lowered = lower_fixture("milestone-1/valid/pipes/applicative_clusters.aivi");
        assert!(
            !lowered.has_errors(),
            "cluster fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let tupled_names = find_signal(lowered.module(), "tupledNames");
        let body = tupled_names
            .body
            .expect("tupledNames signal should have a lowered cluster body");
        let cluster_id = match &lowered.module().exprs()[body].kind {
            ExprKind::Cluster(cluster) => *cluster,
            other => panic!("expected cluster expression, found {other:?}"),
        };
        assert!(
            matches!(
                lowered.module().clusters()[cluster_id].finalizer,
                ClusterFinalizer::ImplicitTuple
            ),
            "pipe-end clusters should lower with an implicit tuple finalizer"
        );
    }

    #[test]
    fn bundle_imports_do_not_hijack_builtin_option_resolution() {
        let lowered = lower_fixture("milestone-1/valid/records/record_shorthand_and_elision.aivi");
        assert!(
            !lowered.has_errors(),
            "record shorthand fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        assert!(
            lowered
                .module()
                .imports()
                .iter()
                .any(|(_, import)| import.imported_name.text() == "Option"),
            "fixture should preserve the explicit Option bundle import"
        );
        assert!(
            lowered
                .module()
                .imports()
                .iter()
                .any(|(_, import)| matches!(
                    import.metadata,
                    ImportBindingMetadata::Bundle(ImportBundleKind::BuiltinOption)
                )),
            "fixture should preserve builtin Option bundle metadata"
        );

        let option_refs = lowered
            .module()
            .types()
            .iter()
            .filter_map(|(_, ty)| match &ty.kind {
                TypeKind::Name(reference)
                    if reference.path.segments().first().text() == "Option" =>
                {
                    Some(reference)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            !option_refs.is_empty(),
            "expected Option references in the lowered HIR"
        );
        assert!(
            option_refs.iter().all(|reference| matches!(
                reference.resolution,
                ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Option))
            )),
            "Option type references should resolve to the builtin even when a bundle import exists: {option_refs:?}"
        );
    }

    #[test]
    fn use_member_imports_preserve_compiler_known_metadata() {
        let lowered = lower_fixture("milestone-2/valid/use-member-imports/main.aivi");
        assert!(
            !lowered.has_errors(),
            "use-member-imports fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let text_imports = lowered
            .module()
            .imports()
            .iter()
            .filter_map(
                |(_, import)| match (&*import.local_name.text(), &import.metadata) {
                    ("http" | "socket", ImportBindingMetadata::Value { ty }) => Some(ty),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();
        assert_eq!(
            text_imports.len(),
            2,
            "expected http/socket imports to carry compiler-known value metadata, got {:?}",
            lowered.module().imports().iter().collect::<Vec<_>>()
        );
        assert!(
            text_imports
                .iter()
                .all(|ty| matches!(ty, ImportValueType::Primitive(BuiltinType::Text))),
            "expected http/socket imports to lower as Text-valued bindings, got {text_imports:?}"
        );

        let request_import = lowered.module().imports().iter().find_map(|(_, import)| {
            match (&*import.local_name.text(), &import.metadata) {
                ("Request", ImportBindingMetadata::TypeConstructor { kind, .. }) => Some(kind),
                _ => None,
            }
        });
        assert_eq!(
            request_import,
            Some(&Kind::constructor(1)),
            "expected Request import to preserve unary constructor kind metadata"
        );

        let channel_import = lowered.module().imports().iter().find_map(|(_, import)| {
            match (&*import.local_name.text(), &import.metadata) {
                ("Channel", ImportBindingMetadata::TypeConstructor { kind, .. }) => Some(kind),
                _ => None,
            }
        });
        assert_eq!(
            channel_import,
            Some(&Kind::constructor(2)),
            "expected Channel import to preserve binary constructor kind metadata"
        );

        let imported_type_refs = lowered
            .module()
            .types()
            .iter()
            .filter_map(|(_, ty)| match &ty.kind {
                TypeKind::Name(reference)
                    if matches!(
                        reference.path.segments().first().text(),
                        "Request" | "Channel"
                    ) =>
                {
                    Some(reference)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            imported_type_refs.len(),
            2,
            "expected Request/Channel references in lowered HIR"
        );
        assert!(
            imported_type_refs.iter().all(|reference| matches!(
                reference.resolution,
                ResolutionState::Resolved(TypeResolution::Import(_))
            )),
            "imported type references should resolve through import bindings: {imported_type_refs:?}"
        );
    }

    #[test]
    fn use_member_import_aliases_preserve_local_names_and_metadata() {
        let lowered = lower_fixture("milestone-2/valid/use-member-import-aliases/main.aivi");
        assert!(
            !lowered.has_errors(),
            "use-member-import-aliases fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "use-member-import-aliases fixture should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let primary_http = lowered.module().imports().iter().find(|(_, import)| {
            import.imported_name.text() == "http" && import.local_name.text() == "primaryHttp"
        });
        assert!(
            matches!(
                primary_http.map(|(_, import)| &import.metadata),
                Some(ImportBindingMetadata::Value {
                    ty: ImportValueType::Primitive(BuiltinType::Text)
                })
            ),
            "expected aliased http import to preserve Text metadata"
        );

        let aliased_request = lowered.module().imports().iter().find(|(_, import)| {
            import.imported_name.text() == "Request" && import.local_name.text() == "HttpRequest"
        });
        assert!(
            matches!(
                aliased_request.map(|(_, import)| &import.metadata),
                Some(ImportBindingMetadata::TypeConstructor { kind, .. })
                    if kind == &Kind::constructor(1)
            ),
            "expected aliased Request import to preserve constructor kind metadata"
        );

        let imported_type_refs = lowered
            .module()
            .types()
            .iter()
            .filter_map(|(_, ty)| match &ty.kind {
                TypeKind::Name(reference)
                    if matches!(
                        reference.path.segments().first().text(),
                        "HttpRequest" | "NetworkChannel"
                    ) =>
                {
                    Some(reference)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            imported_type_refs.len(),
            2,
            "expected aliased imported type references in lowered HIR"
        );
        assert!(
            imported_type_refs.iter().all(|reference| matches!(
                reference.resolution,
                ResolutionState::Resolved(TypeResolution::Import(_))
            )),
            "aliased imported type references should still resolve through import bindings: {imported_type_refs:?}"
        );
    }

    #[test]
    fn use_db_imports_preserve_intrinsic_metadata_for_builder_helpers() {
        let lowered = lower_text(
            "db-builder-helper-imports.aivi",
            "use aivi.db (paramInt, statement)\n",
        );
        assert!(
            !lowered.has_errors(),
            "db builder import fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let imported = lowered
            .module()
            .imports()
            .iter()
            .map(|(_, import)| {
                (
                    import.imported_name.text().to_owned(),
                    import.metadata.clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            imported.get("paramInt"),
            Some(&ImportBindingMetadata::IntrinsicValue {
                value: IntrinsicValue::DbParamInt,
                ty: super::arrow_import_type(
                    super::primitive_import_type(BuiltinType::Int),
                    super::db_param_import_type(),
                ),
            })
        );
        assert_eq!(
            imported.get("statement"),
            Some(&ImportBindingMetadata::IntrinsicValue {
                value: IntrinsicValue::DbStatement,
                ty: super::arrow_import_type(
                    super::primitive_import_type(BuiltinType::Text),
                    super::arrow_import_type(
                        super::list_import_type(super::db_param_import_type()),
                        super::db_statement_import_type(),
                    ),
                ),
            })
        );
    }

    #[test]
    fn lowers_domains_with_carriers_parameters_and_members() {
        let lowered = lower_fixture("milestone-2/valid/domain-declarations/main.aivi");
        assert!(
            !lowered.has_errors(),
            "domain fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "domain fixture should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let path = match find_named_item(lowered.module(), "Path") {
            Item::Domain(item) => item,
            other => panic!("expected `Path` to lower as a domain item, found {other:?}"),
        };
        assert!(matches!(
            lowered.module().types()[path.carrier].kind,
            TypeKind::Name(ref reference)
                if matches!(
                    reference.resolution,
                    ResolutionState::Resolved(TypeResolution::Builtin(BuiltinType::Text))
                )
        ));
        assert_eq!(path.members.len(), 3);
        assert!(matches!(path.members[0].kind, DomainMemberKind::Literal));
        assert_eq!(path.members[0].name.text(), "root");
        assert!(matches!(path.members[1].kind, DomainMemberKind::Operator));
        assert_eq!(path.members[1].name.text(), "/");
        assert!(matches!(path.members[2].kind, DomainMemberKind::Method));
        assert_eq!(path.members[2].name.text(), "unwrap");

        let non_empty = match find_named_item(lowered.module(), "NonEmpty") {
            Item::Domain(item) => item,
            other => panic!("expected `NonEmpty` to lower as a domain item, found {other:?}"),
        };
        assert_eq!(non_empty.parameters.len(), 1);
    }

    #[test]
    fn lowers_instances_with_same_module_class_resolution_and_local_parameters() {
        let lowered = lower_fixture("milestone-2/valid/instance-declarations/main.aivi");
        assert!(
            !lowered.has_errors(),
            "instance fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "instance fixture should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let instance = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Instance(item) => Some(item),
                _ => None,
            })
            .expect("fixture should lower one instance item");
        assert_eq!(instance.arguments.len(), 1);
        assert!(matches!(
            instance.class.resolution,
            ResolutionState::Resolved(TypeResolution::Item(class_item))
                if matches!(&lowered.module().items()[class_item], Item::Class(class) if class.name.text() == "Eq")
        ));
        assert_eq!(instance.members.len(), 1);
        assert_eq!(instance.members[0].parameters.len(), 2);

        let ExprKind::Apply { arguments, .. } =
            &lowered.module().exprs()[instance.members[0].body].kind
        else {
            panic!("expected instance body to lower as an application");
        };
        let argument_kinds = arguments
            .iter()
            .map(|argument| match &lowered.module().exprs()[*argument].kind {
                ExprKind::Name(reference) => reference.resolution.clone(),
                other => panic!("expected local instance member arguments, found {other:?}"),
            })
            .collect::<Vec<_>>();
        assert!(argument_kinds.iter().all(|resolution| matches!(
            resolution,
            ResolutionState::Resolved(TermResolution::Local(_))
        )));
    }

    #[test]
    fn rejects_duplicate_instances_during_validation() {
        let lowered = lower_fixture("milestone-2/invalid/duplicate-instance/main.aivi");
        assert!(
            !lowered.has_errors(),
            "duplicate-instance fixture should lower cleanly before validation: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-instance"))),
            "expected duplicate-instance validation diagnostic, got {:?}",
            report.diagnostics()
        );
    }

    #[test]
    fn preserves_domain_member_ambiguity_for_contextual_resolution() {
        let lowered = lower_fixture("milestone-2/valid/domain-member-resolution/main.aivi");
        assert!(
            !lowered.has_errors(),
            "domain-member-resolution fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "domain-member-resolution fixture should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let delay = match find_named_item(lowered.module(), "delay") {
            Item::Value(item) => item,
            other => panic!("expected `delay` to lower as a value item, found {other:?}"),
        };
        let ExprKind::Apply { callee, .. } = &lowered.module().exprs()[delay.body].kind else {
            panic!("expected `delay` body to lower as an application");
        };
        let ExprKind::Name(reference) = &lowered.module().exprs()[*callee].kind else {
            panic!("expected `delay` callee to stay a name");
        };
        assert!(
            matches!(
                reference.resolution,
                ResolutionState::Resolved(TermResolution::AmbiguousDomainMembers(ref candidates))
                    if candidates.len() == 2
            ),
            "expected `make` to preserve both domain candidates for later contextual resolution, found {:?}",
            reference.resolution
        );
    }

    #[test]
    fn lowers_authored_domain_member_bindings_into_hir_members() {
        let lowered = lower_text(
            "domain-authored-members.aivi",
            r#"
type Builder = Int -> Duration

domain Duration over Int = {
    make : Builder
    make raw = raw
    unwrap : Duration -> Int
    unwrap duration = duration
}"#,
        );
        assert!(
            !lowered.has_errors(),
            "authored domain members should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let domain = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Domain(item) => Some(item),
                _ => None,
            })
            .expect("fixture should lower one domain item");
        assert_eq!(domain.members.len(), 2);
        assert_eq!(domain.members[0].parameters.len(), 1);
        assert!(domain.members[0].body.is_some());
        assert_eq!(domain.members[1].parameters.len(), 1);
        assert!(domain.members[1].body.is_some());
    }

    #[test]
    fn resolves_suffixed_integers_to_domain_literal_declarations() {
        let lowered = lower_fixture("milestone-2/valid/domain-literal-suffixes/main.aivi");
        assert!(
            !lowered.has_errors(),
            "domain literal suffix fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "domain literal suffix fixture should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let duration_domain_id = lowered
            .module()
            .root_items()
            .iter()
            .copied()
            .find(|item_id| {
                matches!(
                    &lowered.module().items()[*item_id],
                    Item::Domain(item) if item.name.text() == "Duration"
                )
            })
            .expect("fixture should contain Duration domain");

        let delay_body = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Value(item) if item.name.text() == "delay" => Some(item.body),
                _ => None,
            })
            .expect("fixture should contain delay value");

        match &lowered.module().exprs()[delay_body].kind {
            ExprKind::SuffixedInteger(literal) => {
                assert_eq!(&*literal.raw, "250");
                assert_eq!(literal.suffix.text(), "ms");
                assert_eq!(
                    literal.resolution,
                    ResolutionState::Resolved(LiteralSuffixResolution {
                        domain: duration_domain_id,
                        member_index: 0,
                    })
                );
            }
            other => panic!("expected suffixed integer expression, found {other:?}"),
        }
    }

    #[test]
    fn lowers_builtin_noninteger_literals_and_preserves_raw_spelling() {
        let lowered = lower_text(
            "builtin-noninteger-literals.aivi",
            "value pi:Float = 3.14\n\
             value amount:Decimal = 19.25d\n\
             value whole:Decimal = 19d\n\
             value count:BigInt = 123n\n",
        );
        assert!(
            !lowered.has_errors(),
            "builtin noninteger literal source should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let Item::Value(pi) = find_named_item(lowered.module(), "pi") else {
            panic!("expected `pi` to be a value item");
        };
        assert!(matches!(
            &lowered.module().exprs()[pi.body].kind,
            ExprKind::Float(literal) if &*literal.raw == "3.14"
        ));

        let Item::Value(amount) = find_named_item(lowered.module(), "amount") else {
            panic!("expected `amount` to be a value item");
        };
        assert!(matches!(
            &lowered.module().exprs()[amount.body].kind,
            ExprKind::Decimal(literal) if &*literal.raw == "19.25d"
        ));

        let Item::Value(whole) = find_named_item(lowered.module(), "whole") else {
            panic!("expected `whole` to be a value item");
        };
        assert!(matches!(
            &lowered.module().exprs()[whole.body].kind,
            ExprKind::Decimal(literal) if &*literal.raw == "19d"
        ));

        let Item::Value(count) = find_named_item(lowered.module(), "count") else {
            panic!("expected `count` to be a value item");
        };
        assert!(matches!(
            &lowered.module().exprs()[count.body].kind,
            ExprKind::BigInt(literal) if &*literal.raw == "123n"
        ));
    }

    #[test]
    fn lowers_map_and_set_literals() {
        let lowered = lower_text(
            "map-set-literals.aivi",
            "value headers = Map { \"x\": 1, \"y\": 2 }\nvalue tags = Set [\"a\", \"b\"]\n",
        );
        assert!(
            !lowered.has_errors(),
            "map/set literal source should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let headers_body = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Value(item) if item.name.text() == "headers" => Some(item.body),
                _ => None,
            })
            .expect("fixture should contain headers value");
        match &lowered.module().exprs()[headers_body].kind {
            ExprKind::Map(map) => {
                assert_eq!(map.entries.len(), 2);
                assert!(matches!(
                    lowered.module().exprs()[map.entries[0].key].kind,
                    ExprKind::Text(_)
                ));
                assert!(matches!(
                    lowered.module().exprs()[map.entries[0].value].kind,
                    ExprKind::Integer(_)
                ));
            }
            other => panic!("expected map literal expression, found {other:?}"),
        }

        let tags_body = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Value(item) if item.name.text() == "tags" => Some(item.body),
                _ => None,
            })
            .expect("fixture should contain tags value");
        match &lowered.module().exprs()[tags_body].kind {
            ExprKind::Set(elements) => {
                assert_eq!(elements.len(), 2);
                assert!(matches!(
                    lowered.module().exprs()[elements[0]].kind,
                    ExprKind::Text(_)
                ));
            }
            other => panic!("expected set literal expression, found {other:?}"),
        }
    }

    #[test]
    fn duplicate_map_keys_report_hir_diagnostics() {
        let lowered = lower_text(
            "duplicate-map-key.aivi",
            "value headers = Map { \"Authorization\": \"a\", \"Authorization\": \"b\" }\n",
        );
        assert!(
            lowered.has_errors(),
            "duplicate map key should fail lowering"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-map-key"))),
            "expected duplicate-map-key diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn duplicate_record_fields_report_hir_diagnostics() {
        let lowered = lower_text(
            "duplicate-record-field.aivi",
            "type User = { name: Text }\nvalue user:User = { name: \"Ada\", name: \"Grace\" }\n",
        );
        assert!(
            lowered.has_errors(),
            "duplicate record fields should fail lowering"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-record-field"))),
            "expected duplicate-record-field diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn duplicate_record_type_fields_report_hir_diagnostics() {
        let lowered = lower_text(
            "duplicate-record-type-field.aivi",
            "type User = { name: Text, age: Int, name: Bool }\n",
        );
        assert!(
            lowered.has_errors(),
            "duplicate record type fields should fail lowering"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-record-field"))),
            "expected duplicate-record-field diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn duplicate_record_pattern_fields_report_hir_diagnostics() {
        let lowered = lower_text(
            "duplicate-record-pattern-field.aivi",
            "type User = { name: Text }\nfun extract:Text = user:User =>\n    user\n     ||> { name, name } -> name\n",
        );
        assert!(
            lowered.has_errors(),
            "duplicate record pattern fields should fail lowering"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == Some(super::code("duplicate-record-field"))),
            "expected duplicate-record-field diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }

    #[test]
    fn duplicate_set_elements_warn_and_canonicalize() {
        let lowered = lower_text(
            "duplicate-set-element.aivi",
            "value tags = Set [\"news\", \"featured\", \"news\"]\n",
        );
        assert!(
            !lowered.has_errors(),
            "duplicate set elements should canonicalize without a lowering error: {:?}",
            lowered.diagnostics()
        );
        assert!(lowered.diagnostics().iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.code == Some(super::code("duplicate-set-element"))
        }));
        let tags_body = lowered
            .module()
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Value(item) if item.name.text() == "tags" => Some(item.body),
                _ => None,
            })
            .expect("fixture should contain tags value");
        match &lowered.module().exprs()[tags_body].kind {
            ExprKind::Set(elements) => {
                assert_eq!(elements.len(), 2, "set literal should be canonicalized");
                assert!(matches!(
                    lowered.module().exprs()[elements[0]].kind,
                    ExprKind::Text(_)
                ));
                assert!(matches!(
                    lowered.module().exprs()[elements[1]].kind,
                    ExprKind::Text(_)
                ));
            }
            other => panic!("expected set literal expression, found {other:?}"),
        }
    }

    #[test]
    fn exports_can_target_constructors_through_parent_type_items() {
        let lowered = lower_text(
            "constructor-export.aivi",
            "type Status = Idle | Busy\nexport Idle\n",
        );
        assert!(
            !lowered.has_errors(),
            "constructor export source should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "constructor export should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let export = lowered
            .module()
            .root_items()
            .iter()
            .find_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Export(item) => Some(item),
                _ => None,
            })
            .expect("constructor-export source should contain one export item");

        let resolved = match export.resolution {
            ResolutionState::Resolved(item) => item,
            ResolutionState::Unresolved => panic!("constructor export should resolve"),
        };
        let ExportResolution::Item(resolved) = resolved else {
            panic!("constructor export should resolve to the parent type item");
        };
        match &lowered.module().items()[resolved] {
            Item::Type(item) => assert_eq!(item.name.text(), "Status"),
            other => {
                panic!("constructor export should resolve to the parent type item, found {other:?}")
            }
        }
    }

    #[test]
    fn grouped_exports_lower_to_individual_resolved_hir_items() {
        let lowered = lower_text(
            "grouped-export.aivi",
            "type Status = Idle | Busy\nvalue main = Idle\nexport (Idle, main)\n",
        );
        assert!(
            !lowered.has_errors(),
            "grouped export source should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "grouped export source should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let exports = lowered
            .module()
            .root_items()
            .iter()
            .filter_map(|item_id| match &lowered.module().items()[*item_id] {
                Item::Export(item) => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            exports.len(),
            2,
            "grouped export should lower to two HIR export items"
        );
        assert_eq!(
            exports
                .iter()
                .map(|export| export.target.segments().first().text())
                .collect::<Vec<_>>(),
            vec!["Idle", "main"]
        );

        let exported_names = crate::exports::exports(lowered.module());
        assert!(exported_names.find("main").is_some());
        assert!(exported_names.find("Idle").is_some());
        assert!(exported_names.find("Status").is_none());
    }

    #[test]
    fn exports_support_builtin_and_ambient_root_surface_targets() {
        let lowered = lower_text(
            "builtin-export.aivi",
            "export (Int, Option, Some, Eq, Foldable)\n",
        );
        assert!(
            !lowered.has_errors(),
            "builtin export source should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "builtin export source should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let exported_names = exports(lowered.module());
        assert_eq!(
            exported_names
                .find("Int")
                .map(|exported| &exported.metadata),
            Some(&ImportBindingMetadata::BuiltinType(BuiltinType::Int))
        );
        assert_eq!(
            exported_names
                .find("Option")
                .map(|exported| &exported.metadata),
            Some(&ImportBindingMetadata::BuiltinType(BuiltinType::Option))
        );
        assert_eq!(
            exported_names
                .find("Some")
                .map(|exported| &exported.metadata),
            Some(&ImportBindingMetadata::BuiltinTerm(BuiltinTerm::Some))
        );
        assert_eq!(
            exported_names.find("Eq").map(|exported| &exported.metadata),
            Some(&ImportBindingMetadata::AmbientType)
        );
        assert_eq!(
            exported_names
                .find("Foldable")
                .map(|exported| &exported.metadata),
            Some(&ImportBindingMetadata::AmbientType)
        );
    }

    #[test]
    fn local_module_definitions_shadow_builtins() {
        let lowered = lower_text(
            "builtin-shadowing.aivi",
            concat!(
                "value True = 0\n",
                "value chosen = True\n",
                "type Option = Option Int\n",
                "value wrapped:Option = Option 1\n",
            ),
        );
        assert!(
            !lowered.has_errors(),
            "builtin shadowing source should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "builtin shadowing source should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let chosen = match find_named_item(lowered.module(), "chosen") {
            Item::Value(item) => item,
            other => panic!("expected chosen to be a value item, found {other:?}"),
        };
        let chosen_resolution = match &lowered.module().exprs()[chosen.body].kind {
            ExprKind::Name(reference) => &reference.resolution,
            other => panic!("expected chosen body to be a name, found {other:?}"),
        };
        assert!(
            matches!(
                chosen_resolution,
                ResolutionState::Resolved(TermResolution::Item(_))
            ),
            "local term definitions should shadow builtin terms: {chosen_resolution:?}"
        );

        let wrapped = match find_named_item(lowered.module(), "wrapped") {
            Item::Value(item) => item,
            other => panic!("expected wrapped to be a value item, found {other:?}"),
        };
        let annotation = wrapped
            .annotation
            .expect("wrapped should preserve its type annotation");
        let annotation_resolution = match &lowered.module().types()[annotation].kind {
            TypeKind::Name(reference) => &reference.resolution,
            other => panic!("expected wrapped annotation to be a name, found {other:?}"),
        };
        assert!(
            matches!(
                annotation_resolution,
                ResolutionState::Resolved(TypeResolution::Item(_))
            ),
            "local type definitions should shadow builtin types: {annotation_resolution:?}"
        );
    }

    #[test]
    fn lowers_result_blocks_into_nested_result_case_pipes() {
        let lowered = lower_fixture("milestone-2/valid/result-block/main.aivi");
        assert!(
            !lowered.has_errors(),
            "result block fixture should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let report = lowered
            .module()
            .validate(ValidationMode::RequireResolvedNames);
        assert!(
            report.is_ok(),
            "result block fixture should validate as resolved HIR: {:?}",
            report.diagnostics()
        );

        let combined = match find_named_item(lowered.module(), "combined") {
            Item::Value(item) => item,
            other => panic!("expected combined to be a value item, found {other:?}"),
        };
        let ExprKind::Pipe(outer_pipe) = &lowered.module().exprs()[combined.body].kind else {
            panic!("expected combined body to lower into a pipe");
        };
        let outer_stages = outer_pipe.stages.iter().collect::<Vec<_>>();
        assert_eq!(
            outer_stages.len(),
            2,
            "each binding should lower into Ok/Err case arms"
        );
        assert!(matches!(outer_stages[0].kind, PipeStageKind::Case { .. }));
        assert!(matches!(outer_stages[1].kind, PipeStageKind::Case { .. }));

        let PipeStageKind::Case {
            body: inner_body, ..
        } = &outer_stages[0].kind
        else {
            panic!("expected first outer stage to be an Ok case arm");
        };
        let ExprKind::Pipe(inner_pipe) = &lowered.module().exprs()[*inner_body].kind else {
            panic!("expected Ok branch to continue with the nested result binding");
        };
        assert_eq!(inner_pipe.stages.iter().count(), 2);

        let rejected = match find_named_item(lowered.module(), "rejected") {
            Item::Value(item) => item,
            other => panic!("expected rejected to be a value item, found {other:?}"),
        };
        let ExprKind::Pipe(rejected_pipe) = &lowered.module().exprs()[rejected.body].kind else {
            panic!("expected rejected body to lower into a pipe");
        };
        let rejected_stages = rejected_pipe.stages.iter().collect::<Vec<_>>();
        // `rejected` has two bindings and no explicit tail; the outer Ok(left) branch continues
        // into the inner pipe for `right <- requirePositive 22`, whose Ok(right) branch
        // carries the implicit `Ok right` constructor application.
        let PipeStageKind::Case {
            body: outer_ok_body,
            ..
        } = &rejected_stages[0].kind
        else {
            panic!("expected rejected outer Ok branch");
        };
        let ExprKind::Pipe(inner_rejected_pipe) = &lowered.module().exprs()[*outer_ok_body].kind
        else {
            panic!("expected outer Ok branch to continue into inner pipe for the second binding");
        };
        let inner_rejected_stages = inner_rejected_pipe.stages.iter().collect::<Vec<_>>();
        let PipeStageKind::Case {
            body: implicit_tail,
            ..
        } = &inner_rejected_stages[0].kind
        else {
            panic!("expected inner Ok branch to be the implicit tail");
        };
        let ExprKind::Apply { .. } = &lowered.module().exprs()[*implicit_tail].kind else {
            panic!("implicit result tails should lower into an `Ok ...` constructor application");
        };
    }

    #[test]
    fn normalizer_does_not_treat_constructor_type_as_class_constraint() {
        // Standalone type annotations starting with (List A) -> must be parsed as
        // function types, NOT as (List A) => ... constraints.
        // This was a bug: consume_constraint_separator accepted both => and ->.
        let lowered = lower_text(
            "constructor-type-not-constraint.aivi",
            "type (List A) -> (Option A) -> (List A)\n\
             func appendPrev = items prev => items\n",
        );
        assert!(
            !lowered.has_errors(),
            "function with (List A) -> parameter type should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let function = match find_named_item(lowered.module(), "appendPrev") {
            Item::Function(item) => item,
            other => panic!("expected function, got {other:?}"),
        };
        assert!(
            function.context.is_empty(),
            "no class constraints — (List A) is a constructor, not a class"
        );
        assert_eq!(
            function.parameters.len(),
            2,
            "function should have 2 parameters from the normalized annotation"
        );
        assert!(
            function.parameters[0].annotation.is_some(),
            "first parameter should receive a List A annotation"
        );
        assert!(
            function.parameters[1].annotation.is_some(),
            "second parameter should receive an Option A annotation"
        );
    }

    #[test]
    fn ambient_matrix_at_row_has_correct_list_input_type() {
        // __aivi_matrix_atRow takes Option(List A) and returns Option A,
        // not Option A -> Int -> Option A (which was the old incorrect signature).
        let lowered = lower_text(
            "matrix-at-row-type.aivi",
            "use aivi.matrix (Matrix)\n\
             value x:Int = 1\n",
        );
        assert!(
            !lowered.has_errors(),
            "matrix import should lower cleanly: {:?}",
            lowered.diagnostics()
        );

        let function = lowered
            .module()
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Function(f) if f.name.text() == "__aivi_matrix_atRow" => Some(f),
                _ => None,
            })
            .expect("ambient prelude should contain __aivi_matrix_atRow");
        assert_eq!(
            function.parameters.len(),
            2,
            "atRow should have 2 parameters: rowOpt and x"
        );
    }

    #[test]
    fn hoist_item_lowers_to_hir_with_module_path() {
        let lowered = lower_text(
            "hoist-basic.aivi",
            "hoist aivi.list\n",
        );
        // The module path can't be resolved without a real resolver, which is expected.
        // What we verify is that the hoist item itself was lowered into the HIR correctly.
        let hoist_item = lowered
            .module()
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Hoist(h) => Some(h.clone()),
                _ => None,
            })
            .expect("lowered module should contain a hoist item");
        let segs = hoist_item.module.segments();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs.first().text(), "aivi");
        assert_eq!(segs.last().text(), "list");
        assert!(hoist_item.kind_filters.is_empty());
        assert!(hoist_item.hiding.is_empty());
    }

    #[test]
    fn hoist_item_lowers_kind_filters_correctly() {
        let lowered = lower_text(
            "hoist-filters.aivi",
            "hoist aivi.list (func, value)\n",
        );
        let hoist_item = lowered
            .module()
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Hoist(h) => Some(h.clone()),
                _ => None,
            })
            .expect("lowered module should contain a hoist item");
        assert_eq!(hoist_item.kind_filters.len(), 2);
        assert!(matches!(hoist_item.kind_filters[0], HoistKindFilter::Func));
        assert!(matches!(hoist_item.kind_filters[1], HoistKindFilter::Value));
    }

    #[test]
    fn hoist_item_lowers_hiding_list_correctly() {
        let lowered = lower_text(
            "hoist-hiding.aivi",
            "hoist aivi.list hiding (length, head)\n",
        );
        let hoist_item = lowered
            .module()
            .items()
            .iter()
            .find_map(|(_, item)| match item {
                Item::Hoist(h) => Some(h.clone()),
                _ => None,
            })
            .expect("lowered module should contain a hoist item");
        assert!(hoist_item.kind_filters.is_empty());
        assert_eq!(hoist_item.hiding.len(), 2);
        assert_eq!(hoist_item.hiding[0].text(), "length");
        assert_eq!(hoist_item.hiding[1].text(), "head");
    }

    #[test]
    fn hoist_item_emits_diagnostic_for_unknown_kind_filter() {
        let lowered = lower_text(
            "hoist-bad-filter.aivi",
            "hoist aivi.list (funky)\n",
        );
        assert!(
            lowered.has_errors(),
            "hoist with invalid kind filter should emit a diagnostic"
        );
        assert!(
            lowered
                .diagnostics()
                .iter()
                .any(|d| d.code.as_ref().is_some_and(|c| c.name() == "unknown-hoist-kind-filter")),
            "expected unknown-hoist-kind-filter diagnostic, got {:?}",
            lowered.diagnostics()
        );
    }
}
