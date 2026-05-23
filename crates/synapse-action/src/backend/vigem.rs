#[cfg(windows)]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(not(windows))]
use std::sync::Mutex;

use synapse_core::Action;

#[cfg(any(windows, test))]
use synapse_core::{
    ButtonAction, GamepadController, GamepadReport, PadButton, PadId, Stick, Trigger,
};

use crate::{ActionBackend, ActionError, EmitState};

/// Driver-backed `ViGEm` gamepad backend.
///
/// On Windows this lazily connects to `ViGEmBus` and plugs an X360 or DS4
/// target the first time a pad id is referenced. Other platforms fail closed
/// instead of pretending a virtual controller exists.
#[derive(Debug, Default)]
pub struct VigemBackend {
    inner: Mutex<VigemBackendInner>,
}

impl VigemBackend {
    #[must_use]
    #[tracing::instrument(fields(backend = "vigem"))]
    pub fn new() -> Self {
        Self::default()
    }

    /// Probes whether the backing `ViGEm` driver can be reached.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_VIGEM_NOT_INSTALLED` on Windows when the `ViGEmBus`
    /// device interface is absent. Returns `ACTION_BACKEND_UNAVAILABLE` on
    /// non-Windows targets.
    #[tracing::instrument(skip_all, fields(backend = "vigem"))]
    pub fn ensure_ready(&self) -> Result<(), ActionError> {
        let mut inner = self.lock_inner()?;
        inner.ensure_ready()
    }

    fn lock_inner(&self) -> Result<std::sync::MutexGuard<'_, VigemBackendInner>, ActionError> {
        self.inner
            .lock()
            .map_err(|_err| ActionError::VigemPluginFailed {
                detail: "backend=vigem reason=backend mutex poisoned".to_owned(),
            })
    }
}

impl ActionBackend for VigemBackend {
    #[tracing::instrument(skip_all, fields(backend = "vigem"))]
    fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        crate::validate_action(action)?;
        let mut inner = self.lock_inner()?;
        inner.execute(action, state)
    }
}

#[cfg(windows)]
impl Drop for VigemBackend {
    fn drop(&mut self) {
        if let Ok(inner) = self.inner.get_mut() {
            inner.neutral_all_for_drop();
        }
    }
}

#[cfg(windows)]
#[derive(Debug, Default)]
struct VigemBackendInner {
    client: Option<Arc<vigem_client::Client>>,
    pads: HashMap<PadId, VigemPad>,
}

#[cfg(windows)]
impl VigemBackendInner {
    fn ensure_ready(&mut self) -> Result<(), ActionError> {
        self.ensure_client().map(|_client| ())
    }

    fn execute(&mut self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        match action {
            Action::PadButton {
                pad,
                button,
                action: btn_action,
                hold_ms,
            } => self.pad_button(state, *pad, *button, *btn_action, *hold_ms),
            Action::PadStick { pad, stick, x, y } => self.pad_stick(state, *pad, *stick, *x, *y),
            Action::PadTrigger {
                pad,
                trigger,
                value,
            } => self.pad_trigger(state, *pad, *trigger, *value),
            Action::PadReport { pad, report } => self.pad_report(state, *pad, report.clone()),
            Action::ReleaseAll => self.release_all(state),
            _ => Err(routed_non_gamepad_error(action)),
        }
    }

    fn ensure_client(&mut self) -> Result<Arc<vigem_client::Client>, ActionError> {
        if let Some(client) = &self.client {
            return Ok(Arc::clone(client));
        }

        let client = Arc::new(
            vigem_client::Client::connect()
                .map_err(|err| map_vigem_error("connect_vigembus", err))?,
        );
        self.client = Some(Arc::clone(&client));
        Ok(client)
    }

    fn ensure_pad(
        &mut self,
        pad: PadId,
        controller: GamepadController,
    ) -> Result<&mut VigemPad, ActionError> {
        if self
            .pads
            .get(&pad)
            .is_some_and(|target| target.controller() != controller)
            && let Some(mut old_target) = self.pads.remove(&pad)
        {
            old_target.neutralize()?;
        }

        if !self.pads.contains_key(&pad) {
            let client = self.ensure_client()?;
            let target = match controller {
                GamepadController::X360 => Self::plug_x360(client)?,
                GamepadController::Ds4 => Self::plug_ds4(client)?,
            };
            self.pads.insert(pad, target);
        }

        self.pads
            .get_mut(&pad)
            .ok_or_else(|| ActionError::VigemPluginFailed {
                detail: format!("backend=vigem reason=pad target missing after plug-in pad={pad}"),
            })
    }

