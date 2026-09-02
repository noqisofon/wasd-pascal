//! WASD Pascal LSPサーバー。
//!
//! 今回のスコープは診断表示（`textDocument/publishDiagnostics`）のみ。
//! ホバー・補完・定義へジャンプ等は次段階に回す（`README.md`参照）。
//!
//! `wasd-driver::compile`をそのまま呼び出し、返ってきた
//! `wasd_ast::Diagnostic`のリストをLSPの`Diagnostic`型へ変換して配信する
//! （変換ロジックは[`diagnostics`]・[`position`]モジュール）。dialectは
//! 今回は起動時に固定（デフォルトのISO 7185）で動作する。

mod diagnostics;
mod position;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use wasd_driver::CompileOptions;

use diagnostics::to_lsp_diagnostics;

struct Backend {
    client: Client,
    /// 開いているドキュメントの現在の全文。`did_open`/`did_change`で更新する。
    documents: Arc<RwLock<HashMap<Url, String>>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// `text`を固定dialect（デフォルトのISO 7185）でコンパイルし、診断を
    /// `uri`に対して配信する。
    async fn publish_diagnostics_for(&self, uri: Url, text: &str) {
        let result = wasd_driver::compile(text, &CompileOptions::default());
        let diagnostics = to_lsp_diagnostics(&result.diagnostics, text);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                // 今回はdiagnosticsのみなので、他のcapabilityは宣言しない。
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "wasd-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;

        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.publish_diagnostics_for(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // FULL同期なので、最後のcontent changeが変更後の全文を持つ。
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        let text = change.text;

        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.publish_diagnostics_for(uri, &text).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        // ドキュメントが閉じられたら、エディタ上の当該ファイルに対する
        // 診断表示を消しておく。
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
