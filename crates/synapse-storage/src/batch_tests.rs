use super::*;
use std::{error::Error, time::Instant};

const TEST_SCHEMA_VERSION: u32 = 7;
const THROUGHPUT_ROWS: usize = 10_000;
const TARGET_MS: u128 = 200;

#[test]
fn batch_explicit_flush_round_trips_bytes_with_fsv() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?;
    let before = db.scan_cf(cf::CF_EVENTS)?;
    println!(
        "source_of_truth=cf_scan case=explicit_flush before_count={} before_bytes={}",
        before.len(),
        total_value_bytes(&before)
    );

    let expected = vec![
        (
            b"explicit-01".to_vec(),
            br#"{"ts_ns":1,"kind":"a"}"#.to_vec(),
        ),
        (
            b"explicit-02".to_vec(),
            br#"{"ts_ns":2,"kind":"b"}"#.to_vec(),
        ),
    ];
    db.put_batch(cf::CF_EVENTS, expected.clone())?;
    db.flush()?;

    let after = db.scan_cf(cf::CF_EVENTS)?;
    println!(
        "source_of_truth=cf_scan case=explicit_flush after_count={} after_bytes={} final_value={:?}",
        after.len(),
        total_value_bytes(&after),
        printable_rows(&after)
    );
    assert_eq!(sorted_rows(after), sorted_rows(expected));
    Ok(())
}

#[test]
fn batch_timer_flushes_after_interval_with_fsv() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?;
    let expected = vec![(b"timer-01".to_vec(), b"x".to_vec())];

    db.put_batch(cf::CF_ACTION_LOG, expected.clone())?;
    let before = db.scan_cf(cf::CF_ACTION_LOG)?;
    println!(
        "source_of_truth=cf_scan case=timer before_count={} before_bytes={}",
        before.len(),
        total_value_bytes(&before)
    );
    std::thread::sleep(batch::FLUSH_INTERVAL.saturating_mul(2));
    let after = db.scan_cf(cf::CF_ACTION_LOG)?;
    println!(
        "source_of_truth=cf_scan case=timer after_count={} after_bytes={} final_value={:?}",
        after.len(),
        total_value_bytes(&after),
        printable_rows(&after)
    );
    assert_eq!(sorted_rows(after), sorted_rows(expected));
    Ok(())
}

#[test]
fn batch_edges_empty_single_byte_and_size_boundary_with_fsv() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?;

    let before_empty = db.scan_cf(cf::CF_KV)?;
    let empty: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    db.put_batch(cf::CF_KV, empty)?;
    db.flush()?;
    let after_empty = db.scan_cf(cf::CF_KV)?;
    println!(
        "source_of_truth=cf_scan edge=empty before_count={} after_count={} final_value={:?}",
        before_empty.len(),
        after_empty.len(),
        after_empty
    );
    assert!(after_empty.is_empty());

    let single = vec![(b"single-byte".to_vec(), b"z".to_vec())];
    db.put_batch(cf::CF_KV, single.clone())?;
    db.flush()?;
    let after_single = db.scan_cf(cf::CF_KV)?;
    println!(
        "source_of_truth=cf_scan edge=single_byte after_count={} final_value={:?}",
        after_single.len(),
        printable_rows(&after_single)
    );
    assert_eq!(sorted_rows(after_single), sorted_rows(single));

    let boundary_value = vec![b'a'; batch::FLUSH_BYTES + 1];
    let boundary = vec![(b"size-boundary".to_vec(), boundary_value)];
    db.put_batch(cf::CF_EVENTS, boundary.clone())?;
    let after_boundary = db.scan_cf(cf::CF_EVENTS)?;
    println!(
        "source_of_truth=cf_scan edge=size_boundary after_count={} after_bytes={} final_value=key:{:?} value_len:{}",
        after_boundary.len(),
        total_value_bytes(&after_boundary),
        String::from_utf8_lossy(&boundary[0].0),
        boundary[0].1.len()
    );
    assert_eq!(sorted_rows(after_boundary), sorted_rows(boundary));
    Ok(())
}

#[test]
fn batch_throughput_10k_events_under_200ms_with_fsv() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let db = Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?;
    let kvs = event_rows(THROUGHPUT_ROWS);
    let expected_bytes = kvs.iter().map(|(_key, value)| value.len()).sum::<usize>();
    println!(
        "source_of_truth=cf_scan case=throughput before_count={} expected_count={} expected_bytes={expected_bytes}",
        db.scan_cf(cf::CF_EVENTS)?.len(),
        THROUGHPUT_ROWS
    );

    let started = Instant::now();
    db.put_batch(cf::CF_EVENTS, kvs)?;
    db.flush()?;
    let elapsed_ms = started.elapsed().as_millis();

    let after = db.scan_cf(cf::CF_EVENTS)?;
    let after_bytes = total_value_bytes(&after);
    println!(
        "source_of_truth=cf_scan case=throughput after_count={} after_bytes={after_bytes} elapsed_ms={elapsed_ms} target_ms={TARGET_MS} final_value=pass:{}",
        after.len(),
        elapsed_ms <= TARGET_MS
    );
    assert_eq!(after.len(), THROUGHPUT_ROWS);
    assert_eq!(after_bytes, expected_bytes);
    assert!(
        elapsed_ms <= TARGET_MS,
        "10k batch throughput took {elapsed_ms} ms, target {TARGET_MS} ms"
    );
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

fn sorted_rows(mut rows: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<(Vec<u8>, Vec<u8>)> {
    rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    rows
}

fn total_value_bytes(rows: &[(Vec<u8>, Vec<u8>)]) -> usize {
    rows.iter().map(|(_key, value)| value.len()).sum()
}

fn printable_rows(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(key, value)| {
            (
                String::from_utf8_lossy(key).into_owned(),
                String::from_utf8_lossy(value).into_owned(),
            )
        })
        .collect()
}
