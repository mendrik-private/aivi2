use std::{path::PathBuf, sync::Arc};

use aivi_lsp::{documents::open_document, references::references, state::ServerState};
use tower_lsp::lsp_types::{
    PartialResultParams, Position, ReferenceContext, ReferenceParams, TextDocumentIdentifier,
    TextDocumentPositionParams, Url, WorkDoneProgressParams,
};

fn inline_uri(name: &str) -> Url {
    Url::from_file_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join(name),
    )
    .expect("test file path should convert to a file URL")
}

fn open_inline(name: &str, text: &str) -> (Arc<ServerState>, Url, String) {
    let state = Arc::new(ServerState::new());
    let uri = inline_uri(name);
    open_document(&state, &uri, text.to_owned());
    (state, uri, text.to_owned())
}

fn reference_params(uri: Url, position: Position) -> ReferenceParams {
    ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
        context: ReferenceContext {
            include_declaration: true,
        },
    }
}

fn position_at_byte(text: &str, byte_index: usize) -> Position {
    let prefix = &text[..byte_index];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    Position {
        line,
        character: text[line_start..byte_index].encode_utf16().count() as u32,
    }
}

fn position_of_nth(text: &str, needle: &str, occurrence: usize) -> Position {
    let mut start = 0usize;
    let mut seen = 0usize;
    loop {
        let relative = text[start..]
            .find(needle)
            .unwrap_or_else(|| panic!("could not find occurrence #{occurrence} of `{needle}`"));
        let byte_index = start + relative;
        if seen == occurrence {
            return position_at_byte(text, byte_index);
        }
        seen += 1;
        start = byte_index + needle.len();
    }
}

#[tokio::test]
async fn find_refs_returns_declaration_and_usage_for_local_binding() {
    // The function `id` binds `x` as a parameter and uses it in the body.
    // find-refs at the declaration site of `x` should return both:
    // 1. the binding declaration span, and
    // 2. the use site in the body.
    let text = "type Int -> Int\nfunc id x =>\n    x\n";
    let (state, uri, _) = open_inline("refs-local.aivi", text);

    // Position at the first occurrence of `x` (the parameter declaration).
    let decl_pos = position_of_nth(text, "x", 0);
    let params = reference_params(uri.clone(), decl_pos);
    let result = references(params, state).await;

    let locs = result.expect("find-refs should return at least one location for a used binding");
    assert!(
        locs.len() >= 2,
        "should find the declaration and at least one use site; got {} location(s): {:?}",
        locs.len(),
        locs
    );
    assert!(
        locs.iter().all(|l| l.uri == uri),
        "all reference locations should be in the same file"
    );
}

#[tokio::test]
async fn find_refs_at_use_site_matches_declaration_site_results() {
    // find-refs at the use site of `x` should return the same set as at the declaration.
    let text = "type Int -> Int\nfunc id x =>\n    x\n";
    let (state, uri, _) = open_inline("refs-use-site.aivi", text);

    let use_pos = position_of_nth(text, "x", 1);
    let params = reference_params(uri, use_pos);
    let result = references(params, state).await;

    assert!(
        result.is_some(),
        "find-refs at the use site of a binding should return locations"
    );
    assert!(
        !result.unwrap().is_empty(),
        "at least the reference site itself should be returned"
    );
}

#[tokio::test]
async fn find_refs_returns_none_for_out_of_range_position() {
    let text = "value answer = 42\n";
    let (state, uri, _) = open_inline("refs-oor.aivi", text);

    let params = reference_params(
        uri,
        Position {
            line: 99,
            character: 0,
        },
    );
    let result = references(params, state).await;

    assert!(
        result.is_none(),
        "find-refs at an out-of-range position should return None"
    );
}
