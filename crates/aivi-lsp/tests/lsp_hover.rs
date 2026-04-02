use std::{path::PathBuf, sync::Arc};

use aivi_lsp::{documents::open_document, hover::hover, state::ServerState};
use tower_lsp::lsp_types::{
    HoverContents, HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams, Url,
};

fn test_uri(name: &str) -> Url {
    Url::from_file_path(PathBuf::from("/test-documents").join(name))
        .expect("test URI should be valid")
}

fn open_inline(name: &str, text: &str) -> (Arc<ServerState>, Url) {
    let state = Arc::new(ServerState::new());
    let uri = test_uri(name);
    open_document(&state, &uri, text.to_owned());
    (state, uri)
}

fn hover_params(uri: Url, line: u32, character: u32) -> HoverParams {
    HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position { line, character },
        },
        work_done_progress_params: Default::default(),
    }
}

#[tokio::test]
async fn hover_at_value_name_returns_kind_label() {
    // "value answer = 42" — 'answer' starts at character 6 on line 0
    let (state, uri) = open_inline("hover-value.aivi", "value answer = 42\n");
    let result = hover(hover_params(uri, 0, 6), state).await;

    assert!(result.is_some(), "hover at a value name should return a result");

    if let Some(hover_info) = result {
        if let HoverContents::Markup(markup) = hover_info.contents {
            assert!(
                markup.value.contains("value") || markup.value.contains("answer"),
                "hover content should mention the kind or name; got: {}",
                markup.value
            );
        }
    }
}

#[tokio::test]
async fn hover_at_out_of_range_position_returns_none() {
    let (state, uri) = open_inline("hover-empty.aivi", "value answer = 42\n");
    // Line 99 is far beyond the file content
    let result = hover(hover_params(uri, 99, 0), state).await;
    assert!(
        result.is_none(),
        "hover at an out-of-range position should return None"
    );
}
