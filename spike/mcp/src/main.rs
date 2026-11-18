//! Spike: MCP `workboard` mínimo en Rust (rmcp, stdio). Lista los WI de una raíz `_work` leyendo el
//! `estado:` del frontmatter de cada README. Raíz: `WORKBOARD_ROOT`.
use std::path::PathBuf;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde::Deserialize;

#[derive(Clone)]
struct Workboard {
    root: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetWi {
    /// Carpeta del WI, relativa a la raíz (p. ej. `wi123456-algo`).
    folder: String,
}

fn estado(readme: &str) -> Option<String> {
    let body = readme.strip_prefix("---")?;
    let end = body.find("\n---")?;
    body[..end]
        .lines()
        .find_map(|l| l.strip_prefix("estado:").map(|v| v.trim().to_string()))
}

#[tool_router]
impl Workboard {
    fn new(root: PathBuf) -> Self {
        Self { root, tool_router: Self::tool_router() }
    }

    #[tool(description = "Lista los work items de la raíz con su estado")]
    async fn workboard_list_wis(&self) -> Result<CallToolResult, McpError> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&self.root).map_err(|e| McpError::internal_error(e.to_string(), None))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !e.path().is_dir() || name.starts_with(['_', '.']) {
                continue;
            }
            let st = std::fs::read_to_string(e.path().join("README.md")).ok().and_then(|r| estado(&r));
            out.push(format!("{name}: {}", st.unwrap_or_else(|| "-".into())));
        }
        out.sort();
        Ok(CallToolResult::success(vec![ContentBlock::text(out.join("\n"))]))
    }

    #[tool(description = "Devuelve el README de un work item")]
    async fn workboard_get_wi(&self, Parameters(p): Parameters<GetWi>) -> Result<CallToolResult, McpError> {
        let path = self.root.join(&p.folder).join("README.md");
        // Paths seguros: nada fuera de la raíz.
        let canon = path.canonicalize().map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        if !canon.starts_with(self.root.canonicalize().unwrap()) {
            return Err(McpError::invalid_params("fuera de la raíz", None));
        }
        let text = std::fs::read_to_string(canon).map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Workboard {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Spike del MCP workboard en Rust: estado de los WI de la carpeta _work.")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = std::env::var("WORKBOARD_ROOT").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    let service = Workboard::new(root).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
