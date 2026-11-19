//! MCP `workboard` por stdio. Lo lanza el agente (Claude Code, Copilot…) con la config que arma la app.
//! Variables: `WORKBOARD_SESSION`, `WORKBOARD_MCP_WRITE`, `WORKBOARD_ROOT`, `WORKBOARD_CONFIG_DIR`.

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use wb_mcp::{Ctx, ItemUpdate, ToolResult};

#[derive(Clone)]
struct Server {
    ctx: Ctx,
    tool_router: ToolRouter<Self>,
}

fn reply(ctx: &Ctx, result: ToolResult) -> Result<CallToolResult, McpError> {
    Ok(match result {
        Ok(v) => CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&v).unwrap())]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e.render(&ctx.lang()))]),
    })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListItems {
    /// Text to look for in the folder name, id, title or notes.
    #[serde(default)]
    search: String,
    /// Only items in this status.
    #[serde(default)]
    status: String,
    /// Include closed items (done or cancelled). By default they are left out.
    #[serde(default)]
    include_closed: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ItemRef {
    /// Work item id or folder name.
    item: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UpdateItem {
    /// Work item id or folder name.
    item: String,
    status: Option<String>,
    /// Complete list of environments where it already ran.
    environments: Option<Vec<String>>,
    /// One-line summary of what is left.
    notes: Option<String>,
    branch: Option<String>,
    #[serde(rename = "type")]
    item_type: Option<String>,
    /// Agreed date (YYYY-MM-DD). Only when the user gives it.
    due: Option<String>,
    /// Agreed hours. Only when the user gives them.
    hours: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddTask {
    item: String,
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CompleteTask {
    item: String,
    /// Task index (from workboard_get_item) or its text (exact, or a fragment matching a single task).
    task: String,
    /// false = reopen it.
    #[serde(default = "yes")]
    done: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MarkRan {
    item: String,
    /// File name or its path inside the item folder.
    file: String,
    environment: String,
    /// false = remove the mark.
    #[serde(default = "yes")]
    ran: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddNote {
    item: String,
    title: String,
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RunTool {
    name: String,
    #[serde(default)]
    args: Vec<String>,
    /// Item whose folder is the working directory (default: the first one of the session).
    #[serde(default)]
    item: String,
}

fn yes() -> bool {
    true
}

#[tool_router]
impl Server {
    fn new(ctx: Ctx) -> Self {
        Self { ctx, tool_router: Self::tool_router() }
    }

    #[tool(
        description = "Mode (session, write or read-only), the root folder and the work items of this session with their status, tasks and folders. Call it first."
    )]
    async fn workboard_session(&self) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::session(&self.ctx))
    }

    #[tool(
        description = "Search work items by text (folder, id, title, notes) and/or status. Closed items are left out unless asked for."
    )]
    async fn workboard_list_items(
        &self,
        Parameters(p): Parameters<ListItems>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::list_items(&self.ctx, &p.search, &p.status, p.include_closed))
    }

    #[tool(
        description = "Details of a work item: frontmatter, tasks with their index, files (and where scripts already ran) and the deploy section of its README."
    )]
    async fn workboard_get_item(
        &self,
        Parameters(p): Parameters<ItemRef>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::get_item(&self.ctx, &p.item))
    }

    #[tool(
        description = "Change status, type, environments (full list), notes (one line: what is left), branch, agreed date or agreed hours of a work item. Only the fields you pass change; \"\" clears a field. Date and hours only when the user gives them."
    )]
    async fn workboard_update_item(
        &self,
        Parameters(p): Parameters<UpdateItem>,
    ) -> Result<CallToolResult, McpError> {
        let upd = ItemUpdate {
            status: p.status,
            environments: p.environments,
            notes: p.notes,
            branch: p.branch,
            item_type: p.item_type,
            due: p.due,
            hours: p.hours,
        };
        reply(&self.ctx, wb_mcp::update_item(&self.ctx, &p.item, &upd))
    }

    #[tool(
        description = "Add an open task to the task section of the work item README (creates the section if needed)."
    )]
    async fn workboard_add_task(
        &self,
        Parameters(p): Parameters<AddTask>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::add_task(&self.ctx, &p.item, &p.text))
    }

    #[tool(description = "Check (or uncheck with done=false) a task of the work item.")]
    async fn workboard_complete_task(
        &self,
        Parameters(p): Parameters<CompleteTask>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::complete_task(&self.ctx, &p.item, &p.task, p.done))
    }

    #[tool(
        description = "Record that the user ran a script of the work item in an environment (or remove the mark with ran=false). Only when the user says they ran it, never on your own."
    )]
    async fn workboard_mark_ran(
        &self,
        Parameters(p): Parameters<MarkRan>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::mark_ran(&self.ctx, &p.item, &p.file, &p.environment, p.ran))
    }

    #[tool(
        description = "Create a personal note (markdown) linked to the work item: findings, decisions, things to remember that are not tasks. It goes to the user's notes folder, outside the project."
    )]
    async fn workboard_add_note(
        &self,
        Parameters(p): Parameters<AddNote>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::add_note(&self.ctx, &p.item, &p.title, &p.text))
    }

    #[tool(
        description = "Scripts the user promoted as reusable tools. Before writing a script for something repetitive, check here."
    )]
    async fn workboard_list_tools(&self) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::list_tools(&self.ctx))
    }

    #[tool(
        description = "Run a tool from workboard_list_tools in the folder of a work item. Only inside a Workboard session."
    )]
    async fn workboard_run_tool(
        &self,
        Parameters(p): Parameters<RunTool>,
    ) -> Result<CallToolResult, McpError> {
        reply(&self.ctx, wb_mcp::run_tool(&self.ctx, &p.name, &p.args, &p.item))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("workboard", env!("CARGO_PKG_VERSION")))
            .with_instructions(wb_mcp::instructions(&self.ctx))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Server::new(Ctx::from_env()).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
