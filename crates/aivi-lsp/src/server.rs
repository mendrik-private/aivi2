use std::sync::Arc;

use tower_lsp::{
    Client, LanguageServer,
    jsonrpc::Result,
    lsp_types::{
        CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
        DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
        DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
        Hover, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
        InitializedParams, Location, MessageType, OneOf, SemanticTokensFullOptions,
        SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
        SemanticTokensServerCapabilities, ServerCapabilities, SymbolInformation, SymbolKind,
        TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, TextEdit,
        WorkDoneProgressOptions, WorkspaceSymbolParams,
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
        let maybe_file = self.state.files.get(&uri).map(|file| *file);
        let Some(file) = maybe_file else {
            tracing::error!(
                "publish_diagnostics_for_uri: URI {} is not tracked; diagnostics will not be published",
                uri
            );
            return;
        };

        let lsp_diags = crate::diagnostics::collect_lsp_diagnostics(&self.state.db, file, &uri);
        self.client
            .publish_diagnostics(uri.clone(), lsp_diags, None)
            .await;
        tracing::debug!("Published diagnostics for {}", uri);
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
                            legend: SemanticTokensLegend {
                                token_types: crate::semantic_tokens::TOKEN_TYPES.to_vec(),
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
        // Cancel any in-flight diagnostics task for this URI.
        if let Some((_, handle)) = self.state.pending_diagnostics.remove(&uri) {
            handle.abort();
        }
        // Spawn a debounced diagnostics task: if no further edits arrive within
        // 100 ms the sleep completes and diagnostics are published.
        let state_clone = Arc::clone(&self.state);
        let client_clone = self.client.clone();
        let uri_clone = uri.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let maybe_file = state_clone.files.get(&uri_clone).map(|f| *f);
            let Some(file) = maybe_file else {
                tracing::error!(
                    "did_change debounce: URI {} is not tracked; diagnostics will not be published",
                    uri_clone
                );
                return;
            };
            let lsp_diags =
                crate::diagnostics::collect_lsp_diagnostics(&state_clone.db, file, &uri_clone);
            client_clone
                .publish_diagnostics(uri_clone.clone(), lsp_diags, None)
                .await;
            tracing::debug!("Published diagnostics for {}", uri_clone);
        });
        self.state.pending_diagnostics.insert(uri, handle);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        // Cancel any pending debounced task before removing the document.
        if let Some((_, handle)) = self.state.pending_diagnostics.remove(&uri) {
            handle.abort();
        }
        crate::documents::close_document(&self.state, &uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let maybe_file = self.state.files.get(uri).map(|file| *file);
        let Some(file) = maybe_file else {
            return Ok(None);
        };

        let analysis = crate::analysis::FileAnalysis::load(&self.state.db, file);
        let doc_symbols =
            crate::symbols::convert_symbols(analysis.symbols.as_ref(), analysis.source.as_ref());
        Ok(Some(DocumentSymbolResponse::Nested(doc_symbols)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let maybe_file = self.state.files.get(uri).map(|file| *file);
        let Some(file) = maybe_file else {
            return Ok(None);
        };

        Ok(crate::formatting::format_document(&self.state.db, file))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        Ok(crate::hover::hover(params, Arc::clone(&self.state)).await)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        Ok(crate::completion::completion(params, Arc::clone(&self.state)).await)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        Ok(crate::definition::definition(params, Arc::clone(&self.state)).await)
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_ascii_lowercase();
        let mut results: Vec<SymbolInformation> = Vec::new();

        for entry in self.state.files.iter() {
            let (uri, file) = (entry.key().clone(), *entry.value());
            let analysis = crate::analysis::FileAnalysis::load(&self.state.db, file);
            let source = analysis.source.as_ref();

            let mut stack: Vec<&aivi_hir::LspSymbol> = analysis.symbols.iter().collect();
            while let Some(sym) = stack.pop() {
                if query.is_empty() || sym.name.to_ascii_lowercase().contains(&query) {
                    let range = source.span_to_lsp_range(sym.span.span());
                    let lsp_range = tower_lsp::lsp_types::Range {
                        start: tower_lsp::lsp_types::Position {
                            line: range.start.line,
                            character: range.start.character,
                        },
                        end: tower_lsp::lsp_types::Position {
                            line: range.end.line,
                            character: range.end.character,
                        },
                    };
                    #[allow(deprecated)]
                    results.push(SymbolInformation {
                        name: sym.name.clone(),
                        kind: aivi_lsp_kind_to_symbol_kind(sym.kind),
                        tags: None,
                        deprecated: None,
                        location: Location {
                            uri: uri.clone(),
                            range: lsp_range,
                        },
                        container_name: None,
                    });
                }
                stack.extend(sym.children.iter());
            }
        }

        Ok(Some(results))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        Ok(crate::semantic_tokens::semantic_tokens_full(params, Arc::clone(&self.state)).await)
    }
}

fn aivi_lsp_kind_to_symbol_kind(kind: aivi_hir::LspSymbolKind) -> SymbolKind {
    match kind {
        aivi_hir::LspSymbolKind::File => SymbolKind::FILE,
        aivi_hir::LspSymbolKind::Module => SymbolKind::MODULE,
        aivi_hir::LspSymbolKind::Namespace => SymbolKind::NAMESPACE,
        aivi_hir::LspSymbolKind::Package => SymbolKind::PACKAGE,
        aivi_hir::LspSymbolKind::Class => SymbolKind::CLASS,
        aivi_hir::LspSymbolKind::Method => SymbolKind::METHOD,
        aivi_hir::LspSymbolKind::Property => SymbolKind::PROPERTY,
        aivi_hir::LspSymbolKind::Field => SymbolKind::FIELD,
        aivi_hir::LspSymbolKind::Constructor => SymbolKind::CONSTRUCTOR,
        aivi_hir::LspSymbolKind::Enum => SymbolKind::ENUM,
        aivi_hir::LspSymbolKind::Interface => SymbolKind::INTERFACE,
        aivi_hir::LspSymbolKind::Function => SymbolKind::FUNCTION,
        aivi_hir::LspSymbolKind::Variable => SymbolKind::VARIABLE,
        aivi_hir::LspSymbolKind::Constant => SymbolKind::CONSTANT,
        aivi_hir::LspSymbolKind::String => SymbolKind::STRING,
        aivi_hir::LspSymbolKind::Number => SymbolKind::NUMBER,
        aivi_hir::LspSymbolKind::Boolean => SymbolKind::BOOLEAN,
        aivi_hir::LspSymbolKind::Array => SymbolKind::ARRAY,
        aivi_hir::LspSymbolKind::Object => SymbolKind::OBJECT,
        aivi_hir::LspSymbolKind::Key => SymbolKind::KEY,
        aivi_hir::LspSymbolKind::Null => SymbolKind::NULL,
        aivi_hir::LspSymbolKind::EnumMember => SymbolKind::ENUM_MEMBER,
        aivi_hir::LspSymbolKind::Struct => SymbolKind::STRUCT,
        aivi_hir::LspSymbolKind::Event => SymbolKind::EVENT,
        aivi_hir::LspSymbolKind::Operator => SymbolKind::OPERATOR,
        aivi_hir::LspSymbolKind::TypeParameter => SymbolKind::TYPE_PARAMETER,
    }
}
