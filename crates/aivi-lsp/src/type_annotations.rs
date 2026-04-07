use aivi_base::SourceSpan;
use tower_lsp::lsp_types::{
    self as lsp, CodeAction, CodeActionKind, CodeActionOrCommand, DiagnosticSeverity,
    DiagnosticTag, NumberOrString, Range, TextEdit, Url, WorkspaceEdit,
};

pub const UNNECESSARY_TYPE_ANNOTATION_CODE: &str = "aivi/unnecessary-type-annotation";
pub const MISMATCHED_TYPE_ANNOTATION_CODE: &str = "aivi/mismatched-type-annotation";
pub const MISSING_TYPE_ANNOTATION_CODE: &str = "aivi/missing-type-annotation";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeAnnotationStyle {
    Inline,
    Standalone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeAnnotationDiagnosticKind {
    Unnecessary,
    Mismatched,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAnnotationSite {
    pub style: TypeAnnotationStyle,
    pub type_span: SourceSpan,
    pub full_span: SourceSpan,
    pub removal_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedDeclarationSummary {
    pub item_id: aivi_hir::ItemId,
    pub kind: aivi_hir::TypedDeclarationKind,
    pub name: String,
    pub header_span: SourceSpan,
    pub name_span: SourceSpan,
    pub declared_type: Option<String>,
    pub inferred_type: Option<String>,
    pub annotation_matches_inferred: Option<bool>,
    pub has_explicit_constraints: bool,
    pub annotation_is_independently_inferable: bool,
    pub annotation: Option<TypeAnnotationSite>,
}

pub fn collect_typed_declaration_summaries(
    module: &aivi_hir::Module,
    parsed: &aivi_syntax::Module,
    source: &aivi_base::SourceFile,
) -> Vec<TypedDeclarationSummary> {
    aivi_hir::collect_typed_declarations(module)
        .into_iter()
        .map(|info| {
            let parsed_item = matching_named_item(parsed, info.kind, info.header_span);
            TypedDeclarationSummary {
                item_id: info.item_id,
                kind: info.kind,
                name: info.name,
                header_span: info.header_span,
                name_span: info.name_span,
                declared_type: info.declared_type,
                inferred_type: info.inferred_type,
                annotation_matches_inferred: info.annotation_matches_inferred,
                has_explicit_constraints: info.has_explicit_constraints,
                annotation_is_independently_inferable: annotation_is_independently_inferable(
                    info.kind,
                    parsed_item,
                ),
                annotation: parsed_item.and_then(|item| annotation_site(item, source)),
            }
        })
        .collect()
}

pub fn diagnostic_kind(summary: &TypedDeclarationSummary) -> Option<TypeAnnotationDiagnosticKind> {
    match (
        summary.annotation.as_ref(),
        summary.inferred_type.as_ref(),
        summary.annotation_matches_inferred,
        summary.has_explicit_constraints,
        summary.annotation_is_independently_inferable,
    ) {
        (Some(_), Some(_), Some(true), false, true) => Some(TypeAnnotationDiagnosticKind::Unnecessary),
        (Some(_), Some(_), Some(false), _, _) => Some(TypeAnnotationDiagnosticKind::Mismatched),
        (None, None, _, _, _) => Some(TypeAnnotationDiagnosticKind::Missing),
        _ => None,
    }
}

pub fn collect_type_annotation_diagnostics(
    summaries: &[TypedDeclarationSummary],
    source: &aivi_base::SourceFile,
) -> Vec<lsp::Diagnostic> {
    summaries
        .iter()
        .filter_map(|summary| diagnostic_for_summary(summary, source))
        .collect()
}

pub fn diagnostic_for_summary(
    summary: &TypedDeclarationSummary,
    source: &aivi_base::SourceFile,
) -> Option<lsp::Diagnostic> {
    let kind = diagnostic_kind(summary)?;
    let range = diagnostic_range(summary, kind, source)?;
    let severity = match kind {
        TypeAnnotationDiagnosticKind::Unnecessary => DiagnosticSeverity::HINT,
        TypeAnnotationDiagnosticKind::Mismatched => DiagnosticSeverity::WARNING,
        TypeAnnotationDiagnosticKind::Missing => DiagnosticSeverity::ERROR,
    };

    Some(lsp::Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(diagnostic_code(kind).to_owned())),
        code_description: None,
        source: Some("aivi".to_owned()),
        message: diagnostic_message(summary, kind),
        related_information: None,
        tags: matches!(kind, TypeAnnotationDiagnosticKind::Unnecessary)
            .then_some(vec![DiagnosticTag::UNNECESSARY]),
        data: None,
    })
}

pub fn build_type_annotation_code_actions(
    uri: &Url,
    summaries: &[TypedDeclarationSummary],
    source: &aivi_base::SourceFile,
    request_range: Range,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    for summary in summaries {
        let Some(kind) = diagnostic_kind(summary) else {
            continue;
        };
        let Some(diag) = diagnostic_for_summary(summary, source) else {
            continue;
        };
        if !ranges_overlap(diag.range, request_range) {
            continue;
        }

        let Some(action) = code_action_for_summary(uri, source, summary, kind, diag) else {
            continue;
        };
        actions.push(CodeActionOrCommand::CodeAction(action));
    }

    actions
}

pub fn range_for_span(source: &aivi_base::SourceFile, span: SourceSpan) -> Range {
    crate::diagnostics::lsp_range(source.span_to_lsp_range(span.span()))
}

fn code_action_for_summary(
    uri: &Url,
    source: &aivi_base::SourceFile,
    summary: &TypedDeclarationSummary,
    kind: TypeAnnotationDiagnosticKind,
    diagnostic: lsp::Diagnostic,
) -> Option<CodeAction> {
    let edit = match kind {
        TypeAnnotationDiagnosticKind::Unnecessary => removal_edit(source, summary)?,
        TypeAnnotationDiagnosticKind::Mismatched => replacement_edit(source, summary)?,
        TypeAnnotationDiagnosticKind::Missing => return None,
    };

    let mut changes = std::collections::HashMap::new();
    changes.insert(uri.clone(), vec![edit]);

    let title = match kind {
        TypeAnnotationDiagnosticKind::Unnecessary => "Remove unnecessary type annotation".to_owned(),
        TypeAnnotationDiagnosticKind::Mismatched => {
            "Replace annotation with inferred type".to_owned()
        }
        TypeAnnotationDiagnosticKind::Missing => return None,
    };

    Some(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn removal_edit(
    source: &aivi_base::SourceFile,
    summary: &TypedDeclarationSummary,
) -> Option<TextEdit> {
    let annotation = summary.annotation.as_ref()?;
    Some(TextEdit {
        range: range_for_span(source, annotation.removal_span),
        new_text: String::new(),
    })
}

fn replacement_edit(
    source: &aivi_base::SourceFile,
    summary: &TypedDeclarationSummary,
) -> Option<TextEdit> {
    let annotation = summary.annotation.as_ref()?;
    let replacement = summary.inferred_type.clone()?;
    Some(TextEdit {
        range: range_for_span(source, annotation.type_span),
        new_text: replacement,
    })
}

fn diagnostic_range(
    summary: &TypedDeclarationSummary,
    kind: TypeAnnotationDiagnosticKind,
    source: &aivi_base::SourceFile,
) -> Option<Range> {
    Some(match kind {
        TypeAnnotationDiagnosticKind::Unnecessary => {
            range_for_span(source, summary.annotation.as_ref()?.full_span)
        }
        TypeAnnotationDiagnosticKind::Mismatched => {
            range_for_span(source, summary.annotation.as_ref()?.type_span)
        }
        TypeAnnotationDiagnosticKind::Missing => range_for_span(source, summary.name_span),
    })
}

fn diagnostic_code(kind: TypeAnnotationDiagnosticKind) -> &'static str {
    match kind {
        TypeAnnotationDiagnosticKind::Unnecessary => UNNECESSARY_TYPE_ANNOTATION_CODE,
        TypeAnnotationDiagnosticKind::Mismatched => MISMATCHED_TYPE_ANNOTATION_CODE,
        TypeAnnotationDiagnosticKind::Missing => MISSING_TYPE_ANNOTATION_CODE,
    }
}

fn diagnostic_message(
    summary: &TypedDeclarationSummary,
    kind: TypeAnnotationDiagnosticKind,
) -> String {
    match kind {
        TypeAnnotationDiagnosticKind::Unnecessary => format!(
            "type annotation for `{}` is unnecessary; the compiler infers `{}`",
            summary.name,
            summary.inferred_type.as_deref().unwrap_or("?")
        ),
        TypeAnnotationDiagnosticKind::Mismatched => format!(
            "declared type `{}` for `{}` does not match inferred type `{}`",
            summary.declared_type.as_deref().unwrap_or("?"),
            summary.name,
            summary.inferred_type.as_deref().unwrap_or("?")
        ),
        TypeAnnotationDiagnosticKind::Missing => format!(
            "cannot infer a type for `{}`; add an explicit annotation",
            summary.name
        ),
    }
}

fn matching_named_item<'a>(
    parsed: &'a aivi_syntax::Module,
    kind: aivi_hir::TypedDeclarationKind,
    header_span: SourceSpan,
) -> Option<&'a aivi_syntax::NamedItem> {
    parsed.items.iter().find_map(|item| match (kind, item) {
        (aivi_hir::TypedDeclarationKind::Value, aivi_syntax::Item::Value(named))
        | (aivi_hir::TypedDeclarationKind::Function, aivi_syntax::Item::Fun(named))
        | (aivi_hir::TypedDeclarationKind::Signal, aivi_syntax::Item::Signal(named))
            if named.base.span == header_span =>
        {
            Some(named)
        }
        _ => None,
    })
}

