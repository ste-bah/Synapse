use std::sync::{Arc, Mutex};

use synapse_core::{
    Action, AimCurve, AimNaturalParams, AimStyle, AimTarget, Backend, ButtonAction, ComboInput,
    ComboStep, ElementId, GamepadController, GamepadReport, Key, KeyCode, MouseButton, MouseTarget,
    PadButton, Point, Stick, Trigger,
};
use synapse_hid_host::{
    HOST_COMMAND_KEY_DOWN, HOST_COMMAND_KEY_MODS, HOST_COMMAND_KEY_UP, HOST_COMMAND_MOUSE_BUTTON,
    HOST_COMMAND_MOUSE_MOVE_REL, HOST_COMMAND_MOUSE_WHEEL, HOST_COMMAND_PAD_REPORT,
    HOST_COMMAND_RELEASE_ALL, HostCommandRequest,
};

use super::{HardwareBackend, HardwareGateway, pad};
use crate::{ActionBackend, ActionError, EmitState};

type CommandRecords = Arc<Mutex<Vec<(u8, Vec<u8>)>>>;
type BatchRecords = Arc<Mutex<Vec<usize>>>;

#[derive(Clone, Debug, Default)]
struct StubGateway {
    commands: CommandRecords,
    batch_lengths: BatchRecords,
}

