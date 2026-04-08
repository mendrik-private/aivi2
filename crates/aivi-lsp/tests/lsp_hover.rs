use std::{path::PathBuf, sync::Arc};

use aivi_lsp::{documents::open_document, hover::hover, state::ServerState};
use tower_lsp::lsp_types::{
    Hover, HoverContents, HoverParams, Position, TextDocumentIdentifier,
    TextDocumentPositionParams, Url,
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

fn position_at_byte(text: &str, byte_index: usize) -> Position {
    let prefix = &text[..byte_index];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
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

fn hover_markup(result: Option<Hover>) -> String {
    let hover = result.expect("expected hover result");
    match hover.contents {
        HoverContents::Markup(markup) => markup.value,
        other => panic!("expected markdown hover contents, found {other:?}"),
    }
}

#[tokio::test]
async fn hover_at_value_name_returns_kind_label() {
    // "value answer = 42" — 'answer' starts at character 6 on line 0
    let (state, uri) = open_inline("hover-value.aivi", "value answer = 42\n");
    let markup = hover_markup(hover(hover_params(uri, 0, 6), state).await);

    assert!(
        markup.contains("value answer : Int"),
        "hover should show the inferred value type; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_on_markup_value_name_returns_widget_type() {
    let text = "value main =\n    <Window title=\"AIVI Snake\">\n    </Window>\n";
    let (state, uri) = open_inline("hover-markup-value.aivi", text);
    let markup = hover_markup(hover(hover_params(uri, 0, 6), state).await);

    assert!(
        markup.contains("value main : Widget"),
        "hover should show the surface widget type for markup values; got: {}",
        markup
    );
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
    // "type Int -> Int\nfunc id = x =>\n    x\n"
    // 'id' is on line 1, character 5
    let text = "type Int -> Int\nfunc id = x =>\n    x\n";
    let (state, uri) = open_inline("hover-func.aivi", text);
    let markup = hover_markup(hover(hover_params(uri, 1, 5), state).await);

    assert!(
        markup.contains("func id : Int -> Int"),
        "hover should show the inferred function signature; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_on_signal_declaration_uses_signal_kind_label() {
    // signal tick = 0
    // 'tick' is on line 0, character 7
    let text = "signal tick = 0\n";
    let (state, uri) = open_inline("hover-signal.aivi", text);
    let markup = hover_markup(hover(hover_params(uri, 0, 7), state).await);

    assert!(
        markup.contains("signal tick : Signal Int"),
        "hover should show the inferred signal type; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_colon_separated_from_type_detail() {
    // When a symbol has a type detail the hover should format it as
    //   kind name : detail
    // with a space before the colon (not "kind name: detail").
    let text = "value answer = 42\n";
    let (state, uri) = open_inline("hover-colon.aivi", text);
    let markup = hover_markup(hover(hover_params(uri, 0, 6), state).await);
    assert!(
        markup.contains(" : "),
        "detail separator should be ' : ' with spaces; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_on_reference_site_uses_inferred_declaration_type() {
    let text = "value answer = 42\nvalue total = answer\n";
    let (state, uri) = open_inline("hover-reference.aivi", text);
    let markup = hover_markup(hover(hover_params(uri, 1, 14), state).await);

    assert!(
        markup.contains("value answer : Int"),
        "hover on a reference should resolve to the declaration's inferred type; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_on_mismatched_annotation_mentions_declared_type() {
    let text = "value answer : Text = 42\n";
    let (state, uri) = open_inline("hover-mismatch.aivi", text);
    let markup = hover_markup(hover(hover_params(uri, 0, 6), state).await);

    assert!(
        markup.contains("value answer : Int"),
        "hover should lead with the inferred type when annotations disagree; got: {}",
        markup
    );
    assert!(
        markup.contains("Declared type: `Text`"),
        "hover should also mention the declared type when it mismatches; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_on_from_source_survives_multibyte_text_in_reversi() {
    let text = include_str!("../../../demos/reversi.aivi");
    let (state, uri) = open_inline("hover-reversi-from.aivi", text);
    let position = position_of_nth(text, "from state", 0);
    let markup = hover_markup(
        hover(
            hover_params(uri, position.line, position.character + 5),
            state,
        )
        .await,
    );

    assert!(
        markup.contains("signal state : Signal {"),
        "hover on the `from state` source should resolve without panicking; got: {}",
        markup
    );
}

#[tokio::test]
async fn hover_survives_box_drawing_comments_before_binary_exprs() {
    let text = "/**\n * Mailfox UI entry point.\n **/\n\n// ─── Imports ──────────────────────────────────────\nvalue total = 42\nvalue check = 1 + 2\n";
    let (state, uri) = open_inline("hover-box-drawing-comment.aivi", text);
    let position = position_of_nth(text, "total", 0);
    let markup = hover_markup(hover(hover_params(uri, position.line, position.character), state).await);

    assert!(
        markup.contains("value total : Int"),
        "hover should resolve without panicking when comments contain box-drawing Unicode; got: {}",
        markup
    );
}
