use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, MutexGuard},
};

use synapse_core::{
    Action, AimCurve, AimStyle, AimTarget, GamepadReport, Key, KeyCode, MouseButton, MouseTarget,
    PadButton, PadId, Point, Stick, Trigger,
};

use crate::{ActionBackend, ActionError, EmitState};

mod state;

use state::RecordingState;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordedInput {
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
    DelayMs {
        ms: u32,
    },
    UnicodeUnitDown {
        unit: u16,
    },
    UnicodeUnitUp {
        unit: u16,
    },
    MouseMove {
        to: MouseTarget,
        curve: AimCurve,
        duration_ms: u32,
    },
    MouseMoveAbsolute {
        point: Point,
    },
    MouseMoveRelative {
        dx: f64,
        dy: f64,
    },
    MouseButtonDown {
        button: MouseButton,
    },
    MouseButtonUp {
        button: MouseButton,
    },
    MouseStrokePoint {
        elapsed_ms: f64,
        point: Point,
    },
    MouseScroll {
        dy: i32,
        dx: i32,
        at: Option<Point>,
    },
    AimAt {
        target: AimTarget,
        style: AimStyle,
        deadline_ms: u32,
    },
    ComboAt {
        at_ms: u32,
    },
    PadButtonDown {
        pad: PadId,
        button: PadButton,
    },
    PadButtonUp {
        pad: PadId,
        button: PadButton,
    },
    PadStick {
        pad: PadId,
        stick: Stick,
        x: f32,
        y: f32,
    },
    PadTrigger {
        pad: PadId,
        trigger: Trigger,
        value: f32,
    },
    PadReport {
        pad: PadId,
        report: GamepadReport,
    },
    ReleaseAll {
        held_keys: Vec<KeyCode>,
        held_buttons: Vec<MouseButton>,
        pads: Vec<PadId>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct RecordingBackend {
    inner: Arc<Mutex<RecordingState>>,
}

impl RecordingBackend {
    #[must_use]
    #[tracing::instrument(fields(backend = "recording"))]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn events(&self) -> Vec<RecordedInput> {
        self.state().events.clone()
    }

    #[must_use]
    pub fn event_count(&self) -> usize {
        self.state().events.len()
    }

    #[must_use]
    pub fn events_since(&self, event_count: usize) -> Vec<RecordedInput> {
        self.state()
            .events
            .get(event_count..)
            .map_or_else(Vec::new, <[RecordedInput]>::to_vec)
    }

    #[must_use]
    pub fn held_keys(&self) -> BTreeSet<KeyCode> {
        self.state().held_keys.clone()
    }

    #[must_use]
    pub fn held_buttons(&self) -> BTreeSet<MouseButton> {
        self.state().held_buttons.clone()
    }

    #[must_use]
    pub fn pad_state(&self) -> HashMap<PadId, GamepadReport> {
        self.state().pad_state.clone()
    }

    fn state(&self) -> MutexGuard<'_, RecordingState> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl ActionBackend for RecordingBackend {
    #[tracing::instrument(skip_all, fields(backend = "recording"))]
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        self.state().apply_action(action, state)
    }
}
