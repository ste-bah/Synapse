use std::sync::{Arc, Mutex};

use synapse_core::{
    Action, AimCurve, AimStyle, AimTarget, Backend, ButtonAction, ComboInput, ComboStep,
    GamepadController, GamepadReport, Key, KeyCode, MouseButton, MouseTarget, PadButton, Point,
    Stick, Trigger,
};
use synapse_hid_host::{
    HOST_COMMAND_KEY_DOWN, HOST_COMMAND_KEY_UP, HOST_COMMAND_MOUSE_BUTTON,
    HOST_COMMAND_MOUSE_MOVE_REL, HOST_COMMAND_MOUSE_WHEEL, HOST_COMMAND_PAD_REPORT,
    HOST_COMMAND_RELEASE_ALL,
};

use super::{HardwareBackend, HardwareGateway, pad};
use crate::{ActionBackend, ActionError, EmitState};

type CommandRecords = Arc<Mutex<Vec<(u8, Vec<u8>)>>>;

#[derive(Clone, Debug, Default)]
struct StubGateway {
    commands: CommandRecords,
}

impl StubGateway {
    fn commands(&self) -> Vec<(u8, Vec<u8>)> {
        self.commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl HardwareGateway for StubGateway {
    fn send_command(&mut self, command: u8, payload: &[u8]) -> Result<u32, ActionError> {
        let mut commands = self
            .commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        commands.push((command, payload.to_vec()));
        Ok(u32::try_from(commands.len()).unwrap_or(u32::MAX))
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn keyboard_mouse_and_release_actions_emit_firmware_commands() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();
    let before_commands = records.commands();
    let before_snapshot = state.snapshot();
    let before_command_count = before_commands.len();
    let before_key_count = before_snapshot.held_keys.len();
    let before_button_count = before_snapshot.held_buttons.len();
    let before_pad_count = before_snapshot.pad_state.len();

    backend
        .execute(
            &Action::MouseMoveRelative {
                dx: 12.0,
                dy: -6.0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("mouse move should emit: {error}"));
    backend
        .execute(
            &Action::MouseButton {
                button: MouseButton::Left,
                action: ButtonAction::Press,
                hold_ms: 0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("mouse button should emit: {error}"));
    backend
        .execute(
            &Action::MouseScroll {
                dy: -2,
                dx: 0,
                at: None,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("mouse scroll should emit: {error}"));
    backend
        .execute(
            &Action::KeyPress {
                key: hid_key(0x04),
                hold_ms: 0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("key press should emit: {error}"));
    backend
        .execute(
            &Action::KeyChord {
                keys: vec![hid_key(0x05), hid_key(0x06)],
                hold_ms: 0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("key chord should emit: {error}"));
    backend
        .execute(&Action::ReleaseAll, &mut state)
        .unwrap_or_else(|error| panic!("release all should emit: {error}"));

    let actual_commands = records.commands();
    let after_snapshot = state.snapshot();
    let actual_command_codes = [
        actual_commands[0].0,
        actual_commands[1].0,
        actual_commands[2].0,
        actual_commands[3].0,
        actual_commands[4].0,
        actual_commands[5].0,
        actual_commands[6].0,
        actual_commands[7].0,
        actual_commands[8].0,
        actual_commands[9].0,
        actual_commands[10].0,
    ];
    let actual_mouse_move_payload = payload_array::<4>(&actual_commands[0].1);
    let actual_mouse_down_payload = payload_array::<2>(&actual_commands[1].1);
    let actual_mouse_up_payload = payload_array::<2>(&actual_commands[2].1);
    let actual_wheel_payload = payload_array::<2>(&actual_commands[3].1);
    let actual_key_press_down_payload = payload_array::<1>(&actual_commands[4].1);
    let actual_release_all_payload_len = actual_commands[10].1.len();
    assert_eq!(
        actual_commands,
        vec![
            (HOST_COMMAND_MOUSE_MOVE_REL, vec![12, 0, 250, 255]),
            (HOST_COMMAND_MOUSE_BUTTON, vec![1, 1]),
            (HOST_COMMAND_MOUSE_BUTTON, vec![1, 0]),
            (HOST_COMMAND_MOUSE_WHEEL, vec![254, 0]),
            (HOST_COMMAND_KEY_DOWN, vec![0x04]),
            (HOST_COMMAND_KEY_UP, vec![0x04]),
            (HOST_COMMAND_KEY_DOWN, vec![0x05]),
            (HOST_COMMAND_KEY_DOWN, vec![0x06]),
            (HOST_COMMAND_KEY_UP, vec![0x06]),
            (HOST_COMMAND_KEY_UP, vec![0x05]),
            (HOST_COMMAND_RELEASE_ALL, vec![]),
        ]
    );
    assert_eq!(
        actual_command_codes,
        [
            0x10, 0x11, 0x11, 0x12, 0x20, 0x21, 0x20, 0x20, 0x21, 0x21, 0x40
        ]
    );
    assert_eq!(before_command_count, 0);
    assert_eq!(before_key_count, 0);
    assert_eq!(before_button_count, 0);
    assert_eq!(before_pad_count, 0);
    assert_eq!(actual_mouse_move_payload, [12, 0, 250, 255]);
    assert_eq!(actual_mouse_down_payload, [1, 1]);
    assert_eq!(actual_mouse_up_payload, [1, 0]);
    assert_eq!(actual_wheel_payload, [254, 0]);
    assert_eq!(actual_key_press_down_payload, [0x04]);
    assert_eq!(actual_release_all_payload_len, 0);
    assert!(after_snapshot.held_keys.is_empty());
    assert!(after_snapshot.held_buttons.is_empty());
}

#[test]
fn pad_actions_emit_full_report_payloads() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();

    backend
        .execute(
            &Action::PadReport {
                pad: 0,
                report: GamepadReport {
                    controller: GamepadController::X360,
                    buttons: vec![PadButton::A, PadButton::Rb],
                    thumb_l: (1.0, -1.0),
                    thumb_r: (0.5, -0.5),
                    lt: 0.25,
                    rt: 1.0,
                },
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("pad report should emit: {error}"));
    backend
        .execute(
            &Action::PadButton {
                pad: 0,
                button: PadButton::A,
                action: ButtonAction::Up,
                hold_ms: 0,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("pad button should emit: {error}"));
    backend
        .execute(
            &Action::PadStick {
                pad: 0,
                stick: Stick::Right,
                x: -0.25,
                y: 0.25,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("pad stick should emit: {error}"));
    backend
        .execute(
            &Action::PadTrigger {
                pad: 0,
                trigger: Trigger::Left,
                value: 0.0,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("pad trigger should emit: {error}"));

    let expected_first = pad::encode_report(&GamepadReport {
        controller: GamepadController::X360,
        buttons: vec![PadButton::A, PadButton::Rb],
        thumb_l: (1.0, -1.0),
        thumb_r: (0.5, -0.5),
        lt: 0.25,
        rt: 1.0,
    });
    let commands = records.commands();
    let after_snapshot = state.snapshot();
    let first_pad_payload = payload_array::<14>(&commands[0].1);
    let last_pad_payload = payload_array::<14>(&commands[3].1);
    assert_eq!(
        commands[0],
        (HOST_COMMAND_PAD_REPORT, expected_first.to_vec())
    );
    assert_eq!(commands.len(), 4);
    assert_eq!(first_pad_payload, expected_first);
    assert_eq!(
        after_snapshot.pad_state.get(&0).map_or_else(
            || panic!("pad 0 should remain non-neutral"),
            |report| report.buttons.clone()
        ),
        vec![PadButton::Rb]
    );
    assert_eq!(last_pad_payload[0], 0x20);
    assert_eq!(last_pad_payload[2], 0);
}

#[test]
fn combo_steps_emit_expected_firmware_sequence() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();

    backend
        .execute(
            &Action::Combo {
                steps: vec![
                    ComboStep {
                        at_ms: 0,
                        input: ComboInput::KeyDown { key: hid_key(0x1A) },
                    },
                    ComboStep {
                        at_ms: 10,
                        input: ComboInput::MouseMoveRel { dx: 3.0, dy: 4.0 },
                    },
                    ComboStep {
                        at_ms: 20,
                        input: ComboInput::PadButton {
                            pad: 1,
                            button: PadButton::B,
                            action: ButtonAction::Down,
                        },
                    },
                    ComboStep {
                        at_ms: 30,
                        input: ComboInput::KeyUp { key: hid_key(0x1A) },
                    },
                ],
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("combo should emit: {error}"));

    let commands = records.commands();
    let combo_command_codes = [commands[0].0, commands[1].0, commands[2].0, commands[3].0];
    let combo_mouse_payload = payload_array::<4>(&commands[1].1);
    let combo_pad_payload = payload_array::<14>(&commands[2].1);
    assert_eq!(
        commands[..3],
        [
            (HOST_COMMAND_KEY_DOWN, vec![0x1A]),
            (HOST_COMMAND_MOUSE_MOVE_REL, vec![3, 0, 4, 0]),
            (
                HOST_COMMAND_PAD_REPORT,
                pad::encode_report(&GamepadReport {
                    controller: GamepadController::X360,
                    buttons: vec![PadButton::B],
                    thumb_l: (0.0, 0.0),
                    thumb_r: (0.0, 0.0),
                    lt: 0.0,
                    rt: 0.0,
                })
                .to_vec()
            ),
        ]
    );
    assert_eq!(commands[3], (HOST_COMMAND_KEY_UP, vec![0x1A]));
    assert_eq!(combo_command_codes, [0x20, 0x10, 0x30, 0x21]);
    assert_eq!(combo_mouse_payload, [3, 0, 4, 0]);
    assert_eq!(combo_pad_payload[0], 0x02);
}

#[test]
fn unsupported_or_later_scoped_variants_fail_closed_without_commands() {
    let cases = [
        Action::TypeText {
            text: "a".to_owned(),
            dynamics: synapse_core::KeystrokeDynamics::Burst,
            backend: Backend::Hardware,
        },
        Action::MouseMove {
            to: MouseTarget::Screen {
                point: Point { x: 1, y: 2 },
            },
            curve: AimCurve::Instant,
            duration_ms: 0,
            backend: Backend::Hardware,
        },
        Action::MouseDrag {
            from: Point { x: 1, y: 2 },
            to: Point { x: 3, y: 4 },
            button: MouseButton::Left,
            curve: AimCurve::Instant,
            duration_ms: 0,
            backend: Backend::Hardware,
        },
        Action::AimAt {
            target: AimTarget::Screen {
                point: Point { x: 5, y: 6 },
            },
            style: AimStyle::Snap,
            deadline_ms: 0,
            backend: Backend::Hardware,
        },
        Action::KeyDown {
            key: Key {
                code: KeyCode::Named {
                    value: "ctrl".to_owned(),
                },
                use_scancode: false,
            },
            backend: Backend::Hardware,
        },
        Action::MouseButton {
            button: MouseButton::X1,
            action: ButtonAction::Down,
            hold_ms: 0,
            backend: Backend::Hardware,
        },
        Action::MouseScroll {
            dy: 1,
            dx: 1,
            at: None,
            backend: Backend::Hardware,
        },
        Action::MouseMoveRelative {
            dx: 128.0,
            dy: 0.0,
            backend: Backend::Hardware,
        },
    ];

    for (case_index, action) in cases.into_iter().enumerate() {
        let observed_case_index = case_index;
        let gateway = StubGateway::default();
        let records = gateway.clone();
        let backend = HardwareBackend::with_gateway(gateway);
        let mut state = EmitState::new();
        let before_commands = records.commands();
        let before_snapshot = state.snapshot();
        let result = backend.execute(&action, &mut state);
        let after_commands = records.commands();
        let after_snapshot = state.snapshot();
        let result_is_err = result.is_err();
        let before_command_count = before_commands.len();
        let after_command_count = after_commands.len();
        let after_key_count = after_snapshot.held_keys.len();
        let after_button_count = after_snapshot.held_buttons.len();
        let after_pad_count = after_snapshot.pad_state.len();
        assert!(result_is_err);
        assert!(observed_case_index < 8);
        assert!(before_commands.is_empty());
        assert_eq!(before_snapshot, EmitState::new().snapshot());
        assert!(after_commands.is_empty());
        assert_eq!(before_command_count, 0);
        assert_eq!(after_command_count, 0);
        assert_eq!(after_key_count, 0);
        assert_eq!(after_button_count, 0);
        assert_eq!(after_pad_count, 0);
        assert_eq!(after_snapshot, EmitState::new().snapshot());
    }
}

fn payload_array<const N: usize>(payload: &[u8]) -> [u8; N] {
    match payload.try_into() {
        Ok(array) => array,
        Err(error) => panic!("payload length should match expected fixed array length: {error:?}"),
    }
}

fn hid_key(value: u8) -> Key {
    Key {
        code: KeyCode::HidCode { value },
        use_scancode: true,
    }
}
