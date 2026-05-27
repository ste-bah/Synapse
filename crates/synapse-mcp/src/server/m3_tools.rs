use super::{
    AudioTailParams, AudioTailResponse, AudioTranscribeParams, AudioTranscribeResponse,
    AuditIntelligenceQueryParams, AuditIntelligenceQueryResponse, ErrorData, Json, Parameters,
    ProfileActivateParams, ProfileActivateResponse, ProfileListParams, ProfileListResponse,
    ProfileQualityRefreshParams, ProfileQualityRefreshResponse, ProfileRegistryDisableParams,
    ProfileRegistryDisableResponse, ProfileRegistryExportParams, ProfileRegistryExportResponse,
    ProfileRegistryImportParams, ProfileRegistryImportResponse, ProfileRegistryInspectParams,
    ProfileRegistryInspectResponse, ProfileRegistryInstallParams, ProfileRegistryInstallResponse,
    ProfileRegistrySearchParams, ProfileRegistrySearchResponse, ReflexCancelParams,
    ReflexCancelResponse, ReflexHistoryParams, ReflexHistoryResponse, ReflexListParams,
    ReflexListResponse, ReflexRegisterParams, ReflexRegisterResponse, ReplayRecordParams,
    ReplayRecordResponse, StorageGcOnceParams, StorageGcOnceResponse, StorageInspectParams,
    StorageInspectResponse, StoragePressureSampleParams, StoragePressureSampleResponse,
    StoragePutProbeRowsParams, StoragePutProbeRowsResponse, SubscribeCancelParams,
    SubscribeCancelResponse, SubscribeParams, SubscribeResponse, SynapseService,
    apply_storage_pressure_sample, cancel_reflex, cancel_subscription, disable_registry_profile,
    export_registry, history_reflexes, import_registry, inspect_registry, inspect_storage,
    install_registry_package, list_profiles, list_reflexes, put_probe_rows,
    query_audit_intelligence, record_replay, refresh_profile_quality, register_reflex,
    run_storage_gc_once, search_registry, subscribe_to_events, tail_audio, tool, tool_router,
    transcribe_audio,
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
        self.ensure_supported_use_allows_action("reflex_register")?;
        self.refresh_reflex_audit_context()?;
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
        let response = match self.activate_profile_locked(&params.0, self.allow_unknown_profile()?)
        {
            Ok(response) => response,
            Err(error) => {
                self.persist_profile_activation_denied(&params.0.profile_id, &error);
                return Err(error);
            }
        };
        self.apply_backend_resolution_for_profile(&response.active_profile_id)?;
        self.persist_profile_activation_success(&response.active_profile_id, response.changed)?;
        Ok(Json(response))
    }

    #[tool(description = "Refresh local profile quality scoring from stored action audit rows")]
    pub async fn profile_quality_refresh(
        &self,
        params: Parameters<ProfileQualityRefreshParams>,
    ) -> Result<Json<ProfileQualityRefreshResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_quality_refresh",
            profile_id = %params.0.profile_id,
            max_audit_rows = params.0.max_audit_rows,
            "tool.invocation kind=profile_quality_refresh"
        );
        self.require_m3_permissions(
            "profile_quality_refresh",
            &crate::m3::profile_quality::required_permissions_refresh(&params.0),
        )?;
        let profile_runtime = self.profile_runtime()?;
        let reflex_runtime = self.reflex_runtime()?;
        refresh_profile_quality(&profile_runtime, &reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Search local profile registry rows")]
    pub async fn profile_registry_search(
        &self,
        params: Parameters<ProfileRegistrySearchParams>,
    ) -> Result<Json<ProfileRegistrySearchResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_search",
            row_kind = ?params.0.row_kind,
            include_disabled = params.0.include_disabled,
            limit = params.0.limit,
            "tool.invocation kind=profile_registry_search"
        );
        self.require_m3_permissions(
            "profile_registry_search",
            &crate::m3::profile_registry::required_permissions_search(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        search_registry(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Inspect one local profile registry row by key or id")]
    pub async fn profile_registry_inspect(
        &self,
        params: Parameters<ProfileRegistryInspectParams>,
    ) -> Result<Json<ProfileRegistryInspectResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_inspect",
            row_key = ?params.0.row_key,
            package_id = ?params.0.package_id,
            profile_id = ?params.0.profile_id,
            installed_profile_id = ?params.0.installed_profile_id,
            "tool.invocation kind=profile_registry_inspect"
        );
        self.require_m3_permissions(
            "profile_registry_inspect",
            &crate::m3::profile_registry::required_permissions_inspect(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        inspect_registry(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Install or update a local profile registry package manifest")]
    pub async fn profile_registry_install(
        &self,
        params: Parameters<ProfileRegistryInstallParams>,
    ) -> Result<Json<ProfileRegistryInstallResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_install",
            source_id = %params.0.source_id,
            manifest_path = %params.0.manifest_path,
            "tool.invocation kind=profile_registry_install"
        );
        self.require_m3_permissions(
            "profile_registry_install",
            &crate::m3::profile_registry::required_permissions_install(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        install_registry_package(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Disable or remove an installed local profile registry row")]
    pub async fn profile_registry_disable(
        &self,
        params: Parameters<ProfileRegistryDisableParams>,
    ) -> Result<Json<ProfileRegistryDisableResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_disable",
            profile_id = %params.0.profile_id,
            state = %params.0.state,
            "tool.invocation kind=profile_registry_disable"
        );
        self.require_m3_permissions(
            "profile_registry_disable",
            &crate::m3::profile_registry::required_permissions_disable(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        disable_registry_profile(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Export local profile registry rows to a JSON bundle")]
    pub async fn profile_registry_export(
        &self,
        params: Parameters<ProfileRegistryExportParams>,
    ) -> Result<Json<ProfileRegistryExportResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_export",
            output_path = %params.0.output_path,
            row_kind = ?params.0.row_kind,
            limit = params.0.limit,
            "tool.invocation kind=profile_registry_export"
        );
        self.require_m3_permissions(
            "profile_registry_export",
            &crate::m3::profile_registry::required_permissions_export(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        export_registry(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Import a local profile registry JSON bundle")]
    pub async fn profile_registry_import(
        &self,
        params: Parameters<ProfileRegistryImportParams>,
    ) -> Result<Json<ProfileRegistryImportResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_import",
            bundle_path = %params.0.bundle_path,
            "tool.invocation kind=profile_registry_import"
        );
        self.require_m3_permissions(
            "profile_registry_import",
            &crate::m3::profile_registry::required_permissions_import(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        import_registry(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Summarize profile-linked audit outcomes for registry intelligence")]
    pub async fn audit_intelligence_query(
        &self,
        params: Parameters<AuditIntelligenceQueryParams>,
    ) -> Result<Json<AuditIntelligenceQueryResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "audit_intelligence_query",
            profile_id = %params.0.profile_id,
            max_rows = params.0.max_rows,
            "tool.invocation kind=audit_intelligence_query"
        );
        self.require_m3_permissions(
            "audit_intelligence_query",
            &crate::m3::profile_registry::required_permissions_audit(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        query_audit_intelligence(&reflex_runtime, &params.0).map(Json)
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
