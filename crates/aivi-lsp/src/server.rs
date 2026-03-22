use std::sync::Arc;

use tower_lsp::{
    Client, LanguageServer,
    jsonrpc::Result,
    lsp_types::{
        CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
        DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
        DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
        Hover, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
        InitializedParams, MessageType, OneOf, SemanticTokensFullOptions, SemanticTokensOptions,
        SemanticTokensParams, SemanticTokensResult, SemanticTokensServerCapabilities,
        ServerCapabilities, SymbolInformation, TextDocumentSyncCapability, TextDocumentSyncKind,
        TextDocumentSyncOptions, TextEdit, WorkDoneProgressOptions, WorkspaceSymbolParams,
    },
};

use crate::state::ServerState;

pub struct Backend {
    pub client: Client,
    pub state: Arc<ServerState>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(ServerState::new()),
        }
    }

    async fn publish_diagnostics_for_uri(&self, uri: tower_lsp::lsp_types::Url) {
        let maybe_file = self.state.files.get(&uri).map(|f| *f);
        let Some(file) = maybe_file else { return };

        let lsp_diags = {
            let mut db = self.state.db.write();
            crate::diagnostics::collect_lsp_diagnostics(&mut db, file, &uri)
        };

        self.client.publish_diagnostics(uri, lsp_diags, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..Default::default()
                    },
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_owned()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                            legend: tower_lsp::lsp_types::SemanticTokensLegend {
                                token_types: Vec::new(),
                                token_modifiers: Vec::new(),
                            },
                            range: None,
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "aivi language server initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        crate::documents::open_document(&self.state, &uri, text);
        self.publish_diagnostics_for_uri(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
            crate::documents::change_document(&self.state, &uri, change.text);
        }
        self.publish_diagnostics_for_uri(uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        crate::documents::close_document(&self.state, &uri);
        // Clear diagnostics.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let maybe_file = self.state.files.get(uri).map(|f| *f);
        let Some(file) = maybe_file else {
            return Ok(None);
        };

        let (symbols, text, path) = {
            let mut db = self.state.db.write();
            let symbols = aivi_query::symbol_index(&mut db, file);
            let text = file.text(&db).to_owned();
            let path = file.path(&db).to_path_buf();
            (symbols, text, path)
        };

        let mut source_db = aivi_base::SourceDatabase::new();
        let file_id = source_db.add_file(path, text);
        let source_file = &source_db[file_id];

        let doc_symbols = crate::symbols::convert_symbols(symbols, source_file);
        Ok(Some(DocumentSymbolResponse::Nested(doc_symbols)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let maybe_file = self.state.files.get(uri).map(|f| *f);
        let Some(file) = maybe_file else {
            return Ok(None);
        };

        let (text, path) = {
            let db = self.state.db.read();
            let text = file.text(&db).to_owned();
            let path = file.path(&db).to_path_buf();
            (text, path)
        };

        Ok(crate::formatting::format_document(&text, &path))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        Ok(crate::hover::hover(params, Arc::clone(&self.state)).await)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        Ok(crate::completion::completion(params).await)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        Ok(crate::definition::definition(params, Arc::clone(&self.state)).await)
    }

    async fn symbol(
        &self,
        _params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        Ok(Some(Vec::new()))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        Ok(crate::semantic_tokens::semantic_tokens_full(params).await)
    }
}
