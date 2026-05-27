use super::{
    AudioTailParams, AudioTailResponse, AudioTranscribeParams, AudioTranscribeResponse, ErrorData,
    Json, Parameters, ProfileActivateParams, ProfileActivateResponse, ProfileListParams,
    ProfileListResponse, ReflexCancelParams, ReflexCancelResponse, ReflexHistoryParams,
    ReflexHistoryResponse, ReflexListParams, ReflexListResponse, ReflexRegisterParams,
    ReflexRegisterResponse, ReplayRecordParams, ReplayRecordResponse, StorageGcOnceParams,
    StorageGcOnceResponse, StorageInspectParams, StorageInspectResponse,
    StoragePressureSampleParams, StoragePressureSampleResponse, StoragePutProbeRowsParams,
    StoragePutProbeRowsResponse, SubscribeCancelParams, SubscribeCancelResponse, SubscribeParams,
    SubscribeResponse, SynapseService, apply_storage_pressure_sample, cancel_reflex,
    cancel_subscription, history_reflexes, inspect_storage, list_profiles, list_reflexes,
    put_probe_rows, record_replay, register_reflex, run_storage_gc_once, subscribe_to_events,
    tail_audio, tool, tool_router, transcribe_audio,
};

#[tool_router(router = m3_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(description = "Subscribe to filtered event notifications")]
    pub async fn subscribe(
        &self,
        params: Parameters<SubscribeParams>,
    ) -> Result<Json<SubscribeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "subscribe",
            kinds_count = params.0.kinds.len(),
            snapshot_first = params.0.snapshot_first,
            buffer_size = params.0.buffer_size,
            "tool.invocation kind=subscribe"
        );
        self.require_m3_permissions(
            "subscribe",
            &crate::m3::subscribe::required_permissions(&params.0),
        )?;
        if crate::m3::subscribe::requires_a11y_event_bridge(&params.0) {
            self.ensure_a11y_event_bridge()?;
        }
        let sse_state = self.sse_state()?;
        subscribe_to_events(&sse_state, &params.0).map(Json)
    }

    #[tool(description = "Cancel an event subscription")]
    pub async fn subscribe_cancel(
        &self,
        params: Parameters<SubscribeCancelParams>,
    ) -> Result<Json<SubscribeCancelResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "subscribe_cancel",
            subscription_id = %params.0.subscription_id,
            "tool.invocation kind=subscribe_cancel"
        );
        self.require_m3_permissions(
            "subscribe_cancel",
            &crate::m3::subscribe::required_permissions_cancel(&params.0),
        )?;
        let sse_state = self.sse_state()?;
        cancel_subscription(&sse_state, &params.0).map(Json)
    }

    #[tool(description = "Register a reflex")]
    pub async fn reflex_register(
        &self,
        params: Parameters<ReflexRegisterParams>,
    ) -> Result<Json<ReflexRegisterResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "reflex_register",
            reflex_kind = %params.0.kind,
            priority = params.0.priority,
            "tool.invocation kind=reflex_register"
        );
        let required = crate::m3::reflex::required_permissions_register(&params.0)?;
        self.require_m3_permissions("reflex_register", &required)?;
        if crate::m3::reflex::requires_a11y_event_bridge(&params.0) {
            self.ensure_a11y_event_bridge()?;
        }
        let runtime = self.reflex_runtime()?;
        register_reflex(&runtime, params.0).map(Json)
    }

    #[tool(description = "Cancel a reflex")]
    pub async fn reflex_cancel(
        &self,
        params: Parameters<ReflexCancelParams>,
    ) -> Result<Json<ReflexCancelResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "reflex_cancel",
            reflex_id = %params.0.reflex_id,
            "tool.invocation kind=reflex_cancel"
        );
        self.require_m3_permissions(
            "reflex_cancel",
            &crate::m3::reflex::required_permissions_cancel(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        cancel_reflex(&runtime, &params.0).map(Json)
    }

    #[tool(description = "List registered reflexes")]
    pub async fn reflex_list(
        &self,
        params: Parameters<ReflexListParams>,
    ) -> Result<Json<ReflexListResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "reflex_list",
            include_expired = params.0.include_expired,
            "tool.invocation kind=reflex_list"
        );
        self.require_m3_permissions(
            "reflex_list",
            &crate::m3::reflex::required_permissions_list(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        list_reflexes(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Return persisted reflex audit history")]
    pub async fn reflex_history(
        &self,
        params: Parameters<ReflexHistoryParams>,
    ) -> Result<Json<ReflexHistoryResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "reflex_history",
            reflex_id = ?params.0.reflex_id,
            limit = params.0.limit,
            "tool.invocation kind=reflex_history"
        );
        self.require_m3_permissions(
            "reflex_history",
            &crate::m3::reflex::required_permissions_history(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        history_reflexes(&runtime, &params.0).map(Json)
    }

    #[tool(description = "List loaded profiles")]
    pub async fn profile_list(
        &self,
        params: Parameters<ProfileListParams>,
    ) -> Result<Json<ProfileListResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_list",
            "tool.invocation kind=profile_list"
        );
        self.require_m3_permissions(
            "profile_list",
            &crate::m3::profile::required_permissions_list(&params.0),
        )?;
        let runtime = self.profile_runtime()?;
        list_profiles(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Activate a loaded profile by id")]
    pub async fn profile_activate(
        &self,
        params: Parameters<ProfileActivateParams>,
    ) -> Result<Json<ProfileActivateResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_activate",
            profile_id = %params.0.profile_id,
            "tool.invocation kind=profile_activate"
        );
        self.require_m3_permissions(
            "profile_activate",
            &crate::m3::profile::required_permissions_activate(&params.0),
        )?;
        let response = self.activate_profile_locked(&params.0, self.allow_unknown_profile()?)?;
        self.apply_backend_resolution_for_profile(&response.active_profile_id)?;
        Ok(Json(response))
    }

    #[tool(description = "Record observations and/or events to a replay JSONL file")]
    pub async fn replay_record(
        &self,
        params: Parameters<ReplayRecordParams>,
    ) -> Result<Json<ReplayRecordResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "replay_record",
            target = %params.0.target,
            format = %params.0.format,
            duration_ms = params.0.duration_ms,
            "tool.invocation kind=replay_record"
        );
        self.require_m3_permissions(
            "replay_record",
            &crate::m3::replay::required_permissions(&params.0),
        )?;
        let sse_state = self.sse_state()?;
        record_replay(self.m1_state.clone(), sse_state, &params.0)
            .await
            .map(Json)
    }

    #[tool(description = "Return the latest loopback audio tail as PCM s16le bytes")]
    pub async fn audio_tail(
        &self,
        params: Parameters<AudioTailParams>,
    ) -> Result<Json<AudioTailResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "audio_tail",
            seconds = params.0.seconds,
            "tool.invocation kind=audio_tail"
        );
        self.require_m3_permissions(
            "audio_tail",
            &crate::m3::audio::required_permissions_tail(&params.0),
        )?;
        tail_audio(&self.m3_state, &params.0).map(Json)
    }

    #[tool(description = "Transcribe the latest loopback audio tail with Whisper tiny")]
    pub async fn audio_transcribe(
        &self,
        params: Parameters<AudioTranscribeParams>,
    ) -> Result<Json<AudioTranscribeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "audio_transcribe",
            seconds = params.0.seconds,
            language = %params.0.language,
            "tool.invocation kind=audio_transcribe"
        );
        self.require_m3_permissions(
            "audio_transcribe",
            &crate::m3::audio::required_permissions_transcribe(&params.0),
        )?;
        transcribe_audio(&self.m3_state, &params.0).map(Json)
    }

    #[tool(description = "Inspect storage sizes, row counts, and pressure state")]
    pub async fn storage_inspect(
        &self,
        params: Parameters<StorageInspectParams>,
    ) -> Result<Json<StorageInspectResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "storage_inspect",
            "tool.invocation kind=storage_inspect"
        );
        self.require_m3_permissions(
            "storage_inspect",
            &crate::m3::storage::required_permissions_inspect(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        inspect_storage(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Write bounded synthetic probe rows to a storage column family")]
    pub async fn storage_put_probe_rows(
        &self,
        params: Parameters<StoragePutProbeRowsParams>,
    ) -> Result<Json<StoragePutProbeRowsResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "storage_put_probe_rows",
            cf_name = %params.0.cf_name,
            rows = params.0.rows,
            value_bytes = params.0.value_bytes,
            "tool.invocation kind=storage_put_probe_rows"
        );
        self.require_m3_permissions(
            "storage_put_probe_rows",
            &crate::m3::storage::required_permissions_put(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        put_probe_rows(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Run one row-cap storage GC pass for diagnostics")]
    pub async fn storage_gc_once(
        &self,
        params: Parameters<StorageGcOnceParams>,
    ) -> Result<Json<StorageGcOnceResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "storage_gc_once",
            cf_name = %params.0.cf_name,
            soft_cap_rows = params.0.soft_cap_rows,
            hard_cap_rows = params.0.hard_cap_rows,
            "tool.invocation kind=storage_gc_once"
        );
        self.require_m3_permissions(
            "storage_gc_once",
            &crate::m3::storage::required_permissions_gc(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        run_storage_gc_once(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Apply one synthetic free-byte sample through storage pressure handling")]
    pub async fn storage_pressure_sample(
        &self,
        params: Parameters<StoragePressureSampleParams>,
    ) -> Result<Json<StoragePressureSampleResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "storage_pressure_sample",
            free_bytes = params.0.free_bytes,
            "tool.invocation kind=storage_pressure_sample"
        );
        self.require_m3_permissions(
            "storage_pressure_sample",
            &crate::m3::storage::required_permissions_pressure(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        apply_storage_pressure_sample(&runtime, &params.0).map(Json)
    }
}
