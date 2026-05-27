use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use synapse_action::{ActionStateSnapshot, RecordingBackend};
use synapse_core::{Health, SubsystemHealth, error_codes};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    http::sse::SseState,
    m1::{
        FindParams, FindResponse, M1State, ObserveParams, ReadTextParams, SetCaptureTargetParams,
        SetCaptureTargetResponse, SetPerceptionModeParams, SetPerceptionModeResponse,
        SharedM1State, assemble_observation, empty_input_schema, find_in_state, mcp_error,
        read_text_in_state, set_capture_target_in_state, set_perception_mode_in_state,
    },
    m2::{
        ActAimParams, ActAimResponse, ActClickParams, ActClickResponse, ActClipboardParams,
        ActClipboardResponse, ActDragParams, ActDragResponse, ActPadParams, ActPadResponse,
        ActPressParams, ActPressResponse, ActScrollParams, ActScrollResponse, ActTypeParams,
        ActTypeResponse, M2ServiceConfig, ReleaseAllParams, ReleaseAllResponse, SharedM2State,
        act_aim_with_handle, act_click_with_handle, act_clipboard, act_drag_with_handle,
        act_pad_with_handle, act_press_with_handle, act_scroll_with_handle, act_type_with_handle,
        release_all_with_handles, shared_m2_state_from_config_with_shutdown_reason,
        shared_m2_state_from_env,
    },
    m3::{
        M3ServiceConfig, SharedM3State,
        audio::{
            AudioTailParams, AudioTailResponse, AudioTranscribeParams, AudioTranscribeResponse,
            tail_audio, transcribe_audio,
        },
        permissions::{RequiredPermissions, authorization_error},
        profile::{
            ProfileActivateParams, ProfileActivateResponse, ProfileListParams, ProfileListResponse,
            activate_profile, list_profiles,
        },
        profile_quality::{
            ProfileQualityRefreshParams, ProfileQualityRefreshResponse, refresh_profile_quality,
        },
        reflex::{
            ReflexCancelParams, ReflexCancelResponse, ReflexHistoryParams, ReflexHistoryResponse,
            ReflexListParams, ReflexListResponse, ReflexRegisterParams, ReflexRegisterResponse,
            cancel_reflex, history_reflexes, list_reflexes, register_reflex,
        },
        replay::{ReplayRecordParams, ReplayRecordResponse, record_replay},
        shared_m3_state_from_config_with_shutdown_reason_and_sse_state, shared_m3_state_from_env,
        storage::{
            StorageGcOnceParams, StorageGcOnceResponse, StorageInspectParams,
            StorageInspectResponse, StoragePressureSampleParams, StoragePressureSampleResponse,
            StoragePutProbeRowsParams, StoragePutProbeRowsResponse, apply_storage_pressure_sample,
            inspect_storage, put_probe_rows, run_storage_gc_once,
        },
        subscribe::{
            SubscribeCancelParams, SubscribeCancelResponse, SubscribeParams, SubscribeResponse,
            cancel_subscription, subscribe_to_events,
        },
    },
    m4::{
        ActComboParams, ActComboResponse, ActLaunchParams, ActLaunchResponse, ActRunShellParams,
        ActRunShellResponse, M4ServiceConfig, execute_combo, launch, required_combo_permissions,
        run_shell,
    },
};

mod action_audit;
mod audit_context;
mod context;
mod handler;
mod health;
mod m1_tools;
mod m2_tools;
mod m3_tools;
mod m4_tools;
mod target_policy;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct SynapseService {
    started_at: Instant,
    tool_router: ToolRouter<Self>,
    m1_state: SharedM1State,
    m2_state: SharedM2State,
    m3_state: SharedM3State,
    m4_config: M4ServiceConfig,
}

impl SynapseService {
    #[must_use]
    pub fn new() -> Self {
        match Self::try_new() {
            Ok(service) => service,
            Err(error) => panic!("M3 state should initialize from environment: {error:#}"),
        }
    }

    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_env()?,
            m3_state: shared_m3_state_from_env()?,
            m4_config: M4ServiceConfig::from_env()?,
        })
    }

    pub fn try_with_m2_shutdown_reason_and_m3_config(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: CancellationToken,
        m2_config: &M2ServiceConfig,
        m3_config: M3ServiceConfig,
        m4_config: M4ServiceConfig,
    ) -> anyhow::Result<Self> {
        let sse_state = SseState::with_max_subscriptions(m3_config.max_subscriptions);
        Ok(Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_config_with_shutdown_reason(
                m2_config,
                shutdown_cancel.clone(),
                shutdown_reason,
                Some(connection_closed_cancel.clone()),
            )?,
            m3_state: shared_m3_state_from_config_with_shutdown_reason_and_sse_state(
                m3_config,
                shutdown_cancel,
                shutdown_reason,
                Some(connection_closed_cancel),
                sse_state,
            )?,
            m4_config,
        })
    }

    pub fn try_with_m2_shutdown_reason_and_sse_state_and_m3_config(
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: CancellationToken,
        sse_state: SseState,
        m2_config: &M2ServiceConfig,
        m3_config: M3ServiceConfig,
        m4_config: M4ServiceConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            started_at: Instant::now(),
            tool_router: Self::tool_router(),
            m1_state: SharedM1State::default(),
            m2_state: shared_m2_state_from_config_with_shutdown_reason(
                m2_config,
                shutdown_cancel.clone(),
                shutdown_reason,
                Some(connection_closed_cancel.clone()),
            )?,
            m3_state: shared_m3_state_from_config_with_shutdown_reason_and_sse_state(
                m3_config,
                shutdown_cancel,
                shutdown_reason,
                Some(connection_closed_cancel),
                sse_state,
            )?,
            m4_config,
        })
    }

    pub fn m2_emitter_done_receiver(&self) -> Option<watch::Receiver<Option<ActionStateSnapshot>>> {
        self.m2_state
            .lock()
            .ok()
            .and_then(|state| state.emitter_done_receiver())
    }

    pub(crate) fn m3_state_handle(&self) -> SharedM3State {
        Arc::clone(&self.m3_state)
    }

    fn tool_router() -> ToolRouter<Self> {
        Self::m1_tool_router()
            + Self::m2_tool_router()
            + Self::m3_tool_router()
            + Self::m4_tool_router()
    }
}

impl Default for SynapseService {
    fn default() -> Self {
        Self::new()
    }
}
