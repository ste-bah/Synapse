use synapse_core::{ButtonAction, GamepadReport, PadButton, PadId, Stick, Trigger};
use synapse_hid_host::HOST_COMMAND_PAD_REPORT;

use super::{HardwareGateway, sleep_ms};
use crate::{ActionError, EmitState};

pub(super) fn button<G>(
    gateway: &mut G,
    state: &mut EmitState,
    pad: PadId,
    button: PadButton,
    action: ButtonAction,
    hold_ms: u32,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    match action {
        ButtonAction::Down => set_button(gateway, state, pad, button, true),
        ButtonAction::Up => set_button(gateway, state, pad, button, false),
        ButtonAction::Press => {
            set_button(gateway, state, pad, button, true)?;
            sleep_ms(hold_ms);
            set_button(gateway, state, pad, button, false)
        }
    }
}

pub(super) fn stick<G>(
    gateway: &mut G,
    state: &mut EmitState,
    pad: PadId,
    stick: Stick,
    x: f32,
    y: f32,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let mut next = report_for_pad(state, pad);
    match stick {
        Stick::Left => next.thumb_l = (x, y),
        Stick::Right => next.thumb_r = (x, y),
    }
    emit_report(gateway, state, pad, next)
}

pub(super) fn trigger<G>(
    gateway: &mut G,
    state: &mut EmitState,
    pad: PadId,
    trigger: Trigger,
    value: f32,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let mut next = report_for_pad(state, pad);
    match trigger {
        Trigger::Left => next.lt = value,
        Trigger::Right => next.rt = value,
    }
    emit_report(gateway, state, pad, next)
}

pub(super) fn report<G>(
    gateway: &mut G,
    state: &mut EmitState,
    pad: PadId,
    report: GamepadReport,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    emit_report(gateway, state, pad, report)
}

fn set_button<G>(
    gateway: &mut G,
    state: &mut EmitState,
    pad: PadId,
    button: PadButton,
    down: bool,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let mut next = report_for_pad(state, pad);
    if down {
        push_unique(&mut next.buttons, button);
    } else {
        next.buttons.retain(|held| *held != button);
    }
    emit_report(gateway, state, pad, next)
}

fn emit_report<G>(
    gateway: &mut G,
    state: &mut EmitState,
    pad: PadId,
    report: GamepadReport,
) -> Result<(), ActionError>
where
    G: HardwareGateway,
{
    let payload = encode_report(&report);
    gateway.send_command(HOST_COMMAND_PAD_REPORT, &payload)?;
    if is_neutral(&report) {
        state.pad_state.remove(&pad);
    } else {
        state.pad_state.insert(pad, report);
    }
    Ok(())
}

fn report_for_pad(state: &EmitState, pad: PadId) -> GamepadReport {
    state
        .pad_state
        .get(&pad)
        .cloned()
        .unwrap_or_else(GamepadReport::default)
}

pub(super) fn encode_report(report: &GamepadReport) -> [u8; 14] {
    let buttons = buttons_raw(&report.buttons).to_le_bytes();
    let left_trigger = normalized_trigger_to_u8(report.lt);
    let right_trigger = normalized_trigger_to_u8(report.rt);
    let axis_bytes = [
        normalized_axis_to_i16(report.thumb_l.0).to_le_bytes(),
        normalized_axis_to_i16(report.thumb_l.1).to_le_bytes(),
        normalized_axis_to_i16(report.thumb_r.0).to_le_bytes(),
        normalized_axis_to_i16(report.thumb_r.1).to_le_bytes(),
    ];
    [
        buttons[0],
        buttons[1],
        left_trigger,
        right_trigger,
        axis_bytes[0][0],
        axis_bytes[0][1],
        axis_bytes[1][0],
        axis_bytes[1][1],
        axis_bytes[2][0],
        axis_bytes[2][1],
        axis_bytes[3][0],
        axis_bytes[3][1],
        0,
        0,
    ]
}

fn buttons_raw(buttons: &[PadButton]) -> u16 {
    buttons.iter().fold(0, |raw, button| {
        raw | match button {
            PadButton::A => 0x0001,
            PadButton::B => 0x0002,
            PadButton::X => 0x0004,
            PadButton::Y => 0x0008,
            PadButton::Lb => 0x0010,
            PadButton::Rb => 0x0020,
            PadButton::Back => 0x0040,
            PadButton::Start => 0x0080,
            PadButton::Ls => 0x0100,
            PadButton::Rs => 0x0200,
            PadButton::Up => 0x0400,
            PadButton::Down => 0x0800,
            PadButton::Left => 0x1000,
            PadButton::Right => 0x2000,
            PadButton::Guide => 0x4000,
        }
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn normalized_trigger_to_u8(value: f32) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8
}

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

fn is_neutral(report: &GamepadReport) -> bool {
    report.buttons.is_empty()
        && report.thumb_l == (0.0, 0.0)
        && report.thumb_r == (0.0, 0.0)
        && report.lt == 0.0
        && report.rt == 0.0
}

fn push_unique(buttons: &mut Vec<PadButton>, button: PadButton) {
    if !buttons.contains(&button) {
        buttons.push(button);
    }
}
