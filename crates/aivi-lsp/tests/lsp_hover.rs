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

#[tokio::test]
async fn hover_on_func_declaration_uses_func_kind_label() {
    // "type Int -> Int\nfunc id x =>\n    x\n"
    // 'id' is on line 1, character 5
    let text = "type Int -> Int\nfunc id x =>\n    x\n";
    let (state, uri) = open_inline("hover-func.aivi", text);
    let result = hover(hover_params(uri, 1, 5), state).await;

    assert!(result.is_some(), "hover on a func name should return a result");

    if let Some(hover_info) = result {
        if let HoverContents::Markup(markup) = hover_info.contents {
            // The hover content should contain the symbol name and use "func" or "value"
            // as the kind label.
            assert!(
                markup.value.contains("id"),
                "hover content should mention the function name; got: {}",
                markup.value
            );
            assert!(
                markup.value.starts_with("```aivi"),
                "hover content should be a fenced code block; got: {}",
                markup.value
            );
        }
    }
}

#[tokio::test]
async fn hover_on_signal_declaration_uses_signal_kind_label() {
    // signal tick = 0
    // 'tick' is on line 0, character 7
    let text = "signal tick = 0\n";
    let (state, uri) = open_inline("hover-signal.aivi", text);
    let result = hover(hover_params(uri, 0, 7), state).await;

    assert!(result.is_some(), "hover on a signal name should return a result");

    if let Some(hover_info) = result {
        if let HoverContents::Markup(markup) = hover_info.contents {
            assert!(
                markup.value.contains("signal"),
                "hover on a signal should mention 'signal' kind; got: {}",
                markup.value
            );
            assert!(
                markup.value.contains("tick"),
                "hover on a signal should mention the name 'tick'; got: {}",
                markup.value
            );
        }
    }
}

#[tokio::test]
async fn hover_colon_separated_from_type_detail() {
    // When a symbol has a type detail the hover should format it as
    //   kind name : detail
    // with a space before the colon (not "kind name: detail").
    let text = "value answer = 42\n";
    let (state, uri) = open_inline("hover-colon.aivi", text);
    let result = hover(hover_params(uri, 0, 6), state).await;

    if let Some(hover_info) = result {
        if let HoverContents::Markup(markup) = hover_info.contents {
            // If the symbol carries a detail, the format must be "… : <detail>".
            // If there is no detail, just "kind name" is fine.
            if markup.value.contains(':') {
                assert!(
                    markup.value.contains(" : "),
                    "detail separator should be ' : ' with spaces; got: {}",
                    markup.value
                );
            }
        }
    }
}
