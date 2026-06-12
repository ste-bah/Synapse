use super::{
    AudioTailParams, AudioTailResponse, AudioTranscribeParams, AudioTranscribeResponse,
    AuditExportBundleParams, AuditExportBundleResponse, AuditIntelligenceQueryParams,
    AuditIntelligenceQueryResponse, EpisodeSegmentParams, EpisodeSegmentResponse, ErrorData,
    HygieneFlagsParams, HygieneFlagsResponse, HygieneScanStorageParams, HygieneScanStorageResponse,
    HygieneScanTextParams, HygieneScanTextResponse, Json, Parameters, ProfileActivateParams,
    ProfileActivateResponse, ProfileAuthoringDecideParams, ProfileAuthoringDecideResponse,
    ProfileAuthoringExportParams, ProfileAuthoringExportResponse, ProfileAuthoringGenerateParams,
    ProfileAuthoringGenerateResponse, ProfileAuthoringInspectParams,
    ProfileAuthoringInspectResponse, ProfileAuthoringListParams, ProfileAuthoringListResponse,
    ProfileListParams, ProfileListResponse, ProfileQualityRefreshParams,
    ProfileQualityRefreshResponse, ProfileRegistryDisableParams, ProfileRegistryDisableResponse,
    ProfileRegistryExportParams, ProfileRegistryExportResponse, ProfileRegistryImportParams,
    ProfileRegistryImportResponse, ProfileRegistryInstallParams, ProfileRegistryInstallResponse,
    ProfileRegistryQueryParams, ProfileRegistryQueryResponse, ProfileRegistryRollbackParams,
    ProfileRegistryRollbackResponse, ReflexCancelParams, ReflexCancelResponse, ReflexHistoryParams,
    ReflexHistoryResponse, ReflexListParams, ReflexListResponse, ReflexRegisterParams,
    ReflexRegisterResponse, ReplayRecordParams, ReplayRecordResponse, StorageGcOnceParams,
    StorageGcOnceResponse, StorageInspectParams, StorageInspectResponse,
    StoragePressureSampleParams, StoragePressureSampleResponse, StoragePutProbeRowsParams,
    StoragePutProbeRowsResponse, SubscribeCancelParams, SubscribeCancelResponse, SubscribeParams,
    SubscribeResponse, SynapseService, TimelineExclusionsParams, TimelineExclusionsResponse,
    TimelinePauseParams, TimelinePauseResponse, TimelinePurgeParams, TimelinePurgeResponse,
    TimelineResumeParams, TimelineResumeResponse, TimelineSearchParams, TimelineSearchResponse,
    apply_storage_pressure_sample, cancel_reflex, cancel_subscription,
    decide_profile_authoring_candidate, disable_registry_profile, export_audit_bundle,
    export_profile_authoring_candidate, export_registry, generate_profile_authoring_candidate,
    history_reflexes, import_registry, inspect_profile_authoring_candidate, inspect_storage,
    install_registry_package, list_profile_authoring_candidates, list_profiles, list_reflexes,
    pause_timeline, purge_timeline, put_probe_rows, query_audit_intelligence, query_flags,
    query_registry, record_replay, refresh_profile_quality, register_reflex, resume_timeline,
    rollback_registry_profile, run_storage_gc_once, scan_storage, scan_text_tool, search_timeline,
    segment_episodes, subscribe_to_events, tail_audio, tool, tool_router, transcribe_audio,
    update_timeline_exclusions,
};
use rmcp::{RoleServer, service::RequestContext};