fn annotation_is_independently_inferable(
    kind: aivi_hir::TypedDeclarationKind,
    item: Option<&aivi_syntax::NamedItem>,
) -> bool {
    match kind {
        aivi_hir::TypedDeclarationKind::Value | aivi_hir::TypedDeclarationKind::Signal => true,
        aivi_hir::TypedDeclarationKind::Function => item
            .is_some_and(|item| item.parameters.iter().all(|parameter| parameter.annotation.is_some())),
    }
}

fn annotation_site(
    item: &aivi_syntax::NamedItem,
    source: &aivi_base::SourceFile,
) -> Option<TypeAnnotationSite> {
    let annotation = item.annotation.as_ref()?;
    let coverage = annotation_coverage_span(item, annotation.span)?;
    let standalone = coverage.span().start() < item.keyword_span.span().start();

    if standalone {
        let full_span = full_line_span(source, coverage);
        return Some(TypeAnnotationSite {
            style: TypeAnnotationStyle::Standalone,
            type_span: annotation.span,
            full_span,
            removal_span: full_span,
        });
    }

    let colon_start = inline_annotation_colon_start(source.text(), coverage.span().start().as_usize())
        .unwrap_or_else(|| coverage.span().start().as_usize());
    let removal_start = inline_annotation_removal_start(source.text(), colon_start);
    Some(TypeAnnotationSite {
        style: TypeAnnotationStyle::Inline,
        type_span: annotation.span,
        full_span: source.source_span(colon_start..coverage.span().end().as_usize()),
        removal_span: source.source_span(removal_start..coverage.span().end().as_usize()),
    })
}

