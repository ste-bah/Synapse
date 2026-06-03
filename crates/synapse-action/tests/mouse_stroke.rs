use synapse_action::{
    ActionBackend, EmitState, RecordedInput, RecordingBackend, StrokeError, plan_timed_stroke,
};
use synapse_core::{
    Action, Backend, HumanizeParams, MouseButton, PathPoint, PathSpec, StrokeTiming,
    VelocityProfile, error_codes,
};

#[test]
fn stroke_planner_duration_and_speed_keep_tick_count_and_monotonic_time() {
    let path = line_path(0.0, 0.0, 4.0, 0.0);
    let duration_plan = plan_timed_stroke(
        &path,
        VelocityProfile::Constant,
        &StrokeTiming::DurationMs { duration_ms: 4 },
        None,
    )
    .expect("duration stroke should plan");
    let speed_plan = plan_timed_stroke(
        &path,
        VelocityProfile::Constant,
        &StrokeTiming::SpeedPxPerSec { px_per_sec: 1000.0 },
        None,
    )
    .expect("speed stroke should plan");

    println!(
        "readback=mouse_stroke_plan edge=duration_speed before=path:{path:?} after_duration={:?} after_speed={:?}",
        duration_plan.samples, speed_plan.samples
    );
    assert_eq!(duration_plan.samples, speed_plan.samples);
    assert_eq!(duration_plan.samples.len(), 5);
    assert_eq!(duration_plan.duration_ms, 4.0);
    assert!(monotonic_elapsed(&duration_plan.samples));
    assert_eq!(duration_plan.samples[0].point, PathPoint::new(0.0, 0.0));
    assert_eq!(duration_plan.samples[4].point, PathPoint::new(4.0, 0.0));
}

#[test]
fn stroke_planner_rejects_invalid_speed_and_humanize() {
    let path = line_path(0.0, 0.0, 4.0, 0.0);
    let speed_error = plan_timed_stroke(
        &path,
        VelocityProfile::Linear,
        &StrokeTiming::SpeedPxPerSec { px_per_sec: 0.0 },
        None,
    )
    .expect_err("zero speed must fail closed");
    let humanize_error = plan_timed_stroke(
        &path,
        VelocityProfile::Linear,
        &StrokeTiming::DurationMs { duration_ms: 4 },
        Some(HumanizeParams {
            tremor_base_stddev_px: 0.0,
            tremor_velocity_scale: 0.0,
            overshoot_prob: 2.0,
            overshoot_factor_range: (1.03, 1.12),
            micro_pause_prob: 0.0,
            micro_pause_ms_range: (15, 40),
            seed: None,
        }),
    )
    .expect_err("invalid humanize probability must fail closed");

    println!(
        "readback=mouse_stroke_plan edge=invalid before=speed:0 overshoot_prob:2 after_speed={speed_error:?} after_humanize={humanize_error:?}"
    );
    assert!(matches!(speed_error, StrokeError::InvalidSpeed { .. }));
    assert!(matches!(humanize_error, StrokeError::Humanize(_)));
}

#[test]
fn recording_backend_expands_stroke_to_single_down_stream_up() {
    let backend = RecordingBackend::new();
    let mut emit_state = EmitState::new();
    let action = Action::MouseStroke {
        path: line_path(0.0, 0.0, 4.0, 0.0),
        button: Some(MouseButton::Left),
        profile: VelocityProfile::Constant,
        timing: StrokeTiming::DurationMs { duration_ms: 4 },
        humanize: None,
        backend: Backend::Software,
    };
    let before = backend.events();
    println!("readback=mouse_stroke_record edge=button before_events={before:?}");

    backend
        .execute(&action, &mut emit_state)
        .expect("recording backend should expand valid stroke");

    let after = backend.events();
    let stroke_points: Vec<_> = after
        .iter()
        .filter_map(|event| match event {
            RecordedInput::MouseStrokePoint { elapsed_ms, point } => Some((*elapsed_ms, *point)),
            _ => None,
        })
        .collect();
    println!(
        "readback=mouse_stroke_record edge=button after_events={after:?} result_value=points:{}",
        stroke_points.len()
    );

    assert!(matches!(
        after.first(),
        Some(RecordedInput::MouseButtonDown {
            button: MouseButton::Left
        })
    ));
    assert!(matches!(
        after.last(),
        Some(RecordedInput::MouseButtonUp {
            button: MouseButton::Left
        })
    ));
    assert_eq!(stroke_points.len(), 5);
    assert!(stroke_points.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    assert!(backend.held_buttons().is_empty());
    assert!(emit_state.snapshot().held_buttons.is_empty());
}

#[test]
fn recording_backend_hover_stroke_has_no_button_events() {
    let backend = RecordingBackend::new();
    let mut emit_state = EmitState::new();
    let action = Action::MouseStroke {
        path: line_path(0.0, 0.0, 2.0, 0.0),
        button: None,
        profile: VelocityProfile::Constant,
        timing: StrokeTiming::DurationMs { duration_ms: 2 },
        humanize: None,
        backend: Backend::Software,
    };

    backend
        .execute(&action, &mut emit_state)
        .expect("recording backend should expand valid hover stroke");
    let after = backend.events();
    println!("readback=mouse_stroke_record edge=hover after_events={after:?}");

    assert_eq!(after.len(), 3);
    assert!(
        after
            .iter()
            .all(|event| matches!(event, RecordedInput::MouseStrokePoint { .. }))
    );
    assert!(backend.held_buttons().is_empty());
}

#[test]
fn recording_backend_invalid_stroke_does_not_mutate_events() {
    let backend = RecordingBackend::new();
    let mut emit_state = EmitState::new();
    let action = Action::MouseStroke {
        path: line_path(5.0, 5.0, 5.0, 5.0),
        button: Some(MouseButton::Left),
        profile: VelocityProfile::Constant,
        timing: StrokeTiming::DurationMs { duration_ms: 4 },
        humanize: None,
        backend: Backend::Software,
    };
    let before = backend.events();
    println!("readback=mouse_stroke_record edge=invalid before_events={before:?}");

    let error = backend
        .execute(&action, &mut emit_state)
        .expect_err("degenerate stroke path must fail closed");
    let after = backend.events();
    println!(
        "readback=mouse_stroke_record edge=invalid after_events={after:?} data.code={} result_value=events:{}",
        error.code(),
        after.len()
    );

    assert_eq!(error.code(), error_codes::ACTION_TARGET_INVALID);
    assert!(after.is_empty());
    assert!(backend.held_buttons().is_empty());
    assert!(emit_state.snapshot().held_buttons.is_empty());
}

fn line_path(x0: f64, y0: f64, x1: f64, y1: f64) -> PathSpec {
    PathSpec::Line {
        from: PathPoint::new(x0, y0),
        to: PathPoint::new(x1, y1),
    }
}

fn monotonic_elapsed(samples: &[synapse_action::TimedPathPoint]) -> bool {
    samples
        .windows(2)
        .all(|pair| pair[0].elapsed_ms <= pair[1].elapsed_ms)
}
