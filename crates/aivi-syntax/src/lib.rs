#![forbid(unsafe_code)]

//! Milestone 1 surface frontend: lexer, CST, parser, and formatter.

pub mod cst;
pub mod format;
pub mod lex;
pub mod parse;

pub use cst::{
    BigIntLiteral, BinaryOperator, ClassBody, ClassMember, ClassMemberName, DecimalLiteral,
    Decorator, DecoratorArguments, DecoratorPayload, DomainBody, DomainItem, DomainMember,
    DomainMemberName, ErrorItem, ExportItem, Expr, ExprKind, FloatLiteral, FunctionParam,
    Identifier, InstanceBody, InstanceItem, InstanceMember, IntegerLiteral, Item, ItemBase,
    ItemKind, MapExpr, MapExprEntry, MarkupAttribute, MarkupAttributeValue, MarkupNode, Module,
    NamedItem, NamedItemBody, OperatorName, Pattern, PatternKind, PipeCaseArm, PipeExpr, PipeStage,
    PipeStageKind, ProjectionPath, QualifiedName, RecordExpr, RecordField, RecordPatternField,
    RegexLiteral, ResultBinding, ResultBlockExpr, SourceDecorator, SourceProviderContractBody,
    SourceProviderContractFieldValue, SourceProviderContractItem, SourceProviderContractMember,
    SourceProviderContractSchemaMember, SuffixedIntegerLiteral, TextFragment, TextInterpolation,
    TextLiteral, TextSegment, TokenRange, TypeDeclBody, TypeExpr, TypeExprKind, TypeField,
    TypeVariant, UnaryOperator, UseImport, UseItem,
};
pub use format::Formatter;
pub use lex::{LexedModule, Token, TokenKind, lex_module};
pub use parse::{ParsedModule, parse_module};
