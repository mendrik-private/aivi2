use aivi_base::{Diagnostic, Severity, SourceFile, SourceSpan, Span};

use crate::{
    cst::{
        BigIntLiteral, BinaryOperator, ClassBody, ClassMember, ClassMemberName, ClassRequireDecl,
        ClassWithDecl, DecimalLiteral, Decorator, DecoratorArguments, DecoratorPayload, DomainBody,
        DomainItem, DomainMember, DomainMemberName, ErrorItem, ExportItem, Expr, ExprKind,
        FloatLiteral, FromEntry, FromItem, FunctionParam, FunctionSurfaceForm, Identifier,
        InstanceBody, InstanceItem, InstanceMember, IntegerLiteral, Item, ItemBase, MapExpr,
        MapExprEntry, MarkupAttribute, MarkupAttributeValue, MarkupNode, Module, NamedItem,
        NamedItemBody, OperatorName, PatchBlock, PatchEntry, PatchInstruction,
        PatchInstructionKind, PatchSelector, PatchSelectorSegment, Pattern, PatternKind,
        PipeCaseArm, PipeExpr, PipeStage, PipeStageKind, ProjectionPath, QualifiedName, RecordExpr,
        RecordField, RecordPatternField, RegexLiteral, ResultBinding, ResultBlockExpr,
        SignalMergeBody, SignalReactiveArm, SourceDecorator, SourceProviderContractBody,
        SourceProviderContractFieldValue, SourceProviderContractItem, SourceProviderContractMember,
        SourceProviderContractSchemaMember, SuffixedIntegerLiteral, TextFragment,
        TextInterpolation, TextLiteral, TextSegment, TokenRange, TypeCompanionMember, TypeDeclBody,
        TypeExpr, TypeExprKind, TypeField, TypeSumBody, TypeVariant, TypeVariantField,
        UnaryOperator, UseImport, UseItem,
    },
    lex::{LexedModule, Token, TokenKind, lex_fragment, lex_module},
};

use crate::codes::*;

const MAX_PARSE_DEPTH: usize = 256;
const IMPLICIT_FUNCTION_SUBJECT_NAME: &str = "arg1";

#[derive(Clone, Debug)]
struct SubjectPickHead {
    expr: Expr,
    start_index: usize,
}

/// Parser output retaining the lossless token buffer and recoverable diagnostics.
#[derive(Clone, Debug)]
pub struct ParsedModule {
    pub lexed: LexedModule,
    pub module: Module,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedModule {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn all_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.lexed
            .diagnostics()
            .iter()
            .chain(self.diagnostics.iter())
    }

    pub fn has_errors(&self) -> bool {
        self.all_diagnostics()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

pub fn parse_module(source: &SourceFile) -> ParsedModule {
    let lexed = lex_module(source);
    let parser = Parser::new(source, lexed.tokens());
    let (module, diagnostics) = parser.parse();
    ParsedModule {
        lexed,
        module,
        diagnostics,
    }
}

struct Parser<'a> {
    source: &'a SourceFile,
    tokens: &'a [Token],
    cursor: usize,
    diagnostics: Vec<Diagnostic>,
    depth: usize,
}

#[derive(Clone, Debug)]
struct PendingTypeAnnotation {
    span: SourceSpan,
    constraints: Vec<TypeExpr>,
    annotation: TypeExpr,
}

include!("top_level.rs");
include!("decorators.rs");
include!("functions.rs");
include!("types.rs");
include!("expr.rs");
include!("pattern.rs");
include!("helpers.rs");
include!("stops.rs");

#[cfg(test)]
mod tests;
