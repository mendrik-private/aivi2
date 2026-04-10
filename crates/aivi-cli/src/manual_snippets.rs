use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use aivi_hir::ValidationMode;
use aivi_query::{
    RootDatabase, SourceFile as QuerySourceFile, hir_module as query_hir_module,
    parsed_file as query_parsed_file,
};
use serde::Serialize;

#[derive(Clone, Debug)]
struct MarkdownDocument {
    path: PathBuf,
    text: String,
    blocks: Vec<FencedBlock>,
}

#[derive(Clone, Debug)]
struct FencedBlock {
    index: usize,
    fence_info: String,
    body_range: std::ops::Range<usize>,
    start_line: usize,
    end_line: usize,
}

#[derive(Clone, Debug)]
struct Replacement {
    range: std::ops::Range<usize>,
    text: String,
}

#[derive(Clone, Debug)]
struct BlockOutcome {
    formatted_text: String,
    formatting_changed: bool,
    todo_entry: Option<TodoEntry>,
}

#[derive(Clone, Debug, Serialize)]
struct TodoReport {
    root: String,
    scanned_files: usize,
    scanned_blocks: usize,
    rewritten_blocks: usize,
    unresolved_fragments: usize,
    entries: Vec<TodoEntry>,
}

#[derive(Clone, Debug, Serialize)]
struct TodoEntry {
    markdown_path: String,
    block_index: usize,
    fence_info: String,
    start_line: usize,
    end_line: usize,
    formatting_changed: bool,
    syntax_problem_count: usize,
    lsp_problem_count: usize,
    compiler_problem_count: usize,
    diagnostics: Vec<TodoDiagnostic>,
    suggested_snippet: String,
}

#[derive(Clone, Debug, Serialize)]
struct TodoDiagnostic {
    severity: String,
    code: Option<String>,
    message: String,
    rendered: String,
}

#[derive(Clone, Debug)]
struct AnalysisResult {
    syntax_problem_count: usize,
    lsp_problem_count: usize,
    compiler_problem_count: usize,
    diagnostics: Vec<TodoDiagnostic>,
}

pub(crate) fn run(mut args: impl Iterator<Item = OsString>) -> Result<ExitCode, String> {
    let mut root = PathBuf::from("manual");
    let mut todo_path = None;
    let mut write = false;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return super::print_help(Some(std::ffi::OsStr::new("manual-snippets")));
        }

        if argument == "--write" {
            write = true;
            continue;
        }

        if argument == "--root" {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path after `--root`".to_owned())?;
            root = path;
            continue;
        }

        if argument == "--todo" {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "expected a path after `--todo`".to_owned())?;
            todo_path = Some(path);
            continue;
        }

        return Err(format!(
            "unexpected manual-snippets argument `{}`",
            argument.to_string_lossy()
        ));
    }

    if !root.is_dir() {
        return Err(format!(
            "manual-snippets root is not a directory: {}",
            root.display()
        ));
    }

    let todo_path = todo_path.unwrap_or_else(|| root.join("aivi-snippet-todo.json"));
    let synthetic_workspace_root = bundled_workspace_root()?;
    let mut markdown_paths = Vec::new();
    collect_markdown_files(&root, &mut markdown_paths)?;
    markdown_paths.sort();

    let mut db = RootDatabase::new();
    let mut scanned_blocks = 0usize;
    let mut rewritten_blocks = 0usize;
    let mut entries = Vec::new();

    for path in markdown_paths {
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let document = MarkdownDocument {
            blocks: extract_aivi_blocks(&text)?,
            path,
            text,
        };

        if document.blocks.is_empty() {
            continue;
        }

        let relative_markdown_path = document
            .path
            .strip_prefix(&root)
            .unwrap_or(document.path.as_path());
        let mut replacements = Vec::new();

        for block in &document.blocks {
            scanned_blocks += 1;
            let original = &document.text[block.body_range.clone()];
            let synthetic_path = synthetic_snippet_path(
                &synthetic_workspace_root,
                relative_markdown_path,
                block.index,
            );
            let outcome = analyze_block(&mut db, original, block, &document.path, synthetic_path);

            if outcome.formatting_changed {
                rewritten_blocks += 1;
                if write {
                    replacements.push(Replacement {
                        range: block.body_range.clone(),
                        text: outcome.formatted_text.clone(),
                    });
                }
            }

            if let Some(entry) = outcome.todo_entry {
                entries.push(entry);
            }
        }

        if write && !replacements.is_empty() {
            let updated = apply_replacements(&document.text, &replacements);
            fs::write(&document.path, updated)
                .map_err(|error| format!("failed to write {}: {error}", document.path.display()))?;
        }
    }

    write_todo_report(
        &todo_path,
        TodoReport {
            root: root.display().to_string(),
            scanned_files: count_markdown_files(&root)?,
            scanned_blocks,
            rewritten_blocks,
            unresolved_fragments: entries.len(),
            entries,
        },
    )?;

    let mut stdout = std::io::stdout().lock();
    use std::io::Write as _;
    writeln!(
        stdout,
        "manual snippets: {} block{} scanned, {} rewritten, {} todo{} -> {}",
        scanned_blocks,
        plural_suffix(scanned_blocks),
        rewritten_blocks,
        count_unresolved_in_report(&todo_path)?,
        plural_suffix(count_unresolved_in_report(&todo_path)?),
        todo_path.display()
    )
    .map_err(|error| format!("failed to write summary: {error}"))?;

    if (write || (rewritten_blocks == 0)) && count_unresolved_in_report(&todo_path)? == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn analyze_block(
    db: &mut RootDatabase,
    original: &str,
    block: &FencedBlock,
    markdown_path: &Path,
    synthetic_path: PathBuf,
) -> BlockOutcome {
    let normalized_original = normalize_block_body(original);
    let file = QuerySourceFile::new(db, synthetic_path, normalized_original.clone());
    let parsed = query_parsed_file(db, file);
    let formatter = aivi_syntax::Formatter;
    let formatted = ensure_trailing_newline(&formatter.format(parsed.cst()));
    let formatting_changed = formatted != normalized_original;

    if formatting_changed {
        file.set_text(db, formatted.clone());
    }

    let analysis = analyze_current_file(db, file);

    let todo_entry = if analysis.syntax_problem_count > 0
        || analysis.lsp_problem_count > 0
        || analysis.compiler_problem_count > 0
    {
        Some(TodoEntry {
            markdown_path: markdown_path.display().to_string(),
            block_index: block.index,
            fence_info: block.fence_info.clone(),
            start_line: block.start_line,
            end_line: block.end_line,
            formatting_changed,
            syntax_problem_count: analysis.syntax_problem_count,
            lsp_problem_count: analysis.lsp_problem_count,
            compiler_problem_count: analysis.compiler_problem_count,
            diagnostics: analysis.diagnostics,
            suggested_snippet: formatted.clone(),
        })
    } else {
        None
    };

    BlockOutcome {
        formatted_text: formatted,
        formatting_changed,
        todo_entry,
    }
}