impl StubGateway {
    fn commands(&self) -> Vec<(u8, Vec<u8>)> {
        self.commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn batch_lengths(&self) -> Vec<usize> {
        self.batch_lengths
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

    fn send_commands(
        &mut self,
        commands: &[HostCommandRequest<'_>],
    ) -> Result<Vec<u32>, ActionError> {
        self.batch_lengths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(commands.len());
        commands
            .iter()
            .map(|request| self.send_command(request.command, request.payload))
            .collect()
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
                key: named_key("a"),
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
fn named_symbol_and_text_keys_emit_hid_usage_payloads() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();

    backend
        .execute(
            &Action::KeyPress {
                key: named_key("right"),
                hold_ms: 0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("named arrow should emit: {error}"));
    backend
        .execute(
            &Action::KeyPress {
                key: symbol_key('?'),
                hold_ms: 0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("US-layout symbol should emit: {error}"));
    backend
        .execute(
            &Action::TypeText {
                text: "A!az0- ".to_owned(),
                dynamics: synapse_core::KeystrokeDynamics::Burst,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("US-layout text should emit: {error}"));

    assert_eq!(
        records.commands(),
        vec![
            (HOST_COMMAND_KEY_DOWN, vec![0x4F]),
            (HOST_COMMAND_KEY_UP, vec![0x4F]),
            (HOST_COMMAND_KEY_MODS, vec![0x02]),
            (HOST_COMMAND_KEY_DOWN, vec![0x38]),
            (HOST_COMMAND_KEY_UP, vec![0x38]),
            (HOST_COMMAND_KEY_MODS, vec![0x00]),
            (HOST_COMMAND_KEY_MODS, vec![0x02]),
            (HOST_COMMAND_KEY_DOWN, vec![0x04]),
            (HOST_COMMAND_KEY_UP, vec![0x04]),
            (HOST_COMMAND_KEY_MODS, vec![0x00]),
            (HOST_COMMAND_KEY_MODS, vec![0x02]),
            (HOST_COMMAND_KEY_DOWN, vec![0x1E]),
            (HOST_COMMAND_KEY_UP, vec![0x1E]),
            (HOST_COMMAND_KEY_MODS, vec![0x00]),
            (HOST_COMMAND_KEY_DOWN, vec![0x04]),
            (HOST_COMMAND_KEY_UP, vec![0x04]),
            (HOST_COMMAND_KEY_DOWN, vec![0x1D]),
            (HOST_COMMAND_KEY_UP, vec![0x1D]),
            (HOST_COMMAND_KEY_DOWN, vec![0x27]),
            (HOST_COMMAND_KEY_UP, vec![0x27]),
            (HOST_COMMAND_KEY_DOWN, vec![0x2D]),
            (HOST_COMMAND_KEY_UP, vec![0x2D]),
            (HOST_COMMAND_KEY_DOWN, vec![0x2C]),
            (HOST_COMMAND_KEY_UP, vec![0x2C]),
        ]
    );
}

#[test]
fn absolute_hardware_mouse_uses_natural_fast_relative_curve() {
    let mut gateway = StubGateway::default();
    let records = gateway.clone();
    let start = Point { x: 0, y: 0 };
    let target = Point { x: 200, y: 200 };
    let curve = AimCurve::Natural {
        params: AimNaturalParams::FAST,
    };
    println!(
        "readback=hardware_mouse_absolute edge=happy before=start:{start:?} target:{target:?} commands:{:?} batches:{:?}",
        records.commands(),
        records.batch_lengths()
    );

    super::mouse::move_curve_from(&mut gateway, start, target, &curve, 50)
        .unwrap_or_else(|error| panic!("absolute hardware mouse fallback should emit: {error}"));

    let commands = records.commands();
    let actual_points = cumulative_mouse_points(&commands, start);
    let expected_points = crate::sample_curve(&curve, start, target, 50, None)
        .into_iter()
        .skip(1)
        .collect::<Vec<_>>();
    assert!(!commands.is_empty());
    assert_eq!(actual_points, expected_points);
    assert_eq!(actual_points.last(), Some(&target));
    assert!(relative_payloads_within_firmware_range(&commands));
    assert_eq!(records.batch_lengths(), vec![commands.len()]);
    println!(
        "readback=hardware_mouse_absolute edge=happy after=commands:{commands:?} points:{actual_points:?} batches:{:?}",
        records.batch_lengths()
    );
}

#[test]
fn absolute_hardware_mouse_chunks_large_relative_deltas() {
    let mut gateway = StubGateway::default();
    let records = gateway.clone();
    let start = Point { x: 0, y: 0 };
    let target = Point { x: 400, y: -300 };
    println!(
        "readback=hardware_mouse_absolute edge=large_delta before=start:{start:?} target:{target:?} commands:{:?} batches:{:?}",
        records.commands(),
        records.batch_lengths()
    );

    super::mouse::move_curve_from(&mut gateway, start, target, &AimCurve::Instant, 0)
        .unwrap_or_else(|error| panic!("large absolute fallback should chunk: {error}"));

    let commands = records.commands();
    let actual_points = cumulative_mouse_points(&commands, start);
    assert_eq!(actual_points.last(), Some(&target));
    assert_eq!(commands.len(), 4);
    assert_eq!(
        commands,
        vec![
            (HOST_COMMAND_MOUSE_MOVE_REL, vec![127, 0, 129, 255]),
            (HOST_COMMAND_MOUSE_MOVE_REL, vec![127, 0, 129, 255]),
            (HOST_COMMAND_MOUSE_MOVE_REL, vec![127, 0, 210, 255]),
            (HOST_COMMAND_MOUSE_MOVE_REL, vec![19, 0, 0, 0]),
        ]
    );
    assert!(relative_payloads_within_firmware_range(&commands));
    assert_eq!(records.batch_lengths(), vec![commands.len()]);
    println!(
        "readback=hardware_mouse_absolute edge=large_delta after=commands:{commands:?} points:{actual_points:?} batches:{:?}",
        records.batch_lengths()
    );
}

#[test]
fn absolute_hardware_mouse_zero_delta_emits_no_commands() {
    let mut gateway = StubGateway::default();
    let records = gateway.clone();
    let point = Point { x: 77, y: -12 };
    println!(
        "readback=hardware_mouse_absolute edge=zero_delta before=point:{point:?} commands:{:?} batches:{:?}",
        records.commands(),
        records.batch_lengths()
    );

    super::mouse::move_curve_from(&mut gateway, point, point, &AimCurve::Instant, 0)
        .unwrap_or_else(|error| panic!("zero absolute fallback should no-op: {error}"));

    assert!(records.commands().is_empty());
    assert!(records.batch_lengths().is_empty());
    println!(
        "readback=hardware_mouse_absolute edge=zero_delta after=commands:{:?} batches:{:?}",
        records.commands(),
        records.batch_lengths()
    );
}

#[test]
fn absolute_hardware_mouse_repeated_warps_do_not_accumulate_drift() {
    let mut gateway = StubGateway::default();
    let records = gateway.clone();
    let curve = AimCurve::Natural {
        params: AimNaturalParams::FAST,
    };
    let mut current = Point { x: 0, y: 0 };
    let mut cumulative_drift_px = 0_i64;
    println!(
        "readback=hardware_mouse_absolute edge=repeated_warps before=current:{current:?} commands:{:?} batches:{:?}",
        records.commands(),
        records.batch_lengths()
    );

    for index in 0..100 {
        let target = Point {
            x: (index * 7 % 600) - 300,
            y: (index * 11 % 400) - 200,
        };
        let before_count = records.commands().len();

        super::mouse::move_curve_from(&mut gateway, current, target, &curve, 50)
            .unwrap_or_else(|error| panic!("repeated absolute fallback should emit: {error}"));

        let commands = records.commands();
        let new_commands = &commands[before_count..];
        assert!(relative_payloads_within_firmware_range(new_commands));
        let actual_points = cumulative_mouse_points(new_commands, current);
        let final_point = actual_points.last().copied().unwrap_or(current);
        cumulative_drift_px += i64::from((final_point.x - target.x).abs());
        cumulative_drift_px += i64::from((final_point.y - target.y).abs());
        current = target;
    }

    assert!(cumulative_drift_px < 5);
    assert_eq!(records.batch_lengths().len(), 100);
    println!(
        "readback=hardware_mouse_absolute edge=repeated_warps after=final_current:{current:?} cumulative_drift_px:{cumulative_drift_px} command_count:{} batch_count:{}",
        records.commands().len(),
        records.batch_lengths().len()
    );
}

#[test]
fn modifier_chords_emit_mod_bits_before_and_after_key_usage() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();

    backend
        .execute(
            &Action::KeyChord {
                keys: vec![
                    named_key("ctrl"),
                    named_key("shift"),
                    named_key("alt"),
                    named_key("f12"),
                ],
                hold_ms: 0,
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("modifier chord should emit: {error}"));

    let commands = records.commands();
    assert_eq!(
        commands,
        vec![
            (HOST_COMMAND_KEY_MODS, vec![0x01]),
            (HOST_COMMAND_KEY_MODS, vec![0x03]),
            (HOST_COMMAND_KEY_MODS, vec![0x07]),
            (HOST_COMMAND_KEY_DOWN, vec![0x45]),
            (HOST_COMMAND_KEY_UP, vec![0x45]),
            (HOST_COMMAND_KEY_MODS, vec![0x03]),
            (HOST_COMMAND_KEY_MODS, vec![0x01]),
            (HOST_COMMAND_KEY_MODS, vec![0x00]),
        ]
    );
    assert!(state.snapshot().held_keys.is_empty());
    assert_eq!(payload_array::<1>(&commands[2].1), [0x07]);
    assert_eq!(payload_array::<1>(&commands[3].1), [0x45]);
}

#[test]
fn pure_modifier_key_down_up_uses_modifier_byte_only() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();

    backend
        .execute(
            &Action::KeyDown {
                key: named_key("shift"),
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("shift down should emit modifier byte: {error}"));
    let after_down_snapshot = state.snapshot();
    backend
        .execute(
            &Action::KeyUp {
                key: named_key("shift"),
                backend: Backend::Hardware,
            },
            &mut state,
        )
        .unwrap_or_else(|error| panic!("shift up should clear modifier byte: {error}"));

    assert_eq!(
        records.commands(),
        vec![
            (HOST_COMMAND_KEY_MODS, vec![0x02]),
            (HOST_COMMAND_KEY_MODS, vec![0x00]),
        ]
    );
    assert_eq!(after_down_snapshot.held_keys, vec![named_key("shift")]);
    assert!(state.snapshot().held_keys.is_empty());
}

#[test]
fn seventh_non_modifier_key_fails_closed_without_extra_command() {
    let gateway = StubGateway::default();
    let records = gateway.clone();
    let backend = HardwareBackend::with_gateway(gateway);
    let mut state = EmitState::new();

    for usage in 0x04..=0x09 {
        backend
            .execute(
                &Action::KeyDown {
                    key: hid_key(usage),
                    backend: Backend::Hardware,
                },
                &mut state,
            )
            .unwrap_or_else(|error| panic!("first six keys should hold: {error}"));
    }

    let before_commands = records.commands();
    let before_snapshot = state.snapshot();
    let result = backend.execute(
        &Action::KeyDown {
            key: hid_key(0x0A),
            backend: Backend::Hardware,
        },
        &mut state,
    );
    let after_commands = records.commands();
    let after_snapshot = state.snapshot();

    let error = match result {
        Ok(()) => panic!("seventh non-modifier key should fail closed"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        synapse_core::error_codes::ACTION_UNSUPPORTED_KEY
    );
    assert!(
        error.detail().contains("6KRO limit"),
        "unexpected detail: {}",
        error.detail()
    );
    assert_eq!(before_commands.len(), 6);
    assert_eq!(after_commands, before_commands);
    assert_eq!(before_snapshot.held_keys.len(), 6);
    assert_eq!(after_snapshot, before_snapshot);
}

#[test]
fn unsupported_or_later_scoped_variants_fail_closed_without_commands() {
    let cases = [
        Action::TypeText {
            text: "€".to_owned(),
            dynamics: synapse_core::KeystrokeDynamics::Burst,
            backend: Backend::Hardware,
        },
        Action::MouseMove {
            to: MouseTarget::Element {
                element_id: element_id(),
            },
            curve: AimCurve::Instant,
            duration_ms: 0,
            backend: Backend::Hardware,
        },
        Action::AimAt {
            target: AimTarget::Element {
                element_id: element_id(),
            },
            style: AimStyle::Snap,
            deadline_ms: 0,
            backend: Backend::Hardware,
        },
        Action::AimAt {
            target: AimTarget::Track { track_id: 7 },
            style: AimStyle::Snap,
            deadline_ms: 0,
            backend: Backend::Hardware,
        },
        Action::KeyDown {
            key: Key {
                code: KeyCode::Named {
                    value: "not-a-hid-key".to_owned(),
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

    for action in cases {
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

fn cumulative_mouse_points(commands: &[(u8, Vec<u8>)], start: Point) -> Vec<Point> {
    let mut current = start;
    let mut points = Vec::new();
    for (command, payload) in commands {
        assert_eq!(*command, HOST_COMMAND_MOUSE_MOVE_REL);
        let (dx, dy) = relative_mouse_payload(payload);
        current.x += i32::from(dx);
        current.y += i32::from(dy);
        points.push(current);
    }
    points
}

fn relative_payloads_within_firmware_range(commands: &[(u8, Vec<u8>)]) -> bool {
    commands.iter().all(|(_command, payload)| {
        let (dx, dy) = relative_mouse_payload(payload);
        (-127..=127).contains(&dx) && (-127..=127).contains(&dy)
    })
}

fn relative_mouse_payload(payload: &[u8]) -> (i16, i16) {
    let bytes = payload_array::<4>(payload);
    (
        i16::from_le_bytes([bytes[0], bytes[1]]),
        i16::from_le_bytes([bytes[2], bytes[3]]),
    )
}

fn hid_key(value: u8) -> Key {
    Key {
        code: KeyCode::HidCode { value },
        use_scancode: true,
    }
}

fn named_key(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
    }
}

fn symbol_key(value: char) -> Key {
    Key {
        code: KeyCode::Symbol { value },
        use_scancode: false,
    }
}

fn element_id() -> ElementId {
    ElementId::parse("0x1:1")
        .unwrap_or_else(|error| panic!("static element id should parse: {error}"))
}
