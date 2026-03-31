use std::{fs, path::PathBuf, sync::Arc};

use aivi_lsp::{
    definition::definition, documents::open_document, implementation::implementation,
    state::ServerState,
};
use tower_lsp::lsp_types::request::{GotoImplementationParams, GotoImplementationResponse};
use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, Position, TextDocumentIdentifier,
    TextDocumentPositionParams, Url,
};

fn inline_uri(name: &str) -> Url {
    Url::from_file_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join(name),
    )
    .expect("test file path should convert to a file URL")
}

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn fixture_uri(relative: &str) -> Url {
    Url::from_file_path(fixture_path(relative)).expect("fixture path should convert to a file URL")
}

fn fixture_text(relative: &str) -> String {
    fs::read_to_string(fixture_path(relative)).expect("fixture text should be readable")
}

fn definition_params(uri: Url, position: Position) -> GotoDefinitionParams {
    GotoDefinitionParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    }
}

fn implementation_params(uri: Url, position: Position) -> GotoImplementationParams {
    GotoImplementationParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    }
}

fn response_locations(response: GotoDefinitionResponse) -> Vec<Location> {
    match response {
        GotoDefinitionResponse::Scalar(location) => vec![location],
        GotoDefinitionResponse::Array(locations) => locations,
        GotoDefinitionResponse::Link(_) => {
            panic!("navigation should return location results instead of links")
        }
    }
}

fn implementation_locations(response: GotoImplementationResponse) -> Vec<Location> {
    response_locations(response)
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

fn position_at_byte(text: &str, byte_index: usize) -> Position {
    let prefix = &text[..byte_index];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    Position {
        line,
        character: text[line_start..byte_index].encode_utf16().count() as u32,
    }
}

fn open_inline_document(name: &str, text: &str) -> (Arc<ServerState>, Url, String) {
    let state = Arc::new(ServerState::new());
    let uri = inline_uri(name);
    open_document(&state, &uri, text.to_owned());
    (state, uri, text.to_owned())
}

fn open_fixture_document(relative: &str) -> (Arc<ServerState>, Url, String) {
    let state = Arc::new(ServerState::new());
    let uri = fixture_uri(relative);
    let text = fixture_text(relative);
    open_document(&state, &uri, text.clone());
    (state, uri, text)
}

#[tokio::test]
async fn definition_resolves_local_binding_use_site() {
    let text = "type Int -> Int\nfunc id x =>\n    x\n";
    let (state, uri, text) = open_inline_document("local-binding-nav.aivi", text);

    let response = definition(
        definition_params(uri.clone(), position_of_nth(&text, "x", 1)),
        state,
    )
    .await
    .expect("definition should resolve for a local binding use site");

    let locations = response_locations(response);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, uri);
    assert_eq!(locations[0].range.start, position_of_nth(&text, "x", 0));
}

#[tokio::test]
async fn definition_resolves_sum_constructor_use_site() {
    let text = "type Status =\n  | Idle\n  | Busy\n\nvalue current = Idle\n";
    let (state, uri, text) = open_inline_document("constructor-nav.aivi", text);

    let response = definition(
        definition_params(uri.clone(), position_of_nth(&text, "Idle", 1)),
        state,
    )
    .await
    .expect("definition should resolve for a constructor use site");

    let locations = response_locations(response);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, uri);
    assert_eq!(locations[0].range.start, position_of_nth(&text, "Idle", 0));
}

#[tokio::test]
async fn definition_resolves_cross_file_imported_term() {
    let main_relative = "fixtures/frontend/milestone-2/valid/workspace-typeclass-prelude/main.aivi";
    let target_relative =
        "fixtures/frontend/milestone-2/valid/workspace-typeclass-prelude/shared/logic.aivi";
    let (state, uri, text) = open_fixture_document(main_relative);

    let response = definition(
        definition_params(uri, position_of_nth(&text, "liftOne", 1)),
        state,
    )
    .await
    .expect("definition should resolve for an imported term use site");

    let locations = response_locations(response);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, fixture_uri(target_relative));
    assert_eq!(
        locations[0].range.start,
        position_of_nth(&fixture_text(target_relative), "liftOne", 0)
    );
}

#[tokio::test]
async fn definition_resolves_cross_file_imported_type() {
    let main_relative = "fixtures/frontend/milestone-2/valid/workspace-type-imports/main.aivi";
    let target_relative =
        "fixtures/frontend/milestone-2/valid/workspace-type-imports/shared/types.aivi";
    let (state, uri, text) = open_fixture_document(main_relative);

    let response = definition(
        definition_params(uri, position_of_nth(&text, "Greeting", 1)),
        state,
    )
    .await
    .expect("definition should resolve for an imported type use site");

    let locations = response_locations(response);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, fixture_uri(target_relative));
    assert_eq!(
        locations[0].range.start,
        position_of_nth(&fixture_text(target_relative), "Greeting", 0)
    );
}

#[tokio::test]
async fn definition_resolves_class_member_use_site() {
    let relative = "fixtures/frontend/milestone-2/valid/instance-declarations/main.aivi";
    let (state, uri, text) = open_fixture_document(relative);

    let response = definition(
        definition_params(uri.clone(), position_of_nth(&text, "==", 2)),
        state,
    )
    .await
    .expect("definition should resolve for a class member use site");

    let locations = response_locations(response);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, uri);
    assert_eq!(locations[0].range.start, position_of_nth(&text, "(==)", 0));
}

#[tokio::test]
async fn implementation_resolves_class_member_use_site() {
    let relative = "fixtures/frontend/milestone-2/valid/instance-declarations/main.aivi";
    let (state, uri, text) = open_fixture_document(relative);

    let response = implementation(
        implementation_params(uri.clone(), position_of_nth(&text, "==", 2)),
        state,
    )
    .await
    .expect("implementation should resolve for a class member use site");

    let locations = implementation_locations(response);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, uri);
    assert_eq!(locations[0].range.start, position_of_nth(&text, "(==)", 1));
}
