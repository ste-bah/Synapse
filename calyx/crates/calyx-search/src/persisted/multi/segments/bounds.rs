use calyx_core::{CxId, SlotId};

use super::super::{stale, unbounded_multi_sidecar};
use super::MultiSegmentRef;
use crate::error::CliResult;

pub(super) const DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;

pub(super) const BINARY_HEADER_BYTES: u64 = 16 + 2 + 4 + 8 + 8 + 8;
const ROW_HEADER_BYTES: u64 = 16 + 4;
const F32_BYTES: u64 = 4;

pub(in crate::persisted) fn ensure_streaming_row_bounded(
    slot: SlotId,
    cx_id: CxId,
    token_dim: u32,
    token_count: u32,
    encoded_bytes: usize,
) -> CliResult {
    let estimated = BINARY_HEADER_BYTES
        .checked_add(row_estimated_bytes(token_dim, token_count as usize)?)
        .ok_or_else(|| stale("persistent binary multi row byte count overflow"))?;
    if estimated > DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES {
        return Err(unbounded_multi_sidecar(format!(
            "encoded multi row {cx_id} for slot {slot} has {encoded_bytes} source bytes and is estimated {estimated} sidecar bytes; exceeds search binary segment limit {} bytes before decode (tokens={token_count}, token_dim={token_dim})",
            DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES
        )));
    }
    Ok(())
}

pub(in crate::persisted::multi) fn ensure_entry_bounded(
    slot: SlotId,
    rel: &str,
    token_dim: u32,
    row_count: usize,
    token_count: usize,
) -> CliResult {
    ensure_bounded(
        slot,
        rel,
        token_dim,
        row_count,
        token_count,
        DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
    )
}

pub(super) fn ensure_segment_ref_bounded(
    slot: SlotId,
    token_dim: u32,
    segment: &MultiSegmentRef,
) -> CliResult {
    ensure_bounded(
        slot,
        &segment.index_rel,
        token_dim,
        segment.row_count,
        segment.token_count,
        DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
    )
}

fn ensure_bounded(
    slot: SlotId,
    rel: &str,
    token_dim: u32,
    row_count: usize,
    token_count: usize,
    max_bytes: u64,
) -> CliResult {
    let estimated = segment_estimated_bytes(token_dim, row_count, token_count)?;
    if estimated > max_bytes {
        return Err(unbounded_multi_sidecar(format!(
            "persistent binary multi segment for slot {slot} is estimated {estimated} bytes at {rel}; exceeds search binary segment limit {max_bytes} bytes (rows={row_count}, tokens={token_count}, token_dim={token_dim})"
        )));
    }
    Ok(())
}

pub(super) fn segment_estimated_bytes(
    token_dim: u32,
    row_count: usize,
    token_count: usize,
) -> CliResult<u64> {
    let row_headers = (row_count as u64)
        .checked_mul(ROW_HEADER_BYTES)
        .ok_or_else(|| stale("persistent binary multi segment row byte count overflow"))?;
    let payload = (token_count as u64)
        .checked_mul(token_dim as u64)
        .and_then(|components| components.checked_mul(F32_BYTES))
        .ok_or_else(|| stale("persistent binary multi segment token byte count overflow"))?;
    BINARY_HEADER_BYTES
        .checked_add(row_headers)
        .and_then(|sum| sum.checked_add(payload))
        .ok_or_else(|| stale("persistent binary multi segment byte count overflow"))
}

pub(super) fn row_estimated_bytes(token_dim: u32, token_count: usize) -> CliResult<u64> {
    let payload = (token_count as u64)
        .checked_mul(token_dim as u64)
        .and_then(|components| components.checked_mul(F32_BYTES))
        .ok_or_else(|| stale("persistent binary multi row token byte count overflow"))?;
    ROW_HEADER_BYTES
        .checked_add(payload)
        .ok_or_else(|| stale("persistent binary multi row byte count overflow"))
}
