//! Developer readback utility: dump `CF_AGENT_EVENTS` journal rows as JSON lines.
//!
//! Its output is supporting storage evidence only; manual FSV remains separate.
//! Run it against a stopped daemon's `--db` directory and diff the physical
//! #897 agent-event journal (including the #898 state-machine rows) against
//! what the live tools reported.
//!
//! ```text
//! cargo run -p synapse-mcp --example dump_agent_events -- <db-path>
//! ```
//!
//! Errors out (non-zero exit) when the DB cannot be opened or a row fails to
//! decode — a corrupt journal row is a finding, not something to skip.

use synapse_storage::{Db, agent_events::decode_agent_event_key, cf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = std::env::args()
        .nth(1)
        .ok_or("usage: dump_agent_events <db-path>")?;
    let db = Db::open(std::path::Path::new(&db_path), synapse_core::SCHEMA_VERSION)?;
    let rows = db.scan_cf(cf::CF_AGENT_EVENTS)?;
    let mut invalid = 0usize;
    for (key, value) in &rows {
        let (ts_ns, seq) = decode_agent_event_key(key)?;
        match serde_json::from_slice::<serde_json::Value>(value) {
            Ok(record) => println!("{ts_ns}\t{seq}\t{record}"),
            Err(error) => {
                invalid += 1;
                eprintln!("INVALID ROW ts_ns={ts_ns} seq={seq}: {error}");
            }
        }
    }
    eprintln!("rows={} invalid={invalid}", rows.len());
    if invalid > 0 {
        return Err(format!("{invalid} journal rows failed to decode").into());
    }
    Ok(())
}
