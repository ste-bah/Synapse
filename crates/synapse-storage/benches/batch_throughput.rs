use std::{
    error::Error,
    hint::black_box,
    time::{SystemTime, UNIX_EPOCH},
};

use synapse_storage::{Db, cf};

const ROWS: usize = 10_000;
const TARGET_MS: u128 = 200;

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::temp_dir().join(format!(
        "synapse-storage-batch-bench-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let db = Db::open(&root.join("db"), 7)?;
    let kvs = event_rows(ROWS);
    let expected_bytes = kvs.iter().map(|(_key, value)| value.len()).sum::<usize>();
    println!(
        "source_of_truth=cf_scan bench=batch_throughput before_count={} expected_count={} expected_bytes={expected_bytes}",
        db.scan_cf(cf::CF_EVENTS)?.len(),
        ROWS
    );

    let started = std::time::Instant::now();
    db.put_batch(cf::CF_EVENTS, black_box(kvs))?;
    db.flush()?;
    let elapsed_ms = started.elapsed().as_millis();

    let after = db.scan_cf(cf::CF_EVENTS)?;
    let after_bytes = after.iter().map(|(_key, value)| value.len()).sum::<usize>();
    println!(
        "source_of_truth=cf_scan bench=batch_throughput after_count={} after_bytes={after_bytes} elapsed_ms={elapsed_ms} target_ms={TARGET_MS} final_value=pass:{}",
        after.len(),
        elapsed_ms <= TARGET_MS
    );
    assert_eq!(after.len(), ROWS);
    assert_eq!(after_bytes, expected_bytes);
    assert!(
        elapsed_ms <= TARGET_MS,
        "batch_throughput {elapsed_ms} ms exceeded {TARGET_MS} ms"
    );
    drop(db);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn event_rows(count: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    (0..count)
        .map(|index| {
            (
                format!("{index:016x}").into_bytes(),
                format!(r#"{{"ts_ns":{index},"event":"synthetic"}}"#).into_bytes(),
            )
        })
        .collect()
}
