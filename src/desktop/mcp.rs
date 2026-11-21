use std::{fs, path::PathBuf};

use notepad_md::markdown::Markdown;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

use super::{file::decode, spawn_gui};

#[derive(Deserialize, JsonSchema)]
struct DocumentPath {
    /// Local document path, absolute or relative to the server working directory.
    path: String,
}

struct Server;

fn failure(error: impl std::fmt::Display) -> ErrorData {
    eprintln!("notepad-md MCP: {error}");
    ErrorData::internal_error(error.to_string(), None)
}

#[tool_router]
impl Server {
    #[tool(description = "Open a local Markdown document in WYSIWYG View and return immediately.")]
    async fn open_document(
        &self,
        Parameters(input): Parameters<DocumentPath>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let path = std::path::absolute(PathBuf::from(input.path)).map_err(failure)?;
        let bytes = fs::read(&path).map_err(failure)?;
        decode(&bytes).map_err(failure)?;
        spawn_gui(Some(&path)).map_err(failure)?;
        Ok(CallToolResult::structured(
            json!({ "path": path, "status": "launched" }),
        ))
    }

    #[tool(
        description = "Read inline review comments and their anchor status from a saved local Markdown document."
    )]
    async fn list_comments(
        &self,
        Parameters(input): Parameters<DocumentPath>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let path = std::path::absolute(PathBuf::from(input.path)).map_err(failure)?;
        let bytes = fs::read(&path).map_err(failure)?;
        let (_, text) = decode(&bytes).map_err(failure)?;
        let doc = Markdown::parse(text);
        for diagnostic in &doc.diagnostics {
            eprintln!(
                "notepad-md MCP: {}:{}: {}",
                path.display(),
                diagnostic.line,
                diagnostic.message
            );
        }
        Ok(CallToolResult::structured(json!({
            "path": path,
            "comments": doc.records,
            "diagnostics": doc.diagnostics
        })))
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Local notepad-md file review. Saving in the GUI writes inline Markdown comments.",
        )
    }
}

pub fn run() -> super::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            Server
                .serve(rmcp::transport::stdio())
                .await?
                .waiting()
                .await?;
            Ok(())
        })
}
