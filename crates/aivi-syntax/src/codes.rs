//! Named constants for all syntax diagnostic codes.
//! Use these instead of inline `DiagnosticCode::new()` literals.
use aivi_base::DiagnosticCode;

pub const DANGLING_DECORATOR_BLOCK: DiagnosticCode =
    DiagnosticCode::new("syntax", "dangling-decorator-block");
pub const DIRECT_FUNCTION_PARAMETER_ANNOTATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "direct-function-parameter-annotation");
pub const DUPLICATE_STANDALONE_TYPE_ANNOTATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "duplicate-standalone-type-annotation");
pub const EMPTY_RESULT_BLOCK: DiagnosticCode = DiagnosticCode::new("syntax", "empty-result-block");
pub const INVALID_DISCARD_EXPR: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-discard-expr");
pub const INVALID_SUBJECT_PICK: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-subject-pick");
pub const INVALID_ESCAPE_SEQUENCE: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-escape-sequence");
pub const INVALID_MARKUP_CHILD_CONTENT: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-markup-child-content");
pub const INVALID_TEXT_INTERPOLATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "invalid-text-interpolation");
pub const MISMATCHED_MARKUP_CLOSE: DiagnosticCode =
    DiagnosticCode::new("syntax", "mismatched-markup-close");
pub const MISSING_CLASS_MEMBER_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-class-member-type");
pub const MISSING_CLASS_OPEN_BRACE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-class-open-brace");
pub const MISSING_CLASS_REQUIRE_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-class-require-type");
pub const MISSING_CLASS_WITH_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-class-with-type");
pub const MISSING_DECLARATION_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-declaration-body");
pub const MISSING_DECORATOR_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-decorator-name");
pub const MISSING_DOMAIN_CARRIER: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-carrier");
pub const MISSING_DOMAIN_MEMBER_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-member-body");
pub const MISSING_DOMAIN_MEMBER_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-member-name");
pub const MISSING_DOMAIN_MEMBER_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-member-type");
pub const MISSING_DOMAIN_OPEN_BRACE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-open-brace");
pub const MISSING_DOMAIN_OVER: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-domain-over");
pub const MISSING_EXPORT_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-export-name");
pub const MISSING_FROM_ENTRY_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-from-entry-body");
pub const MISSING_FROM_OPEN_BRACE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-from-open-brace");
pub const MISSING_FROM_SOURCE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-from-source");
pub const MISSING_INSTANCE_CLASS: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-class");
pub const MISSING_INSTANCE_MEMBER_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-member-body");
pub const MISSING_INSTANCE_OPEN_BRACE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-open-brace");
pub const MISSING_INSTANCE_TARGET: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-instance-target");
pub const MISSING_ITEM_NAME: DiagnosticCode = DiagnosticCode::new("syntax", "missing-item-name");
pub const MISSING_PIPE_MEMO_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-pipe-memo-name");
pub const MISSING_PROVIDER_CONTRACT_MEMBER_VALUE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-member-value");
pub const MISSING_PROVIDER_CONTRACT_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-name");
pub const MISSING_PROVIDER_CONTRACT_SCHEMA_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-schema-name");
pub const MISSING_PROVIDER_CONTRACT_SCHEMA_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-provider-contract-schema-type");
pub const MISSING_REACTIVE_UPDATE_ARM_ARROW: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-arm-arrow");
pub const MISSING_REACTIVE_UPDATE_ARM_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-arm-body");
pub const MISSING_REACTIVE_UPDATE_ARM_LEFT_ARROW: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-arm-left-arrow");
pub const MISSING_REACTIVE_UPDATE_ARM_PATTERN: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-arm-pattern");
pub const MISSING_REACTIVE_UPDATE_ARM_TARGET: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-arm-target");
pub const MISSING_REACTIVE_UPDATE_ARROW: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-arrow");
pub const MISSING_REACTIVE_UPDATE_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-body");
pub const MISSING_REACTIVE_UPDATE_GUARD: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-guard");
pub const MISSING_REACTIVE_UPDATE_LEFT_ARROW: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-left-arrow");
pub const MISSING_REACTIVE_UPDATE_SOURCE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-source");
pub const MISSING_REACTIVE_UPDATE_SOURCE_PATTERN: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-source-pattern");
pub const MISSING_REACTIVE_UPDATE_SUBJECT: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-subject");
pub const MISSING_REACTIVE_UPDATE_TARGET: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-reactive-update-target");
pub const MISSING_RESULT_BINDING_EXPR: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-result-binding-expr");
pub const MISSING_RESULT_BLOCK_TAIL: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-result-block-tail");
pub const MISSING_STANDALONE_TYPE_ANNOTATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-standalone-type-annotation");
pub const MISSING_TYPE_COMPANION_BODY: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-type-companion-body");
pub const MISSING_TYPE_COMPANION_NAME: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-type-companion-name");
pub const MISSING_TYPE_COMPANION_TYPE: DiagnosticCode =
    DiagnosticCode::new("syntax", "missing-type-companion-type");
pub const MISSING_USE_ALIAS: DiagnosticCode = DiagnosticCode::new("syntax", "missing-use-alias");
pub const MISSING_USE_PATH: DiagnosticCode = DiagnosticCode::new("syntax", "missing-use-path");
pub const NULLARY_FUNCTION_DECLARATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "nullary-function-declaration");
pub const ORPHAN_STANDALONE_TYPE_ANNOTATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "orphan-standalone-type-annotation");
pub const PARSE_DEPTH_EXCEEDED: DiagnosticCode =
    DiagnosticCode::new("syntax", "parse-depth-exceeded");
pub const REMOVED_TEMPORAL_PIPE_OPERATOR: DiagnosticCode =
    DiagnosticCode::new("syntax", "removed-temporal-pipe-operator");
pub const TRAILING_DECLARATION_BODY_TOKEN: DiagnosticCode =
    DiagnosticCode::new("syntax", "trailing-declaration-body-token");
pub const UNEXPECTED_CHARACTER: DiagnosticCode =
    DiagnosticCode::new("syntax", "unexpected-character");
pub const UNEXPECTED_TOKEN: DiagnosticCode = DiagnosticCode::new("syntax", "unexpected-token");
pub const UNEXPECTED_TOP_LEVEL_TOKEN: DiagnosticCode =
    DiagnosticCode::new("syntax", "unexpected-top-level-token");
pub const UNSUPPORTED_CLASS_HEAD_CONSTRAINTS: DiagnosticCode =
    DiagnosticCode::new("syntax", "unsupported-class-head-constraints");
pub const UNTERMINATED_MARKUP_NODE: DiagnosticCode =
    DiagnosticCode::new("syntax", "unterminated-markup-node");
pub const UNTERMINATED_REGEX: DiagnosticCode = DiagnosticCode::new("syntax", "unterminated-regex");
pub const UNTERMINATED_STRING: DiagnosticCode =
    DiagnosticCode::new("syntax", "unterminated-string");
pub const UNTERMINATED_TEXT_INTERPOLATION: DiagnosticCode =
    DiagnosticCode::new("syntax", "unterminated-text-interpolation");