fn annotation_coverage_span(
    item: &aivi_syntax::NamedItem,
    annotation_span: SourceSpan,
) -> Option<SourceSpan> {
    item.constraints
        .iter()
        .fold(Some(annotation_span), |current, constraint| {
            current?.join(constraint.span)
        })
}

fn full_line_span(source: &aivi_base::SourceFile, span: SourceSpan) -> SourceSpan {
    let text = source.text();
    let start = line_start_offset(text, span.span().start().as_usize());
    let end = line_end_offset(text, span.span().end().as_usize());
    source.source_span(start..end)
}

fn line_start_offset(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

fn line_end_offset(text: &str, offset: usize) -> usize {
    let bytes = text.as_bytes();
    let mut cursor = offset;
    while cursor < bytes.len() && bytes[cursor] != b'\n' && bytes[cursor] != b'\r' {
        cursor += 1;
    }
    if cursor < bytes.len() && bytes[cursor] == b'\r' {
        cursor += 1;
        if cursor < bytes.len() && bytes[cursor] == b'\n' {
            cursor += 1;
        }
    } else if cursor < bytes.len() && bytes[cursor] == b'\n' {
        cursor += 1;
    }
    cursor
}

fn inline_annotation_colon_start(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut cursor = from;
    while cursor > 0 {
        cursor -= 1;
        match bytes[cursor] {
            b':' => return Some(cursor),
            b'\n' | b'\r' => return None,
            _ => {}
        }
    }
    None
}

fn inline_annotation_removal_start(text: &str, colon_start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut cursor = colon_start;
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t') {
        cursor -= 1;
    }
    cursor
}

