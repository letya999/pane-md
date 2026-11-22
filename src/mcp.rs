use std::{path::PathBuf, process::Stdio};

use pane_md::{lint, markdown::Markdown};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

use crate::cli::{HAS_DESKTOP_EDITOR, comments_json, launch_gui, read_document};

#[derive(Deserialize, JsonSchema)]
struct DocumentPath {
    /// Local document path, absolute or relative to the server working directory.
    path: String,
}

struct Server {
    router: ToolRouter<Self>,
}

impl Server {
    fn new() -> Self {
        let mut router = Self::tool_router();
        if !HAS_DESKTOP_EDITOR {
            router.remove_route("open_document");
        }
        Self { router }
    }
}

fn failure(error: impl std::fmt::Display) -> ErrorData {
    eprintln!("pane-md MCP: {error}");
    ErrorData::internal_error(error.to_string(), None)
}

async fn inspect(
    path: String,
    inspect: impl FnOnce(PathBuf, String) -> Result<CallToolResult, ErrorData> + Send + 'static,
) -> Result<CallToolResult, ErrorData> {
    tokio::task::spawn_blocking(move || {
        let path = std::path::absolute(path).map_err(failure)?;
        let source = read_document(&path).map_err(failure)?;
        inspect(path, source)
    })
    .await
    .map_err(failure)?
}

#[tool_router]
impl Server {
    #[tool(description = "Open a local Markdown document in WYSIWYG View and return immediately.")]
    async fn open_document(
        &self,
        Parameters(input): Parameters<DocumentPath>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        inspect(input.path, |path, _| {
            launch_gui(Some(&path), Stdio::null()).map_err(failure)?;
            Ok(CallToolResult::structured(
                json!({ "path": path.to_string_lossy(), "status": "launched" }),
            ))
        })
        .await
    }

    #[tool(
        description = "Read inline review comments and their anchor status from a saved local Markdown document."
    )]
    async fn list_comments(
        &self,
        Parameters(input): Parameters<DocumentPath>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        inspect(input.path, |path, text| {
            let doc = Markdown::parse(&text);
            for diagnostic in &doc.diagnostics {
                eprintln!(
                    "pane-md MCP: {}:{}: {}",
                    path.display(),
                    diagnostic.line,
                    diagnostic.message
                );
            }
            Ok(CallToolResult::structured(comments_json(&path, doc)))
        })
        .await
    }

    #[tool(
        description = "Lint a saved Markdown file for broken local links, heading anchors and review records."
    )]
    async fn lint_document(
        &self,
        Parameters(input): Parameters<DocumentPath>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        inspect(input.path, |path, source| {
            Ok(CallToolResult::structured(
                json!({ "path": path.to_string_lossy(), "issues": lint::check_at(&source, &path) }),
            ))
        })
        .await
    }
}

#[tool_handler(router = self.router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Inspect and lint local Markdown documents and their inline review comments.",
        )
    }
}

pub fn run() -> super::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(async {
            Server::new()
                .serve(rmcp::transport::stdio())
                .await?
                .waiting()
                .await?;
            Ok(())
        })
}