fn analyze_current_file(db: &RootDatabase, file: QuerySourceFile) -> AnalysisResult {
    let parsed = query_parsed_file(db, file);
    let hir = query_hir_module(db, file);
    let lsp = aivi_lsp::analysis::FileAnalysis::load(db, file);
    let validation_mode = if hir.hir_diagnostics().is_empty() {
        ValidationMode::RequireResolvedNames
    } else {
        ValidationMode::Structural
    };
    let validation = hir.module().validate(validation_mode);
    let source_db = db.source_database();
    let mut diagnostics = Vec::new();

    for diagnostic in parsed.diagnostics() {
        diagnostics.push(TodoDiagnostic {
            severity: diagnostic.severity.as_str().to_owned(),
            code: diagnostic.code.map(|code| code.to_string()),
            message: diagnostic.message.clone(),
            rendered: diagnostic.render(&source_db),
        });
    }

    for diagnostic in hir.hir_diagnostics() {
        diagnostics.push(TodoDiagnostic {
            severity: diagnostic.severity.as_str().to_owned(),
            code: diagnostic.code.map(|code| code.to_string()),
            message: diagnostic.message.clone(),
            rendered: diagnostic.render(&source_db),
        });
    }

    for diagnostic in validation.diagnostics() {
        diagnostics.push(TodoDiagnostic {
            severity: diagnostic.severity.as_str().to_owned(),
            code: diagnostic.code.map(|code| code.to_string()),
            message: diagnostic.message.clone(),
            rendered: diagnostic.render(&source_db),
        });
    }

    diagnostics.sort_by(|left, right| left.rendered.cmp(&right.rendered));
    diagnostics.dedup_by(|left, right| left.rendered == right.rendered);

    AnalysisResult {
        syntax_problem_count: parsed.diagnostics().len(),
        lsp_problem_count: lsp.diagnostics.len(),
        compiler_problem_count: hir.hir_diagnostics().len() + validation.diagnostics().len(),
        diagnostics,
    }
}

fn extract_aivi_blocks(text: &str) -> Result<Vec<FencedBlock>, String> {
    let mut blocks = Vec::new();
    let mut open_fence = None::<(usize, usize, String, usize)>;
    let mut offset = 0usize;
    let mut line_number = 1usize;

    for line in text.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        let trimmed = trim_line_ending(line);

        if let Some((body_start, start_line, fence_info, index)) = &open_fence {
            if trimmed.starts_with("```") {
                blocks.push(FencedBlock {
                    index: *index,
                    fence_info: fence_info.clone(),
                    body_range: *body_start..line_start,
                    start_line: *start_line,
                    end_line: line_number.saturating_sub(1),
                });
                open_fence = None;
            }
        } else if let Some(info) = trimmed.strip_prefix("```") {
            let info = info.trim();
            let language = info.split_ascii_whitespace().next().unwrap_or_default();
            if language == "aivi" {
                let next_index = blocks.len() + 1;
                open_fence = Some((line_end, line_number + 1, info.to_owned(), next_index));
            }
        }

        offset = line_end;
        line_number += 1;
    }

    if let Some((_, start_line, _, index)) = open_fence {
        return Err(format!(
            "unterminated ```aivi block {index} starting at line {start_line}"
        ));
    }

    Ok(blocks)
}

