#![forbid(unsafe_code)]

//! Milestone 1 surface frontend: lexer, CST, parser, and formatter.

pub mod cst;
pub mod format;
pub mod lex;
pub mod parse;

pub use cst::{
    BinaryOperator, ClassBody, ClassMember, ClassMemberName, Decorator, DecoratorArguments,
    DecoratorPayload, DomainBody, DomainItem, DomainMember, DomainMemberName, ErrorItem,
    ExportItem, Expr, ExprKind, FunctionParam, Identifier, IntegerLiteral, Item, ItemBase,
    ItemKind, MarkupAttribute, MarkupAttributeValue, MarkupNode, Module, NamedItem, NamedItemBody,
    OperatorName, Pattern, PatternKind, PipeCaseArm, PipeExpr, PipeStage, PipeStageKind,
    ProjectionPath, QualifiedName, RecordExpr, RecordField, RecordPatternField, RegexLiteral,
    SourceDecorator, SourceProviderContractBody, SourceProviderContractFieldValue,
    SourceProviderContractItem, SourceProviderContractMember, SourceProviderContractSchemaMember,
    SuffixedIntegerLiteral, TextFragment, TextInterpolation, TextLiteral, TextSegment, TokenRange,
    TypeDeclBody, TypeExpr, TypeExprKind, TypeField, TypeVariant, UnaryOperator, UseItem,
};
pub use format::Formatter;
pub use lex::{LexedModule, Token, TokenKind, lex_module};
pub use parse::{ParsedModule, parse_module};