fn ranges_overlap(a: Range, b: Range) -> bool {
    a.start <= b.end && b.start <= a.end
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tower_lsp::lsp_types::{
        CodeActionOrCommand, DiagnosticSeverity, NumberOrString, Range, Url,
    };

    use super::{
        MISMATCHED_TYPE_ANNOTATION_CODE, MISSING_TYPE_ANNOTATION_CODE,
        TypeAnnotationDiagnosticKind, TypeAnnotationStyle, UNNECESSARY_TYPE_ANNOTATION_CODE,
        build_type_annotation_code_actions, collect_type_annotation_diagnostics,
        collect_typed_declaration_summaries, diagnostic_kind,
    };

    fn parse(
        text: &str,
    ) -> (
        aivi_base::SourceFile,
        aivi_syntax::Module,
        aivi_hir::Module,
        Vec<super::TypedDeclarationSummary>,
    ) {
        let source = aivi_base::SourceFile::new(aivi_base::FileId::new(0), "test.aivi", text);
        let parsed = aivi_syntax::parse_module(&source);
        assert!(
            !parsed.has_errors(),
            "annotation test input should parse cleanly: {:?}",
            parsed.all_diagnostics().collect::<Vec<_>>()
        );
        let lowered = aivi_hir::lower_module(&parsed.module);
        assert!(
            !lowered.has_errors(),
            "annotation test input should lower cleanly: {:?}",
            lowered.diagnostics()
        );
        let summaries = collect_typed_declaration_summaries(lowered.module(), &parsed.module, &source);
        (source, parsed.module, lowered.into_parts().0, summaries)
    }

    fn uri() -> Url {
        Url::from_file_path(PathBuf::from("/test-documents/type-annotations.aivi"))
            .expect("test URI should be valid")
    }

    #[test]
    fn matching_value_annotations_become_unnecessary_hints() {
        let (source, _, _, summaries) = parse("value answer : Int = 42\n");
        let diagnostics = collect_type_annotation_diagnostics(&summaries, &source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code
                    == Some(NumberOrString::String(
                        UNNECESSARY_TYPE_ANNOTATION_CODE.to_owned(),
                    ))
            })
            .expect("expected unnecessary type annotation diagnostic");

        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::HINT));
        assert_eq!(
            diagnostic_kind(&summaries[0]),
            Some(TypeAnnotationDiagnosticKind::Unnecessary)
        );
    }

    #[test]
    fn mismatched_value_annotations_become_warnings() {
        let (source, _, _, summaries) = parse("value answer : Text = 42\n");
        let diagnostics = collect_type_annotation_diagnostics(&summaries, &source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code
                    == Some(NumberOrString::String(
                        MISMATCHED_TYPE_ANNOTATION_CODE.to_owned(),
                    ))
            })
            .expect("expected mismatched type annotation diagnostic");

        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diagnostic.range.start.character, 15);
    }

    #[test]
    fn uninferable_unannotated_functions_become_errors() {
        let (source, _, _, summaries) = parse("func id = x => x\n");
        let diagnostics = collect_type_annotation_diagnostics(&summaries, &source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code
                    == Some(NumberOrString::String(
                        MISSING_TYPE_ANNOTATION_CODE.to_owned(),
                    ))
            })
            .expect("expected missing type annotation diagnostic");

        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn standalone_function_annotations_are_detected_as_standalone() {
        let (_, _, _, summaries) = parse(
            "type Int -> Int\n\
             func id = value => value\n",
        );
        let annotation = summaries[0]
            .annotation
            .as_ref()
            .expect("expected annotation site");
        assert_eq!(annotation.style, TypeAnnotationStyle::Standalone);
    }

    #[test]
    fn parameterized_function_signatures_do_not_become_unnecessary_hints() {
        let (_, _, _, summaries) = parse(
            "type Int -> Int\n\
             func id = value => value\n",
        );

        assert_eq!(diagnostic_kind(&summaries[0]), None);
        assert!(!summaries[0].annotation_is_independently_inferable);
    }

    #[test]
    fn unnecessary_annotation_quick_fix_removes_only_the_annotation_text() {
        let (source, _, _, summaries) = parse("value answer : Int = 42\n");
        let actions = build_type_annotation_code_actions(
            &uri(),
            &summaries,
            &source,
            Range {
                start: tower_lsp::lsp_types::Position {
                    line: 0,
                    character: 0,
                },
                end: tower_lsp::lsp_types::Position {
                    line: 0,
                    character: 64,
                },
            },
        );
        let action = actions
            .into_iter()
            .find_map(|action| match action {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Remove unnecessary type annotation" =>
                {
                    Some(action)
                }
                _ => None,
            })
            .expect("expected unnecessary type annotation quick fix");
        let edit = action
            .edit
            .and_then(|edit| edit.changes)
            .and_then(|mut changes| changes.remove(&uri()))
            .and_then(|mut edits| edits.pop())
            .expect("expected removal edit");

        assert_eq!(edit.new_text, "");
        assert_eq!(edit.range.start.character, 12);
        assert_eq!(edit.range.end.character, 18);
    }
}
