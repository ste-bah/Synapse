use synapse_core::{
    Action, AimTarget, ButtonAction, ComboInput, Key, KeyCode, KeystrokeDynamics, MouseButton,
    MouseTarget, Point,
};

use crate::{ActionBackend, ActionError, EmitState};

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use enigo::{
        Axis, Button as EnigoButton, Coordinate, Direction, Enigo, Key as EnigoKey, Keyboard,
        Mouse, Settings,
    };

    use super::{
        Action, ActionBackend, ActionError, AimTarget, ButtonAction, ComboInput, EmitState, Key,
        KeyCode, KeystrokeDynamics, MouseButton, MouseTarget, Point,
    };

    #[derive(Debug, Default)]
    pub struct SoftwareBackend;

    impl SoftwareBackend {
        #[must_use]
        #[tracing::instrument(fields(backend = "software"))]
        pub fn new() -> Self {
            Self
        }
    }

    /// Reads the current X11 cursor position.
    ///
    /// # Errors
    ///
    /// Returns `ActionError::BackendUnavailable` when the X11 backend cannot be initialized
    /// or the cursor location cannot be read.
    pub fn cursor_position() -> Result<Point, ActionError> {
        let enigo = enigo()?;
        let (x, y) = enigo
            .location()
            .map_err(enigo_error("read cursor position"))?;
        Ok(Point { x, y })
    }

    /// Moves the current X11 cursor and returns the separately-read final
    /// position.
    ///
    /// # Errors
    ///
    /// Returns `ActionError::BackendUnavailable` when the X11 backend cannot be
    /// initialized, the move fails, or final cursor readback fails.
    pub fn set_cursor_position(point: Point) -> Result<Point, ActionError> {
        let mut enigo = enigo()?;
        enigo
            .move_mouse(point.x, point.y, Coordinate::Abs)
            .map_err(enigo_error("restore cursor position"))?;
        cursor_position()
    }

    impl ActionBackend for SoftwareBackend {
        #[tracing::instrument(skip_all, fields(backend = "software_linux_x11"))]
        fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
            crate::validate_action(action)?;
            match action {
                Action::KeyPress { key, hold_ms, .. } => press_key(key, *hold_ms, state),
                Action::KeyDown { key, .. } => key_down(key, state),
                Action::KeyUp { key, .. } => key_up(key, state),
                Action::KeyChord { keys, hold_ms, .. } => key_chord(keys, *hold_ms, state),
                Action::TypeText { text, dynamics, .. } => type_text(text, dynamics),
                Action::MouseMove {
                    to,
                    curve,
                    duration_ms,
                    ..
                } => mouse_move(to, curve, *duration_ms),
                Action::MouseMoveRelative { dx, dy, .. } => mouse_move_relative(*dx, *dy),
                Action::MouseButton {
                    button,
                    action,
                    hold_ms,
                    ..
                } => mouse_button(*button, *action, *hold_ms, state),
                Action::MouseDrag {
                    from,
                    to,
                    button,
                    curve,
                    duration_ms,
                    ..
                } => mouse_drag(*from, *to, *button, curve, *duration_ms, state),
                Action::MouseStroke { .. } => Err(ActionError::BackendUnavailable {
                    detail: "software Linux/X11 backend cannot emit mouse_stroke yet".to_owned(),
                }),
                Action::MouseScroll { dy, dx, at, .. } => mouse_scroll(*dy, *dx, *at),
                Action::AimAt {
                    target,
                    style,
                    deadline_ms,
                    ..
                } => aim_at(target, *style, *deadline_ms),
                Action::Combo { steps, .. } => combo(steps, state),
                Action::ReleaseAll => release_all(state),
                Action::PadButton { .. }
                | Action::PadStick { .. }
                | Action::PadTrigger { .. }
                | Action::PadReport { .. } => Err(ActionError::BackendUnavailable {
                    detail: "software backend cannot emit gamepad actions on Linux".to_owned(),
                }),
            }
        }
    }

    fn press_key(key: &Key, hold_ms: u32, state: &mut EmitState) -> Result<(), ActionError> {
        let mut enigo = enigo()?;
        emit_key(&mut enigo, key, Direction::Press)?;
        state.hold_key(key);
        sleep_ms(hold_ms);
        emit_key(&mut enigo, key, Direction::Release)?;
        state.release_key(key);
        Ok(())
    }

    fn key_down(key: &Key, state: &mut EmitState) -> Result<(), ActionError> {
        let mut enigo = enigo_preserving_held_keys()?;
        emit_key(&mut enigo, key, Direction::Press)?;
        state.hold_key(key);
        Ok(())
    }

    fn key_up(key: &Key, state: &mut EmitState) -> Result<(), ActionError> {
        let mut enigo = enigo()?;
        emit_key(&mut enigo, key, Direction::Release)?;
        state.release_key(key);
        Ok(())
    }

    fn key_chord(keys: &[Key], hold_ms: u32, state: &mut EmitState) -> Result<(), ActionError> {
        let mut enigo = enigo()?;
        for key in keys {
            emit_key(&mut enigo, key, Direction::Press)?;
            state.hold_key(key);
        }
        sleep_ms(hold_ms);
        for key in keys.iter().rev() {
            emit_key(&mut enigo, key, Direction::Release)?;
            state.release_key(key);
        }
        Ok(())
    }

    fn type_text(text: &str, dynamics: &KeystrokeDynamics) -> Result<(), ActionError> {
        if text.is_empty() {
            return Ok(());
        }

        let mut enigo = enigo()?;
        for event in crate::sample_typing_schedule(text, dynamics, None) {
            sleep_ms(event.iki_ms_before);
            enigo
                .key(EnigoKey::Unicode(event.r#char), Direction::Click)
                .map_err(enigo_error("emit text character"))?;
        }
        Ok(())
    }

    fn mouse_move(
        target: &MouseTarget,
        curve: &synapse_core::AimCurve,
        duration_ms: u32,
    ) -> Result<(), ActionError> {
        let MouseTarget::Screen { point } = target else {
            return Err(ActionError::TargetInvalid {
                detail: "software backend requires a resolved screen point for mouse movement"
                    .to_owned(),
            });
        };
        move_to_point(*point, curve, duration_ms)
    }

    fn mouse_move_relative(dx: f32, dy: f32) -> Result<(), ActionError> {
        #[allow(clippy::cast_possible_truncation)]
        let x = dx.round() as i32;
        #[allow(clippy::cast_possible_truncation)]
        let y = dy.round() as i32;
        if x == 0 && y == 0 {
            return Ok(());
        }
        let mut enigo = enigo()?;
        enigo
            .move_mouse(x, y, Coordinate::Rel)
            .map_err(enigo_error("emit relative mouse move"))
    }

    fn mouse_button(
        button: MouseButton,
        action: ButtonAction,
        hold_ms: u32,
        state: &mut EmitState,
    ) -> Result<(), ActionError> {
        let mut enigo = enigo()?;
        let button = enigo_button(button);
        match action {
            ButtonAction::Down => {
                enigo
                    .button(button, Direction::Press)
                    .map_err(enigo_error("emit mouse button"))?;
                state.apply_mouse_button(mouse_button_from_enigo(button), ButtonAction::Down);
            }
            ButtonAction::Up => {
                enigo
                    .button(button, Direction::Release)
                    .map_err(enigo_error("emit mouse button"))?;
                state.apply_mouse_button(mouse_button_from_enigo(button), ButtonAction::Up);
            }
            ButtonAction::Press => {
                enigo
                    .button(button, Direction::Press)
                    .map_err(enigo_error("emit mouse button"))?;
                state.apply_mouse_button(mouse_button_from_enigo(button), ButtonAction::Down);
                sleep_ms(hold_ms);
                enigo
                    .button(button, Direction::Release)
                    .map_err(enigo_error("emit mouse button"))?;
                state.apply_mouse_button(mouse_button_from_enigo(button), ButtonAction::Up);
            }
        }
        Ok(())
    }

    fn mouse_drag(
        from: Point,
        to: Point,
        button: MouseButton,
        curve: &synapse_core::AimCurve,
        duration_ms: u32,
        state: &mut EmitState,
    ) -> Result<(), ActionError> {
        move_to_point(from, &synapse_core::AimCurve::Instant, 0)?;
        mouse_button(button, ButtonAction::Down, 0, state)?;
        move_to_point(to, curve, duration_ms)?;
        mouse_button(button, ButtonAction::Up, 0, state)
    }

    fn mouse_scroll(dy: i32, dx: i32, at: Option<Point>) -> Result<(), ActionError> {
        if dy == 0 && dx == 0 && at.is_none() {
            return Ok(());
        }
        let mut enigo = enigo()?;
        if let Some(point) = at {
            enigo
                .move_mouse(point.x, point.y, Coordinate::Abs)
                .map_err(enigo_error("move before scroll"))?;
        }
        if dy != 0 {
            enigo
                .scroll(dy, Axis::Vertical)
                .map_err(enigo_error("emit vertical scroll"))?;
        }
        if dx != 0 {
            enigo
                .scroll(dx, Axis::Horizontal)
                .map_err(enigo_error("emit horizontal scroll"))?;
        }
        Ok(())
    }

    fn aim_at(
        target: &AimTarget,
        style: synapse_core::AimStyle,
        deadline_ms: u32,
    ) -> Result<(), ActionError> {
        if style == synapse_core::AimStyle::Track {
            return Err(ActionError::BackendUnavailable {
                detail: "track aim requires the M3 reflex runtime".to_owned(),
            });
        }
        let AimTarget::Screen { point } = target else {
            return Err(ActionError::TargetInvalid {
                detail: "software aim requires a resolved screen point".to_owned(),
            });
        };
        let curve = if style == synapse_core::AimStyle::Snap {
            synapse_core::AimCurve::Instant
        } else {
            synapse_core::AimCurve::Natural {
                params: synapse_core::AimNaturalParams::FAST,
            }
        };
        move_to_point(*point, &curve, deadline_ms)
    }

    fn combo(steps: &[synapse_core::ComboStep], state: &mut EmitState) -> Result<(), ActionError> {
        let start = Instant::now();
        for step in steps {
            sleep_until_combo_step(start, step.at_ms);
            match &step.input {
                ComboInput::KeyDown { key } => key_down(key, state)?,
                ComboInput::KeyUp { key } => key_up(key, state)?,
                ComboInput::KeyPress { key, hold_ms } => {
                    press_key(key, u32::from(*hold_ms), state)?;
                }
                ComboInput::MouseButton { button, action } => {
                    mouse_button(*button, *action, 0, state)?;
                }
                ComboInput::MouseMoveRel { dx, dy } => mouse_move_relative(*dx, *dy)?,
                ComboInput::PadButton { .. } | ComboInput::PadStick { .. } => {
                    return Err(ActionError::BackendUnavailable {
                        detail: "software backend cannot emit gamepad combo steps on Linux"
                            .to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    fn sleep_until_combo_step(start: Instant, at_ms: u32) {
        let elapsed_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        let delay_ms = at_ms.saturating_sub(elapsed_ms);
        sleep_ms(delay_ms);
    }

    fn release_all(state: &mut EmitState) -> Result<(), ActionError> {
        let snapshot = state.snapshot();
        if snapshot.held_keys.is_empty()
            && snapshot.held_buttons.is_empty()
            && snapshot.pad_state.is_empty()
        {
            state.release_all();
            return Ok(());
        }

        let mut enigo = enigo()?;
        for key in snapshot.held_keys.iter().rev() {
            emit_key(&mut enigo, key, Direction::Release)?;
        }
        for button in snapshot.held_buttons.iter().rev() {
            enigo
                .button(enigo_button(*button), Direction::Release)
                .map_err(enigo_error("release held mouse button"))?;
        }
        state.release_all();
        Ok(())
    }

    fn move_to_point(
        point: Point,
        curve: &synapse_core::AimCurve,
        duration_ms: u32,
    ) -> Result<(), ActionError> {
        let mut enigo = enigo()?;
        let samples = if curve == &synapse_core::AimCurve::Instant {
            vec![point]
        } else {
            let start = cursor_position()?;
            crate::sample_curve(curve, start, point, duration_ms, None)
                .into_iter()
                .skip(1)
                .collect()
        };
        let pause = pause_per_sample(duration_ms, samples.len());
        for sample in samples {
            enigo
                .move_mouse(sample.x, sample.y, Coordinate::Abs)
                .map_err(enigo_error("emit absolute mouse move"))?;
            if pause > Duration::ZERO {
                thread::sleep(pause);
            }
        }
        Ok(())
    }

    fn emit_key(enigo: &mut Enigo, key: &Key, direction: Direction) -> Result<(), ActionError> {
        if key.use_scancode {
            let KeyCode::HidCode { value } = &key.code else {
                return Err(unsupported_key(key));
            };
            return enigo
                .raw(u16::from(*value), direction)
                .map_err(enigo_error("emit raw scancode"));
        }
        enigo
            .key(enigo_key(key)?, direction)
            .map_err(enigo_error("emit key"))
    }

    fn enigo_key(key: &Key) -> Result<EnigoKey, ActionError> {
        match &key.code {
            KeyCode::Symbol { value } => Ok(EnigoKey::Unicode(*value)),
            KeyCode::HidCode { .. } => Err(unsupported_key(key)),
            KeyCode::Named { value } => named_key(value).ok_or_else(|| unsupported_key(key)),
        }
    }

    fn named_key(value: &str) -> Option<EnigoKey> {
        let lower = value.trim().to_ascii_lowercase();
        if let Some(ch) = single_ascii(&lower) {
            return Some(EnigoKey::Unicode(ch));
        }
        match lower.as_str() {
            "alt" | "option" => Some(EnigoKey::Alt),
            "backspace" => Some(EnigoKey::Backspace),
            "ctrl" | "control" => Some(EnigoKey::Control),
            "delete" | "del" => Some(EnigoKey::Delete),
            "down" | "arrowdown" => Some(EnigoKey::DownArrow),
            "end" => Some(EnigoKey::End),
            "enter" | "return" => Some(EnigoKey::Return),
            "escape" | "esc" => Some(EnigoKey::Escape),
            "home" => Some(EnigoKey::Home),
            "insert" | "ins" => Some(EnigoKey::Insert),
            "left" | "arrowleft" => Some(EnigoKey::LeftArrow),
            "meta" | "win" | "windows" | "super" | "command" | "cmd" => Some(EnigoKey::Meta),
            "pagedown" | "page_down" => Some(EnigoKey::PageDown),
            "pageup" | "page_up" => Some(EnigoKey::PageUp),
            "right" | "arrowright" => Some(EnigoKey::RightArrow),
            "shift" | "leftshift" | "lshift" | "rightshift" | "rshift" => Some(EnigoKey::Shift),
            "space" => Some(EnigoKey::Space),
            "tab" => Some(EnigoKey::Tab),
            "up" | "arrowup" => Some(EnigoKey::UpArrow),
            "f1" => Some(EnigoKey::F1),
            "f2" => Some(EnigoKey::F2),
            "f3" => Some(EnigoKey::F3),
            "f4" => Some(EnigoKey::F4),
            "f5" => Some(EnigoKey::F5),
            "f6" => Some(EnigoKey::F6),
            "f7" => Some(EnigoKey::F7),
            "f8" => Some(EnigoKey::F8),
            "f9" => Some(EnigoKey::F9),
            "f10" => Some(EnigoKey::F10),
            "f11" => Some(EnigoKey::F11),
            "f12" => Some(EnigoKey::F12),
            "f13" => Some(EnigoKey::F13),
            "f14" => Some(EnigoKey::F14),
            "f15" => Some(EnigoKey::F15),
            "f16" => Some(EnigoKey::F16),
            "f17" => Some(EnigoKey::F17),
            "f18" => Some(EnigoKey::F18),
            "f19" => Some(EnigoKey::F19),
            "f20" => Some(EnigoKey::F20),
            "f21" => Some(EnigoKey::F21),
            "f22" => Some(EnigoKey::F22),
            "f23" => Some(EnigoKey::F23),
            "f24" => Some(EnigoKey::F24),
            _ => None,
        }
    }

    const fn enigo_button(button: MouseButton) -> EnigoButton {
        match button {
            MouseButton::Left => EnigoButton::Left,
            MouseButton::Right => EnigoButton::Right,
            MouseButton::Middle => EnigoButton::Middle,
            MouseButton::X1 => EnigoButton::Back,
            MouseButton::X2 => EnigoButton::Forward,
        }
    }

    fn mouse_button_from_enigo(button: EnigoButton) -> MouseButton {
        match button {
            EnigoButton::Left => MouseButton::Left,
            EnigoButton::Right => MouseButton::Right,
            EnigoButton::Middle => MouseButton::Middle,
            EnigoButton::Back => MouseButton::X1,
            EnigoButton::Forward => MouseButton::X2,
            _ => unreachable!("only mapped X11 mouse buttons are converted back to Synapse"),
        }
    }

    fn enigo() -> Result<Enigo, ActionError> {
        Enigo::new(&Settings::default()).map_err(|err| ActionError::BackendUnavailable {
            detail: format!("failed to initialize enigo X11 backend: {err}"),
        })
    }

    fn enigo_preserving_held_keys() -> Result<Enigo, ActionError> {
        Enigo::new(&Settings {
            release_keys_when_dropped: false,
            ..Settings::default()
        })
        .map_err(|err| ActionError::BackendUnavailable {
            detail: format!("failed to initialize enigo X11 backend: {err}"),
        })
    }

    fn enigo_error(context: &'static str) -> impl FnOnce(enigo::InputError) -> ActionError {
        move |err| ActionError::BackendUnavailable {
            detail: format!("{context} through enigo X11 backend failed: {err}"),
        }
    }

    fn unsupported_key(key: &Key) -> ActionError {
        ActionError::UnsupportedKey {
            detail: format!(
                "software Linux backend does not support key code {:?}",
                key.code
            ),
        }
    }

    fn single_ascii(value: &str) -> Option<char> {
        let mut chars = value.chars();
        let ch = chars.next()?;
        chars.next().is_none().then_some(ch)
    }

    fn sleep_ms(milliseconds: u32) {
        if milliseconds > 0 {
            thread::sleep(Duration::from_millis(u64::from(milliseconds)));
        }
    }

    fn pause_per_sample(duration_ms: u32, samples: usize) -> Duration {
        if duration_ms == 0 || samples <= 1 {
            return Duration::ZERO;
        }
        let divisor = u64::try_from(samples).unwrap_or(u64::MAX).max(1);
        Duration::from_millis(u64::from(duration_ms) / divisor)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub use linux::{SoftwareBackend, cursor_position, set_cursor_position};

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod unsupported {
    use super::{Action, ActionBackend, ActionError, EmitState, Point};

    #[derive(Debug, Default)]
    pub struct SoftwareBackend;

    impl SoftwareBackend {
        #[must_use]
        #[tracing::instrument(fields(backend = "software"))]
        pub fn new() -> Self {
            Self
        }
    }

    /// Reads the current software cursor position from the OS cursor backend.
    ///
    /// # Errors
    ///
    /// Always returns `ActionError::BackendUnavailable` on unsupported targets.
    pub fn cursor_position() -> Result<Point, ActionError> {
        Err(ActionError::BackendUnavailable {
            detail: "software cursor position is implemented on Windows and Linux/X11 only"
                .to_owned(),
        })
    }

    /// Moves the current software cursor and returns final cursor readback.
    ///
    /// # Errors
    ///
    /// Always returns `ActionError::BackendUnavailable` on unsupported targets.
    pub fn set_cursor_position(_point: Point) -> Result<Point, ActionError> {
        Err(ActionError::BackendUnavailable {
            detail: "software cursor restore is implemented on Windows and Linux/X11 only"
                .to_owned(),
        })
    }

    impl ActionBackend for SoftwareBackend {
        #[tracing::instrument(skip_all, fields(backend = "software"))]
        fn execute(&self, action: &Action, state: &mut EmitState) -> Result<(), ActionError> {
            crate::validate_action(action)?;
            if matches!(action, Action::ReleaseAll) {
                let snapshot = state.snapshot();
                if snapshot.held_keys.is_empty()
                    && snapshot.held_buttons.is_empty()
                    && snapshot.pad_state.is_empty()
                {
                    state.release_all();
                    return Ok(());
                }
            }
            Err(ActionError::BackendUnavailable {
                detail: format!(
                    "software backend is implemented on Windows and Linux/X11 only; current target cannot emit action_kind={}",
                    action_kind(action)
                ),
            })
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
            Action::MouseStroke { .. } => "mouse_stroke",
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
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub use unsupported::{SoftwareBackend, cursor_position, set_cursor_position};