#[tool_router(router = m3_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(description = "Subscribe to filtered event notifications")]
    pub async fn subscribe(
        &self,
        params: Parameters<SubscribeParams>,
        request_context: RequestContext<RoleServer>,
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
        let owner_session_id =
            super::context::mcp_session_id_from_request_context(&request_context)?;
        let sse_state = self.sse_state()?;
        subscribe_to_events(&sse_state, &params.0, owner_session_id).map(Json)
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
        if let Err(error) = self.ensure_supported_use_allows_action("reflex_register") {
            self.audit_action_denied("reflex_register", &error);
            return Err(error);
        }
        self.refresh_reflex_audit_context()?;
        if crate::m3::reflex::requires_a11y_event_bridge(&params.0) {
            self.ensure_a11y_event_bridge()?;
        }
        let runtime = self.reflex_runtime()?;
        self.install_reflex_action_gate(&runtime)?;
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
        self.apply_profile_runtime_config_for_profile(&response.active_profile_id)?;
        self.persist_profile_activation_success(&response.active_profile_id, response.changed)?;
        Ok(Json(response))
    }

    #[tool(description = "Generate a local candidate profile patch from replay and audit evidence")]
    pub async fn profile_authoring_generate(
        &self,
        params: Parameters<ProfileAuthoringGenerateParams>,
    ) -> Result<Json<ProfileAuthoringGenerateResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_authoring_generate",
            profile_id = %params.0.profile_id,
            max_audit_rows = params.0.max_audit_rows,
            max_replay_rows = params.0.max_replay_rows,
            replay_path = ?params.0.replay_path,
            "tool.invocation kind=profile_authoring_generate"
        );
        self.require_m3_permissions(
            "profile_authoring_generate",
            &crate::m3::profile_authoring::required_permissions_generate(&params.0),
        )?;
        let profile_runtime = self.profile_runtime()?;
        let reflex_runtime = self.reflex_runtime()?;
        generate_profile_authoring_candidate(&profile_runtime, &reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "List local profile authoring candidates")]
    pub async fn profile_authoring_list(
        &self,
        params: Parameters<ProfileAuthoringListParams>,
    ) -> Result<Json<ProfileAuthoringListResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_authoring_list",
            profile_id = ?params.0.profile_id,
            state = ?params.0.state,
            limit = params.0.limit,
            "tool.invocation kind=profile_authoring_list"
        );
        self.require_m3_permissions(
            "profile_authoring_list",
            &crate::m3::profile_authoring::required_permissions_list(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        list_profile_authoring_candidates(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Inspect one local profile authoring candidate")]
    pub async fn profile_authoring_inspect(
        &self,
        params: Parameters<ProfileAuthoringInspectParams>,
    ) -> Result<Json<ProfileAuthoringInspectResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_authoring_inspect",
            candidate_id = %params.0.candidate_id,
            "tool.invocation kind=profile_authoring_inspect"
        );
        self.require_m3_permissions(
            "profile_authoring_inspect",
            &crate::m3::profile_authoring::required_permissions_inspect(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        inspect_profile_authoring_candidate(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Accept or reject one local profile authoring candidate")]
    pub async fn profile_authoring_decide(
        &self,
        params: Parameters<ProfileAuthoringDecideParams>,
    ) -> Result<Json<ProfileAuthoringDecideResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_authoring_decide",
            candidate_id = %params.0.candidate_id,
            decision = ?params.0.decision,
            "tool.invocation kind=profile_authoring_decide"
        );
        self.require_m3_permissions(
            "profile_authoring_decide",
            &crate::m3::profile_authoring::required_permissions_decide(&params.0),
        )?;
        let profile_runtime = self.profile_runtime()?;
        let reflex_runtime = self.reflex_runtime()?;
        decide_profile_authoring_candidate(&profile_runtime, &reflex_runtime, &params.0).map(Json)
    }

    #[tool(description = "Export a local profile authoring candidate bundle")]
    pub async fn profile_authoring_export(
        &self,
        params: Parameters<ProfileAuthoringExportParams>,
    ) -> Result<Json<ProfileAuthoringExportResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_authoring_export",
            candidate_id = %params.0.candidate_id,
            output_path = %params.0.output_path,
            "tool.invocation kind=profile_authoring_export"
        );
        self.require_m3_permissions(
            "profile_authoring_export",
            &crate::m3::profile_authoring::required_permissions_export(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        export_profile_authoring_candidate(&reflex_runtime, &params.0).map(Json)
    }

    #[tool(
        description = "Refresh local profile quality scoring from stored action, observation, and event rows"
    )]
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

    #[tool(description = "Query local profile registry rows by view: search, inspect, or report")]
    pub async fn profile_registry_query(
        &self,
        params: Parameters<ProfileRegistryQueryParams>,
    ) -> Result<Json<ProfileRegistryQueryResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_query",
            view = ?params.0.view,
            row_kind = ?params.0.row_kind,
            include_disabled = params.0.include_disabled,
            row_key = ?params.0.row_key,
            package_id = ?params.0.package_id,
            profile_id = ?params.0.profile_id,
            installed_profile_id = ?params.0.installed_profile_id,
            limit = params.0.limit,
            max_audit_rows = params.0.max_audit_rows,
            "tool.invocation kind=profile_registry_query"
        );
        self.require_m3_permissions(
            "profile_registry_query",
            &crate::m3::profile_registry::required_permissions_query(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        query_registry(&reflex_runtime, &params.0).map(Json)
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

    #[tool(description = "Rollback an installed profile registry row to a prior trusted package")]
    pub async fn profile_registry_rollback(
        &self,
        params: Parameters<ProfileRegistryRollbackParams>,
    ) -> Result<Json<ProfileRegistryRollbackResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile_registry_rollback",
            profile_id = %params.0.profile_id,
            target_package_id = ?params.0.target_package_id,
            target_package_version = ?params.0.target_package_version,
            "tool.invocation kind=profile_registry_rollback"
        );
        self.require_m3_permissions(
            "profile_registry_rollback",
            &crate::m3::profile_registry::required_permissions_rollback(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        rollback_registry_profile(&reflex_runtime, &params.0).map(Json)
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

    #[tool(description = "Export a local redacted audit bundle after explicit consent")]
    pub async fn audit_export_bundle(
        &self,
        params: Parameters<AuditExportBundleParams>,
    ) -> Result<Json<AuditExportBundleResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "audit_export_bundle",
            profile_id = %params.0.profile_id,
            output_path = %params.0.output_path,
            redaction_policy = ?params.0.redaction_policy,
            consent_present = params.0.consent.is_some(),
            max_rows = params.0.max_rows,
            "tool.invocation kind=audit_export_bundle"
        );
        self.require_m3_permissions(
            "audit_export_bundle",
            &crate::m3::audit_export::required_permissions_bundle(&params.0),
        )?;
        let reflex_runtime = self.reflex_runtime()?;
        export_audit_bundle(&reflex_runtime, &params.0).map(Json)
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

    #[tool(
        description = "Score one text blob for prompt-injection/adversarial instruction heuristics; optionally persist source-linked flag rows"
    )]
    pub async fn hygiene_scan_text(
        &self,
        params: Parameters<HygieneScanTextParams>,
    ) -> Result<Json<HygieneScanTextResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "hygiene_scan_text",
            text_bytes = params.0.text.len(),
            persist = params.0.persist,
            "tool.invocation kind=hygiene_scan_text"
        );
        self.require_m3_permissions(
            "hygiene_scan_text",
            &crate::m3::hygiene::required_permissions_scan_text(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        scan_text_tool(&runtime, &params.0).map(Json)
    }

    #[tool(
        description = "Batch-scan CF_OBSERVATIONS/CF_TIMELINE text fields for prompt-injection heuristics and persist source-linked flag rows"
    )]
    pub async fn hygiene_scan_storage(
        &self,
        params: Parameters<HygieneScanStorageParams>,
    ) -> Result<Json<HygieneScanStorageResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "hygiene_scan_storage",
            limit_rows = params.0.limit_rows,
            flag_limit = params.0.flag_limit,
            has_cursor = params.0.cursor.is_some(),
            "tool.invocation kind=hygiene_scan_storage"
        );
        self.require_m3_permissions(
            "hygiene_scan_storage",
            &crate::m3::hygiene::required_permissions_scan_storage(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        scan_storage(&runtime, &params.0).map(Json)
    }

    #[tool(description = "Query prompt-injection hygiene flag rows persisted in CF_KV")]
    pub async fn hygiene_flags(
        &self,
        params: Parameters<HygieneFlagsParams>,
    ) -> Result<Json<HygieneFlagsResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "hygiene_flags",
            source_cf = ?params.0.source_cf,
            source_key_hex = ?params.0.source_key_hex,
            limit = params.0.limit,
            has_cursor = params.0.cursor.is_some(),
            "tool.invocation kind=hygiene_flags"
        );
        self.require_m3_permissions(
            "hygiene_flags",
            &crate::m3::hygiene::required_permissions_flags(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        query_flags(&runtime, &params.0).map(Json)
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

    #[tool(
        description = "Search the operator activity timeline (CF_TIMELINE) by time range, app, kind, actor, and case-insensitive text over titles/paths/URLs; pages via cursor"
    )]
    pub async fn timeline_search(
        &self,
        params: Parameters<TimelineSearchParams>,
    ) -> Result<Json<TimelineSearchResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "timeline_search",
            start_ts_ns = params.0.start_ts_ns,
            end_ts_ns = params.0.end_ts_ns,
            has_text = params.0.text.is_some(),
            limit = params.0.limit,
            has_cursor = params.0.cursor.is_some(),
            "tool.invocation kind=timeline_search"
        );
        self.require_m3_permissions(
            "timeline_search",
            &crate::m3::timeline::required_permissions(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        search_timeline(&runtime, &params.0).map(Json)
    }

    #[tool(
        description = "Pause the operator activity timeline recorder: zero new rows across all feeds until timeline_resume. State survives daemon restart; optional duration_ms arms an auto-resume deadline"
    )]
    pub async fn timeline_pause(
        &self,
        params: Parameters<TimelinePauseParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<TimelinePauseResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "timeline_pause",
            duration_ms = params.0.duration_ms,
            "tool.invocation kind=timeline_pause"
        );
        self.require_m3_permissions(
            "timeline_pause",
            &crate::m3::timeline_control::required_permissions_pause(&params.0),
        )?;
        let by_session = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());
        pause_timeline(&self.m3_state, &params.0, &by_session).map(Json)
    }

    #[tool(
        description = "Resume the operator activity timeline recorder after timeline_pause; writes and verifies a session_start boundary row"
    )]
    pub async fn timeline_resume(
        &self,
        params: Parameters<TimelineResumeParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<TimelineResumeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "timeline_resume",
            "tool.invocation kind=timeline_resume"
        );
        self.require_m3_permissions(
            "timeline_resume",
            &crate::m3::timeline_control::required_permissions_resume(&params.0),
        )?;
        let by_session = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());
        resume_timeline(&self.m3_state, &params.0, &by_session).map(Json)
    }

    #[tool(
        description = "List or mutate the timeline recorder's per-process exclusion list; excluded executables (e.g. keepass.exe) never produce timeline rows. Env baseline (SYNAPSE_TIMELINE_EXCLUDE) is immutable at runtime"
    )]
    pub async fn timeline_exclusions(
        &self,
        params: Parameters<TimelineExclusionsParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<TimelineExclusionsResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "timeline_exclusions",
            add_count = params.0.add.as_deref().unwrap_or_default().len(),
            remove_count = params.0.remove.as_deref().unwrap_or_default().len(),
            "tool.invocation kind=timeline_exclusions"
        );
        self.require_m3_permissions(
            "timeline_exclusions",
            &crate::m3::timeline_control::required_permissions_exclusions(&params.0),
        )?;
        let by_session = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());
        update_timeline_exclusions(&self.m3_state, &params.0, &by_session).map(Json)
    }

    #[tool(
        description = "Hard-delete operator timeline rows matching the filters (same semantics as timeline_search) and write a counts-only audit row. Requires at least one filter or all=true; supports dry_run; purge audit rows are only deleted when kinds explicitly includes \"purge\""
    )]
    pub async fn timeline_purge(
        &self,
        params: Parameters<TimelinePurgeParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<TimelinePurgeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "timeline_purge",
            all = params.0.all,
            dry_run = params.0.dry_run,
            has_text = params.0.text.is_some(),
            has_cursor = params.0.cursor.is_some(),
            "tool.invocation kind=timeline_purge"
        );
        self.require_m3_permissions(
            "timeline_purge",
            &crate::m3::timeline::required_permissions_purge(&params.0),
        )?;
        let by_session = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());
        let runtime = self.reflex_runtime()?;
        purge_timeline(&runtime, &params.0, &by_session).map(Json)
    }

    #[tool(
        description = "Segment the operator activity timeline (CF_TIMELINE) into derived episodes (CF_EPISODES): contiguous spans of focused work with app, document, duration, and interaction summary. Deterministic and re-runnable; the range snaps outward to whole local days and each day is replaced atomically, so re-segmentation is idempotent"
    )]
    pub async fn episode_segment(
        &self,
        params: Parameters<EpisodeSegmentParams>,
    ) -> Result<Json<EpisodeSegmentResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "episode_segment",
            start_ts_ns = params.0.start_ts_ns,
            end_ts_ns = params.0.end_ts_ns,
            include_agent_activity = params.0.include_agent_activity,
            dry_run = params.0.dry_run,
            "tool.invocation kind=episode_segment"
        );
        self.require_m3_permissions(
            "episode_segment",
            &crate::m3::episodes::required_permissions(&params.0),
        )?;
        let runtime = self.reflex_runtime()?;
        segment_episodes(&runtime, &params.0).map(Json)
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