    fn plug_x360(client: Arc<vigem_client::Client>) -> Result<VigemPad, ActionError> {
        let mut target =
            vigem_client::Xbox360Wired::new(client, vigem_client::TargetId::XBOX360_WIRED);
        target
            .plugin()
            .map_err(|err| map_vigem_error("plugin_x360_target", err))?;
        target
            .wait_ready()
            .map_err(|err| map_vigem_error("wait_ready_x360_target", err))?;
        let neutral = neutral_gamepad_report(GamepadController::X360);
        target
            .update(&xgamepad_from_snapshot(x360_report_snapshot(&neutral)))
            .map_err(|err| map_vigem_error("initial_neutral_x360_report", err))?;
        Ok(VigemPad::new_x360(target, neutral))
    }

    fn plug_ds4(client: Arc<vigem_client::Client>) -> Result<VigemPad, ActionError> {
        let mut target =
            vigem_client::DualShock4Wired::new(client, vigem_client::TargetId::DUALSHOCK4_WIRED);
        target
            .plugin()
            .map_err(|err| map_vigem_error("plugin_ds4_target", err))?;
        target
            .wait_ready()
            .map_err(|err| map_vigem_error("wait_ready_ds4_target", err))?;
        let neutral = neutral_gamepad_report(GamepadController::Ds4);
        target
            .update(&ds4_report_snapshot(&neutral).into_vigem_report())
            .map_err(|err| map_vigem_error("initial_neutral_ds4_report", err))?;
        Ok(VigemPad::new_ds4(target, neutral))
    }

    fn pad_button(
        &mut self,
        state: &mut EmitState,
        pad: PadId,
        button: PadButton,
        action: ButtonAction,
        hold_ms: u32,
    ) -> Result<(), ActionError> {
        match action {
            ButtonAction::Down => {
                let mut report = report_for_pad(state, pad);
                push_unique(&mut report.buttons, button);
                self.send_report(pad, &report)?;
                apply_pad_button(state, pad, button, ButtonAction::Down);
                Ok(())
            }
            ButtonAction::Up => {
                let mut report = report_for_pad(state, pad);
                report.buttons.retain(|held| *held != button);
                self.send_report(pad, &report)?;
                apply_pad_button(state, pad, button, ButtonAction::Up);
                Ok(())
            }
            ButtonAction::Press => {
                let mut report = report_for_pad(state, pad);
                push_unique(&mut report.buttons, button);
                self.send_report(pad, &report)?;
                apply_pad_button(state, pad, button, ButtonAction::Down);
                if hold_ms > 0 {
                    std::thread::sleep(Duration::from_millis(u64::from(hold_ms)));
                }
                report.buttons.retain(|held| *held != button);
                self.send_report(pad, &report)?;
                apply_pad_button(state, pad, button, ButtonAction::Up);
                Ok(())
            }
        }
    }

    fn pad_stick(
        &mut self,
        state: &mut EmitState,
        pad: PadId,
        stick: Stick,
        x: f32,
        y: f32,
    ) -> Result<(), ActionError> {
        let mut report = report_for_pad(state, pad);
        match stick {
            Stick::Left => report.thumb_l = (x, y),
            Stick::Right => report.thumb_r = (x, y),
        }
        self.send_report(pad, &report)?;
        apply_pad_stick(state, pad, stick, x, y);
        Ok(())
    }

    fn pad_trigger(
        &mut self,
        state: &mut EmitState,
        pad: PadId,
        trigger: Trigger,
        value: f32,
    ) -> Result<(), ActionError> {
        let mut report = report_for_pad(state, pad);
        match trigger {
            Trigger::Left => report.lt = value,
            Trigger::Right => report.rt = value,
        }
        self.send_report(pad, &report)?;
        apply_pad_trigger(state, pad, trigger, value);
        Ok(())
    }

    fn pad_report(
        &mut self,
        state: &mut EmitState,
        pad: PadId,
        report: GamepadReport,
    ) -> Result<(), ActionError> {
        self.send_report(pad, &report)?;
        apply_pad_report(state, pad, report);
        Ok(())
    }

    fn send_report(&mut self, pad: PadId, report: &GamepadReport) -> Result<(), ActionError> {
        let target = self.ensure_pad(pad, report.controller)?;
        target.update(report)
    }

