use std::{collections::BTreeMap, time::Instant};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Json},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use synapse_core::Health;

#[derive(Debug, Clone)]
pub struct SynapseService {
    started_at: Instant,
    tool_router: ToolRouter<Self>,
}

impl SynapseService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
        }
    }

    fn health_payload(&self) -> Health {
        Health {
            ok: true,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: option_env!("VERGEN_GIT_SHA").unwrap_or("dev").to_owned(),
            uptime_s: self.started_at.elapsed().as_secs(),
            subsystems: BTreeMap::new(),
        }
    }
}

impl Default for SynapseService {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl SynapseService {
    #[tool(description = "Return server health")]
    pub async fn health(&self) -> Json<Health> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "health",
            "tool.invocation kind=health"
        );
        Json(self.health_payload())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SynapseService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "synapse-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Synapse M0 health server")
    }
}

#[cfg(test)]
mod tests {
    use super::SynapseService;

    #[test]
    fn health_payload_is_m0_hardcoded() {
        let service = SynapseService::new();
        let payload = service.health_payload();
        assert!(payload.ok);
        assert_eq!(payload.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(payload.build, "dev");
        assert!(payload.subsystems.is_empty());
    }

    #[test]
    fn uptime_uses_monotonic_elapsed() {
        let service = SynapseService::new();
        let first = service.health_payload().uptime_s;
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = service.health_payload().uptime_s;
        assert!(second >= first);
    }
}
