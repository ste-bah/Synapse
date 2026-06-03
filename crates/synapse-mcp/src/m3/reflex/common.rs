use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;
use synapse_core::{
    Action, Backend, ButtonAction, ComboInput, ComboStep, DataPredicate, EventFilter,
    ReflexButtonTarget, ReflexLifetime, ReflexThen,
};
use synapse_reflex::ReflexError;

use crate::{
    m2::{ActPressParams, ActTypeParams, action_from_press_params, action_from_type_params},
    m3::a11y_events,
};

pub(super) fn reflex_kind_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "enum": ["aim_track", "hold_move", "hold_button", "combo", "on_event", "path_follow"]
    })
}

pub(super) const fn default_reflex_priority() -> u32 {
    synapse_reflex::DEFAULT_REFLEX_PRIORITY
}

pub(super) const fn default_lifetime() -> ReflexLifetime {
    ReflexLifetime::UntilCancelled
}

pub(super) const fn default_backend() -> Backend {
    Backend::Auto
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ReflexWhenParam {
    Filter(EventFilter),
    WindowEvent(WindowEventWhen),
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowEventWhen {
    pub kind: String,
    #[serde(default, rename = "match")]
    pub match_clause: WindowEventMatch,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowEventMatch {
    #[serde(default)]
    pub window_title_regex: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ReflexThenParam {
    Core(ReflexThen),
    Steps { steps: Vec<ReflexThenStep> },
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexThenStep {
    pub action: String,
    #[serde(default = "empty_params")]
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ReflexComboStepParam {
    Core(ComboStep),
    Tool(ReflexTimedThenStep),
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflexTimedThenStep {
    #[serde(default)]
    pub at_ms: u32,
    pub action: String,
    #[serde(default = "empty_params")]
    pub params: serde_json::Value,
}

pub(super) fn required_then(
    then: Option<ReflexThenParam>,
    kind: &'static str,
) -> Result<ReflexThenParam, ReflexError> {
    then.ok_or_else(|| ReflexError::ParamsInvalid {
        detail: format!("{kind} reflex requires then"),
    })
}

pub(super) fn actions_from_then(
    then: ReflexThenParam,
    backend: Backend,
) -> Result<Vec<Action>, ReflexError> {
    let mut actions = match then {
        ReflexThenParam::Core(ReflexThen::Action { action }) => vec![action],
        ReflexThenParam::Core(ReflexThen::Actions { actions }) => actions,
        ReflexThenParam::Core(ReflexThen::Combo {
            steps,
            backend: combo_backend,
        }) => vec![Action::Combo {
            steps,
            backend: combo_backend,
        }],
        ReflexThenParam::Steps { steps } => actions_from_demo_steps(steps)?,
    };
    for action in &mut actions {
        apply_backend_default(action, backend);
    }
    Ok(actions)
}

pub(super) const fn button_down_action(button: &ReflexButtonTarget, backend: Backend) -> Action {
    match *button {
        ReflexButtonTarget::Mouse { button } => Action::MouseButton {
            button,
            action: ButtonAction::Down,
            hold_ms: 0,
            backend,
        },
        ReflexButtonTarget::Pad { pad, button } => Action::PadButton {
            pad,
            button,
            action: ButtonAction::Down,
            hold_ms: 0,
        },
    }
}

pub(super) fn combo_steps_from_params(
    steps: Option<Vec<ReflexComboStepParam>>,
    then: Option<ReflexThenParam>,
) -> Result<Vec<ComboStep>, ReflexError> {
    if let Some(steps) = steps {
        if steps.is_empty() {
            return Err(ReflexError::ParamsInvalid {
                detail: "combo steps must contain at least one step".to_owned(),
            });
        }
        return steps
            .into_iter()
            .enumerate()
            .map(|(index, step)| combo_step_from_param(index, step))
            .collect();
    }

    match then {
        Some(ReflexThenParam::Core(ReflexThen::Combo { steps, .. })) if !steps.is_empty() => {
            Ok(steps)
        }
        Some(ReflexThenParam::Core(ReflexThen::Combo { .. })) => Err(ReflexError::ParamsInvalid {
            detail: "combo steps must contain at least one step".to_owned(),
        }),
        Some(ReflexThenParam::Steps { steps }) => steps
            .into_iter()
            .enumerate()
            .map(|(index, step)| timed_demo_step_to_combo_step(index, 0, step))
            .collect(),
        Some(ReflexThenParam::Core(_)) | None => Err(ReflexError::ParamsInvalid {
            detail: "combo reflex requires steps or then.kind=combo".to_owned(),
        }),
    }
}

impl ReflexWhenParam {
    pub(super) fn requires_a11y_event_bridge(&self) -> bool {
        match self {
            Self::Filter(filter) => a11y_events::event_filter_requires_a11y_bridge(filter),
            Self::WindowEvent(_) => true,
        }
    }

    pub(super) fn into_event_filter(self) -> Result<EventFilter, ReflexError> {
        match self {
            Self::Filter(filter) => Ok(filter),
            Self::WindowEvent(when) => when.into_event_filter(),
        }
    }
}

impl WindowEventWhen {
    fn into_event_filter(self) -> Result<EventFilter, ReflexError> {
        let kind = normalize_window_event_kind(&self.kind)?;
        let mut filters = vec![EventFilter::Kind { kind }];
        if let Some(pattern) = self.match_clause.window_title_regex {
            validate_regex(&pattern)?;
            filters.push(EventFilter::Data {
                path: "/window_title".to_owned(),
                predicate: DataPredicate::Regex { pattern },
            });
        }
        if filters.len() == 1 {
            Ok(filters.remove(0))
        } else {
            Ok(EventFilter::And { args: filters })
        }
    }
}

fn combo_step_from_param(
    index: usize,
    step: ReflexComboStepParam,
) -> Result<ComboStep, ReflexError> {
    match step {
        ReflexComboStepParam::Core(step) => Ok(step),
        ReflexComboStepParam::Tool(step) => {
            let at_ms = step.at_ms;
            let demo_step = ReflexThenStep {
                action: step.action,
                params: step.params,
            };
            timed_demo_step_to_combo_step(index, at_ms, demo_step)
        }
    }
}

fn timed_demo_step_to_combo_step(
    index: usize,
    at_ms: u32,
    step: ReflexThenStep,
) -> Result<ComboStep, ReflexError> {
    let action = action_from_demo_step(index, step)?;
    match action {
        Action::KeyPress { key, hold_ms, .. } => {
            let hold_ms = u16::try_from(hold_ms).map_err(|_err| ReflexError::ParamsInvalid {
                detail: format!("combo steps[{index}] hold_ms exceeds u16::MAX"),
            })?;
            Ok(ComboStep {
                at_ms,
                input: ComboInput::KeyPress { key, hold_ms },
            })
        }
        Action::MouseButton { button, action, .. } => Ok(ComboStep {
            at_ms,
            input: ComboInput::MouseButton { button, action },
        }),
        Action::MouseMoveRelative { dx, dy, .. } => Ok(ComboStep {
            at_ms,
            input: ComboInput::MouseMoveRel { dx, dy },
        }),
        other => Err(ReflexError::ParamsInvalid {
            detail: format!(
                "combo steps[{index}] action {other:?} cannot be used as one timed combo input"
            ),
        }),
    }
}

fn normalize_window_event_kind(raw: &str) -> Result<String, ReflexError> {
    let kind = raw.trim().replace('_', "-").to_ascii_lowercase();
    if kind.is_empty() {
        return Err(ReflexError::ParamsInvalid {
            detail: "window event kind must not be empty".to_owned(),
        });
    }
    Ok(kind)
}

fn validate_regex(pattern: &str) -> Result<(), ReflexError> {
    if pattern.trim().is_empty() {
        return Err(ReflexError::ParamsInvalid {
            detail: "window_title_regex must not be empty".to_owned(),
        });
    }
    regex::Regex::new(pattern).map_err(|error| ReflexError::ParamsInvalid {
        detail: format!("window_title_regex is invalid: {error}"),
    })?;
    Ok(())
}

fn actions_from_demo_steps(steps: Vec<ReflexThenStep>) -> Result<Vec<Action>, ReflexError> {
    if steps.is_empty() {
        return Err(ReflexError::ParamsInvalid {
            detail: "then.steps must contain at least one action".to_owned(),
        });
    }
    steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| action_from_demo_step(index, step))
        .collect()
}

fn action_from_demo_step(index: usize, step: ReflexThenStep) -> Result<Action, ReflexError> {
    match step.action.trim() {
        "act_type" => {
            let params = serde_json::from_value::<ActTypeParams>(step.params).map_err(|error| {
                ReflexError::ParamsInvalid {
                    detail: format!("then.steps[{index}].act_type params invalid: {error}"),
                }
            })?;
            action_from_type_params(&params).map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("then.steps[{index}].act_type params invalid: {error}"),
            })
        }
        "act_press" => {
            let params =
                serde_json::from_value::<ActPressParams>(step.params).map_err(|error| {
                    ReflexError::ParamsInvalid {
                        detail: format!("then.steps[{index}].act_press params invalid: {error}"),
                    }
                })?;
            action_from_press_params(&params).map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("then.steps[{index}].act_press params invalid: {error}"),
            })
        }
        other => Err(ReflexError::ParamsInvalid {
            detail: format!(
                "then.steps[{index}].action {other:?} is unsupported; supported actions: act_type, act_press"
            ),
        }),
    }
}

fn empty_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
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
        | Action::MouseStroke { backend, .. }
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
        | Action::MouseStroke { .. }
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
