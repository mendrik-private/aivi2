//! Named constants for all HIR diagnostic codes.
//! Use these instead of inline `DiagnosticCode::new()` literals.
use aivi_base::DiagnosticCode;

pub const APPLICATIVE_CLUSTER_MISMATCH: DiagnosticCode = DiagnosticCode::new("hir", "applicative-cluster-mismatch");
pub const CASE_BRANCH_TYPE_MISMATCH: DiagnosticCode = DiagnosticCode::new("hir", "case-branch-type-mismatch");
pub const CIRCULAR_SIGNAL_DEPENDENCY: DiagnosticCode = DiagnosticCode::new("hir", "circular-signal-dependency");
pub const FANOUT_SUBJECT_NOT_LIST: DiagnosticCode = DiagnosticCode::new("hir", "fanout-subject-not-list");
pub const INVALID_BINARY_OPERATOR: DiagnosticCode = DiagnosticCode::new("hir", "invalid-binary-operator");
pub const INVALID_FANIN_PROJECTION: DiagnosticCode = DiagnosticCode::new("hir", "invalid-fanin-projection");
pub const INVALID_PIPE_STAGE_INPUT: DiagnosticCode = DiagnosticCode::new("hir", "invalid-pipe-stage-input");
pub const INVALID_PROJECTION: DiagnosticCode = DiagnosticCode::new("hir", "invalid-projection");
pub const INVALID_REGEX_LITERAL: DiagnosticCode = DiagnosticCode::new("hir", "invalid-regex-literal");
pub const INVALID_TRUTHY_FALSY_PROJECTION: DiagnosticCode = DiagnosticCode::new("hir", "invalid-truthy-falsy-projection");
pub const INVALID_TYPE_APPLICATION: DiagnosticCode = DiagnosticCode::new("hir", "invalid-type-application");
pub const INVALID_UNARY_OPERATOR: DiagnosticCode = DiagnosticCode::new("hir", "invalid-unary-operator");
pub const MISSING_DEFAULT_INSTANCE: DiagnosticCode = DiagnosticCode::new("hir", "missing-default-instance");
pub const MISSING_EQ_INSTANCE: DiagnosticCode = DiagnosticCode::new("hir", "missing-eq-instance");
pub const MISSING_INSTANCE_REQUIREMENT: DiagnosticCode = DiagnosticCode::new("hir", "missing-instance-requirement");
pub const NON_EXHAUSTIVE_CASE_PATTERN: DiagnosticCode = DiagnosticCode::new("hir", "non-exhaustive-case-pattern");
pub const REACTIVE_UPDATE_SELF_REFERENCE: DiagnosticCode = DiagnosticCode::new("hir", "reactive-update-self-reference");
pub const RECORD_ROW_RENAME_COLLISION: DiagnosticCode = DiagnosticCode::new("hir", "record-row-rename-collision");
pub const RECORD_ROW_TRANSFORM_SOURCE: DiagnosticCode = DiagnosticCode::new("hir", "record-row-transform-source");
pub const RESULT_BLOCK_BINDING_NOT_RESULT: DiagnosticCode = DiagnosticCode::new("hir", "result-block-binding-not-result");
pub const RESULT_BLOCK_ERROR_MISMATCH: DiagnosticCode = DiagnosticCode::new("hir", "result-block-error-mismatch");
pub const SOURCE_OPTION_TYPE_MISMATCH: DiagnosticCode = DiagnosticCode::new("hir", "source-option-type-mismatch");
pub const SOURCE_OPTION_UNBOUND_CONTRACT_PARAMETER: DiagnosticCode = DiagnosticCode::new("hir", "source-option-unbound-contract-parameter");
pub const TRUTHY_FALSY_BRANCH_TYPE_MISMATCH: DiagnosticCode = DiagnosticCode::new("hir", "truthy-falsy-branch-type-mismatch");
pub const TRUTHY_FALSY_SUBJECT_NOT_CANONICAL: DiagnosticCode = DiagnosticCode::new("hir", "truthy-falsy-subject-not-canonical");
pub const TYPE_MISMATCH: DiagnosticCode = DiagnosticCode::new("hir", "type-mismatch");
pub const UNKNOWN_PROJECTION_FIELD: DiagnosticCode = DiagnosticCode::new("hir", "unknown-projection-field");
pub const UNKNOWN_RECORD_ROW_FIELD: DiagnosticCode = DiagnosticCode::new("hir", "unknown-record-row-field");
pub const UNRESOLVED_NAME: DiagnosticCode = DiagnosticCode::new("hir", "unresolved-name");
pub const UNSUPPORTED_PATCH_REMOVE: DiagnosticCode = DiagnosticCode::new("hir", "unsupported-patch-remove");
