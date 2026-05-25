use std::sync::{Arc, Mutex};

use rmcp::ErrorData;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use synapse_core::{
    Action, Backend, EventFilter, ReflexLifetime, ReflexStatus, ReflexThen, error_codes,
    new_reflex_id,
};
use synapse_reflex::{ReflexError, ReflexRuntime, ScheduledReflex};

use crate::m1::mcp_error;

use super::M3ToolStub;

fn reflex_kind_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "enum": ["aim_track", "hold_move", "hold_button", "combo", "on_event"]
    })
}

const fn default_reflex_priority() -> u32 {
    synapse_reflex::DEFAULT_REFLEX_PRIORITY
}

const fn default_lifetime() -> ReflexLifetime {
    ReflexLifetime::UntilCancelled
}

const fn default_backend() -> Backend {
    Backend::Auto
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexRegisterParams {
    #[schemars(schema_with = "reflex_kind_schema")]
    pub kind: String,
    #[serde(default)]
    pub when: Option<EventFilter>,
    pub then: ReflexThen,
    #[serde(default = "default_reflex_priority")]
    #[schemars(default = "default_reflex_priority", range(min = 0, max = 1000))]
    pub priority: u32,
    #[serde(default = "default_lifetime")]
    #[schemars(default = "default_lifetime")]
    pub lifetime: ReflexLifetime,
    #[serde(default = "default_backend")]
    #[schemars(default = "default_backend")]
    pub backend: Backend,
    #[serde(default)]
    pub exclusive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexRegisterResponse {
    pub reflex_id: String,
    pub state: ReflexStatus,
}

#[must_use]
pub const fn reflex_register() -> M3ToolStub {
    M3ToolStub::new("reflex_register")
}

#[must_use]
pub const fn reflex_cancel() -> M3ToolStub {
    M3ToolStub::new("reflex_cancel")
}

#[must_use]
pub const fn reflex_list() -> M3ToolStub {
    M3ToolStub::new("reflex_list")
}

#[must_use]
pub const fn reflex_history() -> M3ToolStub {
    M3ToolStub::new("reflex_history")
}

pub fn register_reflex(
    runtime: &Arc<Mutex<ReflexRuntime>>,
    params: ReflexRegisterParams,
) -> Result<ReflexRegisterResponse, ErrorData> {
    let reflex = scheduled_reflex_from_params(params)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    let mut runtime = runtime.lock().map_err(|_err| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "reflex runtime lock poisoned",
        )
    })?;
    let state = runtime
        .register(&reflex)
        .map_err(|error| mcp_error(error.code(), error.to_string()))?;
    drop(runtime);
    Ok(ReflexRegisterResponse {
        reflex_id: state.id.clone(),
        state,
    })
}

fn scheduled_reflex_from_params(
    params: ReflexRegisterParams,
) -> Result<ScheduledReflex, ReflexError> {
    let reflex_id = new_reflex_id();
    let actions = actions_from_then(params.then, params.backend);
    match params.kind.as_str() {
        "on_event" => {
            let when = params.when.ok_or_else(|| ReflexError::ParamsInvalid {
                detail: "on_event reflex requires when filter".to_owned(),
            })?;
            Ok(ScheduledReflex::on_event(reflex_id, when, actions)
                .with_priority(params.priority)
                .with_lifetime(params.lifetime)
                .with_exclusive(params.exclusive))
        }
        "aim_track" | "hold_move" | "hold_button" | "combo" => Err(ReflexError::ParamsInvalid {
            detail: format!(
                "reflex kind {} is declared but not yet supported by the scheduler MCP adapter",
                params.kind
            ),
        }),
        other => Err(ReflexError::KindInvalid {
            detail: format!("unknown reflex kind: {other}"),
        }),
    }
}

fn actions_from_then(then: ReflexThen, backend: Backend) -> Vec<Action> {
    let mut actions = match then {
        ReflexThen::Action { action } => vec![action],
        ReflexThen::Actions { actions } => actions,
        ReflexThen::Combo {
            steps,
            backend: combo_backend,
        } => vec![Action::Combo {
            steps,
            backend: combo_backend,
        }],
    };
    for action in &mut actions {
        apply_backend_default(action, backend);
    }
    actions
}

fn apply_backend_default(action: &mut Action, fallback: Backend) {
    if fallback == Backend::Auto {
        return;
    }
    match action {
        Action::KeyPress { backend, .. }
        | Action::KeyDown { backend, .. }
        | Action::KeyUp { backend, .. }
        | Action::KeyChord { backend, .. }
        | Action::TypeText { backend, .. }
        | Action::MouseMove { backend, .. }
        | Action::MouseMoveRelative { backend, .. }
        | Action::MouseButton { backend, .. }
        | Action::MouseDrag { backend, .. }
        | Action::MouseScroll { backend, .. }
        | Action::AimAt { backend, .. }
        | Action::Combo { backend, .. }
            if *backend == Backend::Auto =>
        {
            *backend = fallback;
        }
        Action::KeyPress { .. }
        | Action::KeyDown { .. }
        | Action::KeyUp { .. }
        | Action::KeyChord { .. }
        | Action::TypeText { .. }
        | Action::MouseMove { .. }
        | Action::MouseMoveRelative { .. }
        | Action::MouseButton { .. }
        | Action::MouseDrag { .. }
        | Action::MouseScroll { .. }
        | Action::AimAt { .. }
        | Action::Combo { .. }
        | Action::PadButton { .. }
        | Action::PadStick { .. }
        | Action::PadTrigger { .. }
        | Action::PadReport { .. }
        | Action::ReleaseAll => {}
    }
}
