mod auth;
mod session;
pub mod sse;
mod transport;

use crate::{m2::M2ServiceConfig, m3::M3ServiceConfig};

pub async fn serve(
    bind: &str,
    allow_non_loopback: bool,
    m2_config: &M2ServiceConfig,
    m3_config: M3ServiceConfig,
) -> anyhow::Result<std::process::ExitCode> {
    transport::serve(bind, allow_non_loopback, m2_config, m3_config).await
}
