mod artifact;
mod command_query;
mod errors;
mod lifecycle;
mod response;
mod routing;
mod types;
mod util;
mod validation;

const AUDIT_TOOL: &str = "audit";
const REPLAY_TOOL: &str = "replay";
const AUDIT_SOT: &str =
    "CF_ACTION_LOG + daemon lifecycle JSONL ledgers + profile audit storage rows";
const REPLAY_SOT: &str =
    "Synapse replay JSONL artifacts + CF_KV demo-record row + CF_TIMELINE DemoMarker rows";
const DEFAULT_LIFECYCLE_LIMIT: usize = 20;
const MAX_LIFECYCLE_LIMIT: usize = 100;
const DEFAULT_MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_LINE_BYTES: usize = 512 * 1024;
const DEFAULT_ARTIFACT_MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_ARTIFACT_MAX_RECORDS: usize = 5_000;
const MAX_ARTIFACT_RECORDS: usize = 50_000;
