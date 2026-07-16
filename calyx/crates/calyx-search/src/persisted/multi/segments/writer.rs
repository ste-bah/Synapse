use super::*;

pub(in crate::persisted) struct StreamingSegmentsWriter {
    vault_dir: PathBuf,
    root: PathBuf,
    slot: SlotId,
    token_dim: u32,
    base_seq: u64,
    current_rows: Vec<EncodedMultiRow>,
    current_bytes: u64,
    row_count: usize,
    token_count: usize,
    segments: Vec<MultiSegmentRef>,
    max_segment_bytes: u64,
}

#[derive(Debug)]
pub(in crate::persisted) struct SegmentFlush {
    pub(in crate::persisted) ordinal: usize,
    pub(in crate::persisted) row_count: usize,
    pub(in crate::persisted) total_rows: usize,
    pub(in crate::persisted) detail: String,
}

impl StreamingSegmentsWriter {
    pub(in crate::persisted) fn new(
        vault_dir: &Path,
        root: &Path,
        slot: SlotId,
        token_dim: u32,
        base_seq: u64,
    ) -> Self {
        Self::with_max_segment_bytes(
            vault_dir,
            root,
            slot,
            token_dim,
            base_seq,
            bounds::DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
        )
    }

    fn with_max_segment_bytes(
        vault_dir: &Path,
        root: &Path,
        slot: SlotId,
        token_dim: u32,
        base_seq: u64,
        max_segment_bytes: u64,
    ) -> Self {
        Self {
            vault_dir: vault_dir.to_path_buf(),
            root: root.to_path_buf(),
            slot,
            token_dim,
            base_seq,
            current_rows: Vec::new(),
            current_bytes: bounds::BINARY_HEADER_BYTES,
            row_count: 0,
            token_count: 0,
            segments: Vec::new(),
            max_segment_bytes,
        }
    }

    pub(in crate::persisted) fn push(
        &mut self,
        cx_id: CxId,
        tokens: Vec<Vec<f32>>,
    ) -> CliResult<Option<SegmentFlush>> {
        let token_count = u32::try_from(tokens.len()).map_err(|_| {
            unbounded_multi_sidecar(format!(
                "persistent multi row {cx_id} for slot {} has more than u32::MAX tokens",
                self.slot
            ))
        })?;
        let bytes = encode_slot_vector(&SlotVector::Multi {
            token_dim: self.token_dim,
            tokens,
        })?;
        self.push_encoded(cx_id, token_count, bytes)
    }

    pub(in crate::persisted) fn push_encoded(
        &mut self,
        cx_id: CxId,
        token_count: u32,
        bytes: Vec<u8>,
    ) -> CliResult<Option<SegmentFlush>> {
        let encoded = EncodedMultiSlotVector::new(&bytes).map_err(|error| {
            CalyxError::aster_corrupt_shard(format!(
                "slot {} cx {cx_id} has malformed encoded multi payload: {}",
                self.slot, error.message
            ))
        })?;
        if encoded.token_dim() != self.token_dim || encoded.token_count() != token_count {
            return Err(stale(format!(
                "slot {} cx {cx_id} encoded multi shape mismatch: token_dim={} expected={}, token_count={} expected={token_count}",
                self.slot,
                encoded.token_dim(),
                self.token_dim,
                encoded.token_count()
            )));
        }
        let row_bytes = bounds::row_estimated_bytes(self.token_dim, token_count as usize)?;
        if bounds::BINARY_HEADER_BYTES + row_bytes > self.max_segment_bytes {
            return Err(unbounded_multi_sidecar(format!(
                "persistent multi row {cx_id} for slot {} is estimated {} bytes; exceeds search binary segment limit {} bytes (tokens={}, token_dim={})",
                self.slot,
                bounds::BINARY_HEADER_BYTES + row_bytes,
                self.max_segment_bytes,
                token_count,
                self.token_dim
            )));
        }
        let mut flushed = None;
        if !self.current_rows.is_empty() && self.current_bytes + row_bytes > self.max_segment_bytes
        {
            flushed = self.flush_current()?;
        }
        self.current_bytes += row_bytes;
        self.token_count = self
            .token_count
            .checked_add(token_count as usize)
            .ok_or_else(|| stale("streaming multi token_count overflow"))?;
        self.row_count = self
            .row_count
            .checked_add(1)
            .ok_or_else(|| stale("streaming multi row_count overflow"))?;
        self.current_rows.push(EncodedMultiRow {
            cx_id,
            token_count,
            bytes,
        });
        Ok(flushed)
    }

    pub(in crate::persisted) fn finish(
        mut self,
    ) -> CliResult<(SearchIndexEntry, Option<SegmentFlush>)> {
        let flushed = match self.flush_current() {
            Ok(flushed) => flushed,
            Err(primary) => {
                return match self.cleanup_segments() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(stale(format!(
                        "multi final segment write failed [{}] {}; cleanup also failed [{}] {}",
                        primary.code(),
                        primary.message(),
                        cleanup.code(),
                        cleanup.message()
                    ))),
                };
            }
        };
        let result = write_segments_manifest(
            &self.vault_dir,
            &self.root,
            self.slot,
            SegmentManifestBuild {
                token_dim: self.token_dim,
                row_count: self.row_count,
                token_count: self.token_count,
                base_seq: self.base_seq,
                segments: self.segments.clone(),
            },
        );
        match result {
            Ok(entry) => Ok((entry, flushed)),
            Err(primary) => match self.cleanup_segments() {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(stale(format!(
                    "multi segment manifest publish failed [{}] {}; cleanup also failed [{}] {}",
                    primary.code(),
                    primary.message(),
                    cleanup.code(),
                    cleanup.message()
                ))),
            },
        }
    }

    pub(in crate::persisted) fn abort(mut self) -> CliResult {
        self.current_rows.clear();
        self.cleanup_segments()
    }

    fn flush_current(&mut self) -> CliResult<Option<SegmentFlush>> {
        if self.current_rows.is_empty() {
            return Ok(None);
        }
        let ordinal = self.segments.len();
        let rows = std::mem::take(&mut self.current_rows);
        let segment = write_encoded_binary_segment(
            &self.vault_dir,
            &self.root,
            self.slot,
            self.token_dim,
            &rows,
            self.base_seq,
            ordinal,
        )?;
        let bytes = bounds::segment_estimated_bytes(
            self.token_dim,
            segment.row_count,
            segment.token_count,
        )?;
        let flush = SegmentFlush {
            ordinal,
            row_count: segment.row_count,
            total_rows: self.row_count,
            detail: format!(
                "segment={ordinal} rows={} tokens={} bytes={bytes} sha256={} path={}",
                segment.row_count, segment.token_count, segment.sha256, segment.index_rel
            ),
        };
        self.segments.push(segment);
        self.current_bytes = bounds::BINARY_HEADER_BYTES;
        Ok(Some(flush))
    }

    fn cleanup_segments(&mut self) -> CliResult {
        let mut failures = Vec::new();
        for segment in self.segments.drain(..) {
            let path = checked_segment_path(&self.vault_dir, &segment.index_rel, self.slot)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        if !failures.is_empty() {
            return Err(stale(format!(
                "failed to remove partial multi segments for slot {}: {}",
                self.slot,
                failures.join("; ")
            )));
        }
        Ok(())
    }
}
