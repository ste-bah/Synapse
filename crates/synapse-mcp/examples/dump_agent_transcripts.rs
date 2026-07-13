//! Developer readback utility: dump `CF_AGENT_TRANSCRIPTS` rows as JSON lines.
//!
//! Its output is supporting storage evidence only; manual FSV remains separate.
//! Run it against a stopped daemon's `--db` directory and diff what the store
//! actually holds against the spawn's raw `stdout.jsonl`. An optional spawn-id
//! argument restricts the dump to one spawn's rows.
//!
//! ```text
//! cargo run -p synapse-mcp --example dump_agent_transcripts -- <db-path> [spawn-id]
//! ```
//!
//! Errors out (non-zero exit) when the DB cannot be opened or a row fails
//! to decode — a corrupt transcript row is a finding, not something to
//! skip.

use synapse_storage::{
    Db,
    agent_transcripts::{agent_transcript_spawn_prefix, decode_agent_transcript_key},
    cf,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = std::env::args()
        .nth(1)
        .ok_or("usage: dump_agent_transcripts <db-path> [spawn-id]")?;
    let spawn_filter = std::env::args().nth(2);
    let db = Db::open(std::path::Path::new(&db_path), synapse_core::SCHEMA_VERSION)?;
    let rows = match &spawn_filter {
        Some(spawn_id) => db.scan_cf_prefix(
            cf::CF_AGENT_TRANSCRIPTS,
            &agent_transcript_spawn_prefix(spawn_id),
        )?,
        None => db.scan_cf(cf::CF_AGENT_TRANSCRIPTS)?,
    };
    let mut invalid = 0_usize;
    for (key, value) in &rows {
        let (spawn_id, line_no) = decode_agent_transcript_key(key)?;
        match serde_json::from_slice::<serde_json::Value>(value) {
            Ok(record) => println!("{spawn_id}\t{line_no}\t{record}"),
            Err(error) => {
                invalid += 1;
                eprintln!("INVALID ROW spawn_id={spawn_id} line_no={line_no}: {error}");
            }
        }
    }
    eprintln!("rows={} invalid={invalid}", rows.len());
    if invalid > 0 {
        return Err(format!("{invalid} transcript rows failed to decode").into());
    }
    Ok(())
}
