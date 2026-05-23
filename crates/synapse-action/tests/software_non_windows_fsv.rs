#![cfg(not(windows))]

use synapse_action::{ActionBackend, ActionError, EmitState, backend::software::SoftwareBackend};
use synapse_core::{
    Action, AimCurve, AimStyle, AimTarget, Backend, ButtonAction, ComboInput, ComboStep,
    GamepadReport, Key, KeyCode, KeystrokeDynamics, KeystrokeNaturalParams, MouseButton,
    MouseTarget, PadButton, Point, Stick, Trigger, error_codes,
};

#[test]
fn software_backend_returns_unavailable_for_every_action_variant_on_non_windows() {
    let backend = SoftwareBackend::new();

    for (edge, action) in action_cases() {
        let mut state = EmitState::new();
        let before = state.snapshot();
        let error = match backend.execute(&action, &mut state) {
            Ok(()) => panic!("non-Windows software backend must fail closed for {edge}"),
            Err(error) => error,
        };
        let after = state.snapshot();

        assert!(matches!(error, ActionError::BackendUnavailable { .. }));
        assert_eq!(error.code(), error_codes::ACTION_BACKEND_UNAVAILABLE);
        assert_eq!(before, after);
        assert!(error.detail().contains("requires Windows"));
        println!(
            "source_of_truth=software_linux edge={edge} before={before:?} after={after:?} after_code={}",
            error.code()
        );
    }
}

fn action_cases() -> Vec<(&'static str, Action)> {
    let mut cases = Vec::new();
    cases.extend(keyboard_actions());
    cases.extend(mouse_actions());
    cases.extend(pad_actions());
    cases.extend(other_actions());
    cases
}

fn keyboard_actions() -> Vec<(&'static str, Action)> {
    vec![
        (
            "key_press",
            Action::KeyPress {
                key: key("a"),
                hold_ms: 1,
                backend: Backend::Software,
            },
        ),
        (
            "key_down",
            Action::KeyDown {
                key: key("shift"),
                backend: Backend::Software,
            },
        ),
        (
            "key_up",
            Action::KeyUp {
                key: key("shift"),
                backend: Backend::Software,
            },
        ),
        (
            "key_chord",
            Action::KeyChord {
                keys: vec![key("control"), key("s")],
                hold_ms: 1,
                backend: Backend::Software,
            },
        ),
        (
            "type_text",
            Action::TypeText {
                text: "synthetic".to_owned(),
                dynamics: KeystrokeDynamics::Natural {
                    params: KeystrokeNaturalParams::FAST,
                },
                backend: Backend::Software,
            },
        ),
    ]
}

fn mouse_actions() -> Vec<(&'static str, Action)> {
    vec![
        (
            "mouse_move",
            Action::MouseMove {
                to: MouseTarget::Screen {
                    point: Point { x: 10, y: 20 },
                },
                curve: AimCurve::Instant,
                duration_ms: 1,
                backend: Backend::Software,
            },
        ),
        (
            "mouse_move_relative",
            Action::MouseMoveRelative {
                dx: 3.0,
                dy: -2.0,
                backend: Backend::Software,
            },
        ),
        (
            "mouse_button",
            Action::MouseButton {
                button: MouseButton::Left,
                action: ButtonAction::Press,
                hold_ms: 1,
                backend: Backend::Software,
            },
        ),
        (
            "mouse_drag",
            Action::MouseDrag {
                from: Point { x: 1, y: 2 },
                to: Point { x: 3, y: 4 },
                button: MouseButton::Left,
                curve: AimCurve::Instant,
                duration_ms: 1,
                backend: Backend::Software,
            },
        ),
        (
            "mouse_scroll",
            Action::MouseScroll {
                dy: 1,
                dx: 0,
                at: Some(Point { x: 5, y: 6 }),
                backend: Backend::Software,
            },
        ),
    ]
}

fn pad_actions() -> Vec<(&'static str, Action)> {
    vec![
        (
            "pad_button",
            Action::PadButton {
                pad: 0,
                button: PadButton::A,
                action: ButtonAction::Press,
                hold_ms: 1,
            },
        ),
        (
            "pad_stick",
            Action::PadStick {
                pad: 0,
                stick: Stick::Left,
                x: 0.25,
                y: -0.5,
            },
        ),
        (
            "pad_trigger",
            Action::PadTrigger {
                pad: 0,
                trigger: Trigger::Right,
                value: 0.75,
            },
        ),
        (
            "pad_report",
            Action::PadReport {
                pad: 0,
                report: GamepadReport {
                    buttons: vec![PadButton::A],
                    thumb_l: (0.0, 0.0),
                    thumb_r: (0.0, 0.0),
                    lt: 0.0,
                    rt: 1.0,
                },
            },
        ),
    ]
}

fn other_actions() -> Vec<(&'static str, Action)> {
    vec![
        (
            "aim_at",
            Action::AimAt {
                target: AimTarget::Screen {
                    point: Point { x: 8, y: 9 },
                },
                style: AimStyle::Snap,
                deadline_ms: 16,
                backend: Backend::Software,
            },
        ),
        (
            "combo",
            Action::Combo {
                steps: vec![ComboStep {
                    at_ms: 0,
                    input: ComboInput::KeyPress {
                        key: key("enter"),
                        hold_ms: 1,
                    },
                }],
                backend: Backend::Software,
            },
        ),
        ("release_all", Action::ReleaseAll),
    ]
}

fn key(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
    }
}