fn apply_replacements(source: &str, replacements: &[Replacement]) -> String {
    let mut updated = source.to_owned();
    let mut ordered = replacements.to_vec();
    ordered.sort_by(|left, right| right.range.start.cmp(&left.range.start));
    for replacement in ordered {
        updated.replace_range(replacement.range, &replacement.text);
    }
    updated
}

fn normalize_block_body(body: &str) -> String {
    let normalized = body.replace("\r\n", "\n");
    if normalized.is_empty() {
        String::new()
    } else {
        ensure_trailing_newline(&normalized)
    }
}

fn ensure_trailing_newline(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    }
}

fn trim_line_ending(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

fn collect_markdown_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("failed to read directory {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read directory entry under {}: {error}",
                root.display()
            )
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;

        if file_type.is_dir() {
            let name = entry.file_name();
            if name == "node_modules" || name.to_string_lossy().starts_with('.') {
                continue;
            }
            collect_markdown_files(&path, files)?;
            continue;
        }

        if file_type.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}

fn count_markdown_files(root: &Path) -> Result<usize, String> {
    let mut files = Vec::new();
    collect_markdown_files(root, &mut files)?;
    Ok(files.len())
}

fn bundled_workspace_root() -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("stdlib");
    if !root.join("aivi.toml").is_file() {
        return Err(format!(
            "failed to locate bundled stdlib workspace root at {}",
            root.display()
        ));
    }
    Ok(fs::canonicalize(&root).unwrap_or(root))
}

fn synthetic_snippet_path(
    workspace_root: &Path,
    relative_markdown_path: &Path,
    index: usize,
) -> PathBuf {
    let mut synthetic = workspace_root.join("manual_snippets");
    for component in relative_markdown_path.components() {
        synthetic.push(component);
    }
    synthetic.set_extension("");
    synthetic.push(format!("block_{index}.aivi"));
    synthetic
}

fn write_todo_report(path: &Path, report: TodoReport) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("failed to serialise {}: {error}", path.display()))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn count_unresolved_in_report(path: &Path) -> Result<usize, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let report: TodoReport = serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    Ok(report.unresolved_fragments)
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

impl<'de> serde::Deserialize<'de> for TodoReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct WireTodoReport {
            root: String,
            scanned_files: usize,
            scanned_blocks: usize,
            rewritten_blocks: usize,
            unresolved_fragments: usize,
            entries: Vec<TodoEntry>,
        }

        let wire = WireTodoReport::deserialize(deserializer)?;
        Ok(Self {
            root: wire.root,
            scanned_files: wire.scanned_files,
            scanned_blocks: wire.scanned_blocks,
            rewritten_blocks: wire.rewritten_blocks,
            unresolved_fragments: wire.unresolved_fragments,
            entries: wire.entries,
        })
    }
}

impl<'de> serde::Deserialize<'de> for TodoEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct WireTodoEntry {
            markdown_path: String,
            block_index: usize,
            fence_info: String,
            start_line: usize,
            end_line: usize,
            formatting_changed: bool,
            syntax_problem_count: usize,
            lsp_problem_count: usize,
            compiler_problem_count: usize,
            diagnostics: Vec<TodoDiagnostic>,
            suggested_snippet: String,
        }

        let wire = WireTodoEntry::deserialize(deserializer)?;
        Ok(Self {
            markdown_path: wire.markdown_path,
            block_index: wire.block_index,
            fence_info: wire.fence_info,
            start_line: wire.start_line,
            end_line: wire.end_line,
            formatting_changed: wire.formatting_changed,
            syntax_problem_count: wire.syntax_problem_count,
            lsp_problem_count: wire.lsp_problem_count,
            compiler_problem_count: wire.compiler_problem_count,
            diagnostics: wire.diagnostics,
            suggested_snippet: wire.suggested_snippet,
        })
    }
}

impl<'de> serde::Deserialize<'de> for TodoDiagnostic {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct WireTodoDiagnostic {
            severity: String,
            code: Option<String>,
            message: String,
            rendered: String,
        }

        let wire = WireTodoDiagnostic::deserialize(deserializer)?;
        Ok(Self {
            severity: wire.severity,
            code: wire.code,
            message: wire.message,
            rendered: wire.rendered,
        })
    }
}