    fn release_all(&mut self, state: &mut EmitState) -> Result<(), ActionError> {
        for (pad, target) in &mut self.pads {
            target
                .neutralize()
                .map_err(|err| add_pad_context(*pad, err))?;
        }
        state.pad_state.clear();
        Ok(())
    }

    fn neutral_all_for_drop(&mut self) {
        for target in self.pads.values_mut() {
            let _neutral_result = target.neutralize();
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
enum VigemPad {
    X360(VigemX360Pad),
    Ds4(VigemDs4Pad),
}

#[cfg(windows)]
impl VigemPad {
    const fn new_x360(
        target: vigem_client::Xbox360Wired<Arc<vigem_client::Client>>,
        report: GamepadReport,
    ) -> Self {
        Self::X360(VigemX360Pad { target, report })
    }

    const fn new_ds4(
        target: vigem_client::DualShock4Wired<Arc<vigem_client::Client>>,
        report: GamepadReport,
    ) -> Self {
        Self::Ds4(VigemDs4Pad { target, report })
    }

    const fn controller(&self) -> GamepadController {
        match self {
            Self::X360(_pad) => GamepadController::X360,
            Self::Ds4(_pad) => GamepadController::Ds4,
        }
    }

    fn update(&mut self, report: &GamepadReport) -> Result<(), ActionError> {
        match self {
            Self::X360(pad) => pad.update(report),
            Self::Ds4(pad) => pad.update(report),
        }
    }

    fn neutralize(&mut self) -> Result<(), ActionError> {
        let neutral = neutral_gamepad_report(self.controller());
        self.update(&neutral)
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct VigemX360Pad {
    target: vigem_client::Xbox360Wired<Arc<vigem_client::Client>>,
    report: GamepadReport,
}

#[cfg(windows)]
impl VigemX360Pad {
    fn update(&mut self, report: &GamepadReport) -> Result<(), ActionError> {
        let gamepad = xgamepad_from_snapshot(x360_report_snapshot(report));
        self.target
            .update(&gamepad)
            .map_err(|err| map_vigem_error("update_x360_report", err))?;
        self.report = report.clone();
        Ok(())
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct VigemDs4Pad {
    target: vigem_client::DualShock4Wired<Arc<vigem_client::Client>>,
    report: GamepadReport,
}

#[cfg(windows)]
impl VigemDs4Pad {
    fn update(&mut self, report: &GamepadReport) -> Result<(), ActionError> {
        let ds4_report = ds4_report_snapshot(report).into_vigem_report();
        self.target
            .update(&ds4_report)
            .map_err(|err| map_vigem_error("update_ds4_report", err))?;
        self.report = report.clone();
        Ok(())
    }
}

#[cfg(windows)]
const fn xgamepad_from_snapshot(snapshot: X360ReportSnapshot) -> vigem_client::XGamepad {
    vigem_client::XGamepad {
        buttons: vigem_client::XButtons {
            raw: snapshot.buttons_raw,
        },
        left_trigger: snapshot.left_trigger,
        right_trigger: snapshot.right_trigger,
        thumb_lx: snapshot.thumb_lx,
        thumb_ly: snapshot.thumb_ly,
        thumb_rx: snapshot.thumb_rx,
        thumb_ry: snapshot.thumb_ry,
    }
}

#[cfg(windows)]
fn add_pad_context(pad: PadId, error: ActionError) -> ActionError {
    match error {
        ActionError::VigemNotInstalled { detail } => ActionError::VigemNotInstalled {
            detail: format!("pad={pad} {detail}"),
        },
        ActionError::VigemPluginFailed { detail } => ActionError::VigemPluginFailed {
            detail: format!("pad={pad} {detail}"),
        },
        other => other,
    }
}

#[cfg(windows)]
fn map_vigem_error(context: &'static str, error: vigem_client::Error) -> ActionError {
    match error {
        vigem_client::Error::BusNotFound => ActionError::VigemNotInstalled {
            detail: format!("backend=vigem context={context} driver=ViGEmBus error={error}"),
        },
        vigem_client::Error::BusAccessFailed(code) => ActionError::VigemPluginFailed {
            detail: format!(
                "backend=vigem context={context} driver=ViGEmBus access_failed_win32={code}"
            ),
        },
        vigem_client::Error::WinError(code) => ActionError::VigemPluginFailed {
            detail: format!("backend=vigem context={context} win32={code}"),
        },
        _ => ActionError::VigemPluginFailed {
            detail: format!("backend=vigem context={context} error={error}"),
        },
    }
}

#[cfg(not(windows))]
#[derive(Debug, Default)]
struct VigemBackendInner;

#[cfg(not(windows))]
impl VigemBackendInner {
    #[allow(clippy::needless_pass_by_ref_mut, clippy::unused_self)]
    fn ensure_ready(&mut self) -> Result<(), ActionError> {
        Err(ActionError::BackendUnavailable {
            detail: "vigem backend requires Windows and the ViGEmBus driver".to_owned(),
        })
    }

    #[allow(clippy::needless_pass_by_ref_mut, clippy::unused_self)]
    fn execute(&mut self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
        if matches!(action, Action::ReleaseAll) && state.pad_state.is_empty() {
            return Ok(());
        }
        Err(ActionError::BackendUnavailable {
            detail: format!(
                "vigem backend requires Windows and the ViGEmBus driver; action_kind={}",
                action_kind(action)
            ),
        })
    }
}

#[cfg(windows)]
fn report_for_pad(state: &EmitState, pad: PadId) -> GamepadReport {
    state
        .pad_state
        .get(&pad)
        .cloned()
        .unwrap_or_else(neutral_x360_gamepad_report)
}

#[cfg(any(windows, test))]
pub(crate) fn apply_pad_button(
    state: &mut EmitState,
    pad: PadId,
    button: PadButton,
    action: ButtonAction,
) {
    let should_remove = {
        let report = state
            .pad_state
            .entry(pad)
            .or_insert_with(neutral_x360_gamepad_report);
        match action {
            ButtonAction::Down => push_unique(&mut report.buttons, button),
            ButtonAction::Up | ButtonAction::Press => {
                report.buttons.retain(|held| *held != button);
            }
        }
        is_neutral_report(report)
    };

    if should_remove {
        state.pad_state.remove(&pad);
    }
}

#[cfg(any(windows, test))]
pub(crate) fn apply_pad_stick(state: &mut EmitState, pad: PadId, stick: Stick, x: f32, y: f32) {
    let should_remove = {
        let report = state
            .pad_state
            .entry(pad)
            .or_insert_with(neutral_x360_gamepad_report);
        match stick {
            Stick::Left => report.thumb_l = (x, y),
            Stick::Right => report.thumb_r = (x, y),
        }
        is_neutral_report(report)
    };

    if should_remove {
        state.pad_state.remove(&pad);
    }
}

#[cfg(any(windows, test))]
pub(crate) fn apply_pad_trigger(state: &mut EmitState, pad: PadId, trigger: Trigger, value: f32) {
    let should_remove = {
        let report = state
            .pad_state
            .entry(pad)
            .or_insert_with(neutral_x360_gamepad_report);
        match trigger {
            Trigger::Left => report.lt = value,
            Trigger::Right => report.rt = value,
        }
        is_neutral_report(report)
    };

    if should_remove {
        state.pad_state.remove(&pad);
    }
}

#[cfg(any(windows, test))]
pub(crate) fn apply_pad_report(state: &mut EmitState, pad: PadId, report: GamepadReport) {
    if is_neutral_report(&report) {
        state.pad_state.remove(&pad);
    } else {
        state.pad_state.insert(pad, report);
    }
}

#[cfg(any(windows, test))]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct X360ReportSnapshot {
    buttons_raw: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumb_lx: i16,
    thumb_ly: i16,
    thumb_rx: i16,
    thumb_ry: i16,
}

#[cfg(any(windows, test))]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Ds4ReportSnapshot {
    buttons: u16,
    special: u8,
    trigger_l: u8,
    trigger_r: u8,
    thumb_lx: u8,
    thumb_ly: u8,
    thumb_rx: u8,
    thumb_ry: u8,
}

#[cfg(any(windows, test))]
fn x360_report_snapshot(report: &GamepadReport) -> X360ReportSnapshot {
    X360ReportSnapshot {
        buttons_raw: x360_buttons_raw(&report.buttons),
        left_trigger: normalized_trigger_to_u8(report.lt),
        right_trigger: normalized_trigger_to_u8(report.rt),
        thumb_lx: normalized_axis_to_i16(report.thumb_l.0),
        thumb_ly: normalized_axis_to_i16(report.thumb_l.1),
        thumb_rx: normalized_axis_to_i16(report.thumb_r.0),
        thumb_ry: normalized_axis_to_i16(report.thumb_r.1),
    }
}

#[cfg(any(windows, test))]
fn ds4_report_snapshot(report: &GamepadReport) -> Ds4ReportSnapshot {
    let trigger_l = normalized_trigger_to_u8(report.lt);
    let trigger_r = normalized_trigger_to_u8(report.rt);
    Ds4ReportSnapshot {
        buttons: ds4_buttons_raw(&report.buttons, trigger_l, trigger_r),
        special: ds4_special_raw(&report.buttons),
        trigger_l,
        trigger_r,
        thumb_lx: normalized_axis_to_ds4_x(report.thumb_l.0),
        thumb_ly: normalized_axis_to_ds4_y(report.thumb_l.1),
        thumb_rx: normalized_axis_to_ds4_x(report.thumb_r.0),
        thumb_ry: normalized_axis_to_ds4_y(report.thumb_r.1),
    }
}

#[cfg(windows)]
impl Ds4ReportSnapshot {
    const fn into_vigem_report(self) -> vigem_client::DS4Report {
        vigem_client::DS4Report {
            thumb_lx: self.thumb_lx,
            thumb_ly: self.thumb_ly,
            thumb_rx: self.thumb_rx,
            thumb_ry: self.thumb_ry,
            buttons: self.buttons,
            special: self.special,
            trigger_l: self.trigger_l,
            trigger_r: self.trigger_r,
        }
    }
}

#[cfg(any(windows, test))]
fn x360_buttons_raw(buttons: &[PadButton]) -> u16 {
    buttons.iter().fold(0, |raw, button| {
        raw | match button {
            PadButton::Up => 0x0001,
            PadButton::Down => 0x0002,
            PadButton::Left => 0x0004,
            PadButton::Right => 0x0008,
            PadButton::Start => 0x0010,
            PadButton::Back => 0x0020,
            PadButton::Ls => 0x0040,
            PadButton::Rs => 0x0080,
            PadButton::Lb => 0x0100,
            PadButton::Rb => 0x0200,
            PadButton::Guide => 0x0400,
            PadButton::A => 0x1000,
            PadButton::B => 0x2000,
            PadButton::X => 0x4000,
            PadButton::Y => 0x8000,
        }
    })
}

#[cfg(any(windows, test))]
fn ds4_buttons_raw(buttons: &[PadButton], trigger_l: u8, trigger_r: u8) -> u16 {
    let mut raw = ds4_dpad_raw(buttons);
    for button in buttons {
        raw |= match button {
            PadButton::A => 1 << 5,
            PadButton::B => 1 << 6,
            PadButton::X => 1 << 4,
            PadButton::Y => 1 << 7,
            PadButton::Lb => 1 << 8,
            PadButton::Rb => 1 << 9,
            PadButton::Ls => 1 << 14,
            PadButton::Rs => 1 << 15,
            PadButton::Back => 1 << 12,
            PadButton::Start => 1 << 13,
            PadButton::Up
            | PadButton::Down
            | PadButton::Left
            | PadButton::Right
            | PadButton::Guide => 0,
        };
    }
    if trigger_l > 0 {
        raw |= 1 << 10;
    }
    if trigger_r > 0 {
        raw |= 1 << 11;
    }
    raw
}

#[cfg(any(windows, test))]
fn ds4_dpad_raw(buttons: &[PadButton]) -> u16 {
    let up = buttons.contains(&PadButton::Up);
    let right = buttons.contains(&PadButton::Right);
    let down = buttons.contains(&PadButton::Down);
    let left = buttons.contains(&PadButton::Left);
    match (up, right, down, left) {
        (true, true, false, false) => 0x1,
        (false, true, true, false) => 0x3,
        (false, false, true, true) => 0x5,
        (true, false, false, true) => 0x7,
        (true, false, false, false) => 0x0,
        (false, true, false, false) => 0x2,
        (false, false, true, false) => 0x4,
        (false, false, false, true) => 0x6,
        _ => 0x8,
    }
}

#[cfg(any(windows, test))]
fn ds4_special_raw(buttons: &[PadButton]) -> u8 {
    u8::from(buttons.contains(&PadButton::Guide))
}

#[cfg(any(windows, test))]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn normalized_trigger_to_u8(value: f32) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8
}

#[cfg(any(windows, test))]
#[allow(clippy::cast_possible_truncation)]
fn normalized_axis_to_i16(value: f32) -> i16 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value.clamp(-1.0, 1.0);
    if clamped.is_sign_negative() {
        (clamped * 32_768.0).round() as i16
    } else {
        (clamped * f32::from(i16::MAX)).round() as i16
    }
}

#[cfg(any(windows, test))]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn normalized_axis_to_ds4_x(value: f32) -> u8 {
    if !value.is_finite() {
        return 0x80;
    }
    ((value.clamp(-1.0, 1.0) + 1.0) * 127.5).round() as u8
}

#[cfg(any(windows, test))]
fn normalized_axis_to_ds4_y(value: f32) -> u8 {
    normalized_axis_to_ds4_x(-value)
}

#[cfg(any(windows, test))]
const fn neutral_gamepad_report(controller: GamepadController) -> GamepadReport {
    GamepadReport::neutral(controller)
}

#[cfg(any(windows, test))]
const fn neutral_x360_gamepad_report() -> GamepadReport {
    neutral_gamepad_report(GamepadController::X360)
}

#[cfg(any(windows, test))]
fn is_neutral_report(report: &GamepadReport) -> bool {
    report.buttons.is_empty()
        && report.thumb_l == (0.0, 0.0)
        && report.thumb_r == (0.0, 0.0)
        && report.lt == 0.0
        && report.rt == 0.0
}

#[cfg(any(windows, test))]
fn push_unique(buttons: &mut Vec<PadButton>, button: PadButton) {
    if !buttons.contains(&button) {
        buttons.push(button);
    }
}

#[cfg(windows)]
fn routed_non_gamepad_error(action: &Action) -> ActionError {
    ActionError::BackendUnavailable {
        detail: format!(
            "backend=vigem reason=routed non-gamepad action through gamepad backend action_kind={}",
            action_kind(action)
        ),
    }
}

const fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::KeyPress { .. } => "key_press",
        Action::KeyDown { .. } => "key_down",
        Action::KeyUp { .. } => "key_up",
        Action::KeyChord { .. } => "key_chord",
        Action::TypeText { .. } => "type_text",
        Action::MouseMove { .. } => "mouse_move",
        Action::MouseMoveRelative { .. } => "mouse_move_relative",
        Action::MouseButton { .. } => "mouse_button",
        Action::MouseDrag { .. } => "mouse_drag",
        Action::MouseScroll { .. } => "mouse_scroll",
        Action::PadButton { .. } => "pad_button",
        Action::PadStick { .. } => "pad_stick",
        Action::PadTrigger { .. } => "pad_trigger",
        Action::PadReport { .. } => "pad_report",
        Action::AimAt { .. } => "aim_at",
        Action::Combo { .. } => "combo",
        Action::ReleaseAll => "release_all",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    use synapse_core::{AimCurve, Backend, MouseTarget, Point};

    #[test]
    fn x360_report_snapshot_maps_buttons_axes_and_triggers() {
        let report = GamepadReport {
            controller: GamepadController::X360,
            buttons: vec![
                PadButton::A,
                PadButton::Start,
                PadButton::Lb,
                PadButton::A,
                PadButton::Guide,
            ],
            thumb_l: (1.0, -1.0),
            thumb_r: (0.5, -0.5),
            lt: 1.0,
            rt: 0.5,
        };
        let before = "buttons=[a,start,lb,a,guide] thumb_l=(1,-1) thumb_r=(0.5,-0.5) lt=1 rt=0.5";
        let after = x360_report_snapshot(&report);
        println!("source_of_truth=vigem_x360_report edge=happy before={before} after={after:?}");
        assert_eq!(
            after,
            X360ReportSnapshot {
                buttons_raw: 0x1510,
                left_trigger: 255,
                right_trigger: 128,
                thumb_lx: 32_767,
                thumb_ly: -32_768,
                thumb_rx: 16_384,
                thumb_ry: -16_384,
            }
        );
    }

    #[test]
    fn x360_report_snapshot_clamps_invalid_numeric_edges() {
        let report = GamepadReport {
            controller: GamepadController::X360,
            buttons: vec![PadButton::Down, PadButton::Right],
            thumb_l: (1.5, -2.0),
            thumb_r: (f32::NAN, f32::INFINITY),
            lt: 2.0,
            rt: f32::NAN,
        };
        let before = "buttons=[down,right] thumb_l=(1.5,-2.0) thumb_r=(NaN,inf) lt=2.0 rt=NaN";
        let after = x360_report_snapshot(&report);
        println!("source_of_truth=vigem_x360_report edge=clamp before={before} after={after:?}");
        assert_eq!(
            after,
            X360ReportSnapshot {
                buttons_raw: 0x000a,
                left_trigger: 255,
                right_trigger: 0,
                thumb_lx: 32_767,
                thumb_ly: -32_768,
                thumb_rx: 0,
                thumb_ry: 0,
            }
        );
    }

    #[test]
    fn ds4_report_snapshot_maps_buttons_axes_triggers_and_specials() {
        let report = GamepadReport {
            controller: GamepadController::Ds4,
            buttons: vec![
                PadButton::A,
                PadButton::B,
                PadButton::X,
                PadButton::Y,
                PadButton::Lb,
                PadButton::Rb,
                PadButton::Ls,
                PadButton::Rs,
                PadButton::Back,
                PadButton::Start,
                PadButton::Guide,
                PadButton::Up,
                PadButton::Right,
            ],
            thumb_l: (1.0, -1.0),
            thumb_r: (0.0, 0.5),
            lt: 0.25,
            rt: 1.0,
        };
        let before = "controller=ds4 buttons=[a,b,x,y,lb,rb,ls,rs,back,start,guide,up,right] thumb_l=(1,-1) thumb_r=(0,0.5) lt=0.25 rt=1";
        let after = ds4_report_snapshot(&report);
        println!("source_of_truth=vigem_ds4_report edge=happy before={before} after={after:?}");
        assert_eq!(
            after,
            Ds4ReportSnapshot {
                buttons: 0xfff1,
                special: 0x01,
                trigger_l: 64,
                trigger_r: 255,
                thumb_lx: 255,
                thumb_ly: 255,
                thumb_rx: 128,
                thumb_ry: 64,
            }
        );
    }

    #[test]
    fn ds4_report_snapshot_clamps_invalid_numeric_and_dpad_edges() {
        let report = GamepadReport {
            controller: GamepadController::Ds4,
            buttons: vec![
                PadButton::Up,
                PadButton::Down,
                PadButton::Left,
                PadButton::Right,
            ],
            thumb_l: (f32::NAN, f32::INFINITY),
            thumb_r: (-2.0, 2.0),
            lt: f32::NAN,
            rt: 2.0,
        };
        let before = "controller=ds4 buttons=[up,down,left,right] thumb_l=(NaN,inf) thumb_r=(-2,2) lt=NaN rt=2";
        let after = ds4_report_snapshot(&report);
        println!("source_of_truth=vigem_ds4_report edge=clamp before={before} after={after:?}");
        assert_eq!(
            after,
            Ds4ReportSnapshot {
                buttons: 0x0808,
                special: 0,
                trigger_l: 0,
                trigger_r: 255,
                thumb_lx: 128,
                thumb_ly: 128,
                thumb_rx: 0,
                thumb_ry: 0,
            }
        );
    }

    #[test]
    fn pad_state_helpers_track_partial_updates_and_neutral_removal() {
        let mut state = EmitState::new();
        let before = state.snapshot();
        println!("source_of_truth=vigem_pad_state edge=partial before={before:?}");
        apply_pad_button(&mut state, 3, PadButton::B, ButtonAction::Down);
        apply_pad_stick(&mut state, 3, Stick::Left, 0.25, -0.75);
        apply_pad_trigger(&mut state, 3, Trigger::Right, 0.5);
        let after_down = state.snapshot();
        println!("source_of_truth=vigem_pad_state edge=partial after_down={after_down:?}");
        assert_eq!(after_down.pad_state[&3].buttons, vec![PadButton::B]);
        assert_eq!(after_down.pad_state[&3].thumb_l, (0.25, -0.75));
        assert!((after_down.pad_state[&3].rt - 0.5).abs() < f32::EPSILON);

        apply_pad_button(&mut state, 3, PadButton::B, ButtonAction::Up);
        apply_pad_stick(&mut state, 3, Stick::Left, 0.0, 0.0);
        apply_pad_trigger(&mut state, 3, Trigger::Right, 0.0);
        let after_neutral = state.snapshot();
        println!("source_of_truth=vigem_pad_state edge=partial after_neutral={after_neutral:?}");
        assert!(!after_neutral.pad_state.contains_key(&3));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_backend_fails_closed_without_state_mutation() {
        let backend = VigemBackend::new();
        let mut state = EmitState::new();
        let before = state.snapshot();
        println!("source_of_truth=vigem_non_windows edge=pad_report before={before:?}");
        let result = backend.execute(
            &Action::PadReport {
                pad: 1,
                report: GamepadReport {
                    controller: GamepadController::X360,
                    buttons: vec![PadButton::A],
                    thumb_l: (0.0, 0.0),
                    thumb_r: (0.0, 0.0),
                    lt: 0.0,
                    rt: 0.0,
                },
            },
            &mut state,
        );
        let after = state.snapshot();
        let error = result
            .err()
            .unwrap_or_else(|| panic!("non-Windows ViGEm pad report must fail closed"));
        println!(
            "source_of_truth=vigem_non_windows edge=pad_report after={after:?} after_code={}",
            error.code()
        );
        assert_eq!(
            error.code(),
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert_eq!(before, after);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_ensure_ready_and_non_pad_edges_fail_closed() {
        let backend = VigemBackend::new();
        let ensure_error = backend
            .ensure_ready()
            .err()
            .unwrap_or_else(|| panic!("non-Windows ensure_ready must fail closed"));
        println!(
            "source_of_truth=vigem_non_windows edge=ensure_ready before=platform:not_windows after_code={} after_detail={:?}",
            ensure_error.code(),
            ensure_error.detail()
        );
        assert_eq!(
            ensure_error.code(),
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );

        let mut state = EmitState::new();
        let before = state.snapshot();
        let non_pad = Action::MouseMove {
            to: MouseTarget::Screen {
                point: Point { x: 1, y: 2 },
            },
            curve: AimCurve::Instant,
            duration_ms: 0,
            backend: Backend::Vigem,
        };
        let result = backend.execute(&non_pad, &mut state);
        let after = state.snapshot();
        let error = result
            .err()
            .unwrap_or_else(|| panic!("non-Windows non-pad action must fail closed"));
        println!(
            "source_of_truth=vigem_non_windows edge=non_pad before={before:?} after={after:?} after_code={}",
            error.code()
        );
        assert_eq!(
            error.code(),
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert_eq!(before, after);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_empty_release_all_is_noop_but_non_empty_pad_state_fails() {
        let backend = VigemBackend::new();
        let mut empty = EmitState::new();
        let before_empty = empty.snapshot();
        let empty_result = backend.execute(&Action::ReleaseAll, &mut empty);
        let after_empty = empty.snapshot();
        println!(
            "source_of_truth=vigem_non_windows edge=empty_release before={before_empty:?} after={after_empty:?} result={empty_result:?}"
        );
        assert!(empty_result.is_ok());
        assert_eq!(before_empty, after_empty);

        let mut seeded = EmitState::new();
        apply_pad_report(
            &mut seeded,
            2,
            GamepadReport {
                controller: GamepadController::X360,
                buttons: vec![PadButton::Y],
                thumb_l: (0.0, 0.0),
                thumb_r: (0.0, 0.0),
                lt: 0.0,
                rt: 0.0,
            },
        );
        let before_seeded = seeded.snapshot();
        let seeded_result = backend.execute(&Action::ReleaseAll, &mut seeded);
        let after_seeded = seeded.snapshot();
        let error = seeded_result
            .err()
            .unwrap_or_else(|| panic!("non-empty non-Windows release_all must fail closed"));
        println!(
            "source_of_truth=vigem_non_windows edge=non_empty_release before={before_seeded:?} after={after_seeded:?} after_code={}",
            error.code()
        );
        assert_eq!(
            error.code(),
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert_eq!(before_seeded, after_seeded);
    }

    #[cfg(windows)]
    #[test]
    fn vigem_error_mapping_preserves_declared_codes() {
        let not_installed = map_vigem_error("connect_vigembus", vigem_client::Error::BusNotFound);
        println!(
            "source_of_truth=vigem_error_mapping edge=bus_missing after_code={} after_detail={:?}",
            not_installed.code(),
            not_installed.detail()
        );
        assert_eq!(
            not_installed.code(),
            synapse_core::error_codes::ACTION_VIGEM_NOT_INSTALLED
        );

        let plugin_failed = map_vigem_error("plugin_x360_target", vigem_client::Error::NoFreeSlot);
        println!(
            "source_of_truth=vigem_error_mapping edge=plugin_failed after_code={} after_detail={:?}",
            plugin_failed.code(),
            plugin_failed.detail()
        );
        assert_eq!(
            plugin_failed.code(),
            synapse_core::error_codes::ACTION_VIGEM_PLUGIN_FAILED
        );
    }
}
