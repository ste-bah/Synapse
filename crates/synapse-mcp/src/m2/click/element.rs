use std::time::Instant;

use rmcp::ErrorData;
use synapse_action::{
    ActionError, ActionHandle, DoubleClickTiming, ElementClickOutcome, EmitState, RecordingBackend,
    click_element_or_fallback,
};
use synapse_core::{Action, ButtonAction, MouseButton, MouseTarget, Point, error_codes};
use tokio::time::{Duration, sleep};

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT as WinPoint, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        UI::WindowsAndMessaging::{
            EnumChildWindows, GetClassNameW, GetWindowRect, IsWindow, IsWindowVisible,
            PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
            WM_RBUTTONDOWN, WM_RBUTTONUP,
        },
    },
    core::BOOL,
};

use super::{
    action_error_to_mcp, backend_used_name, record,
    schema::{ActClickElementTarget, ActClickParams, ActClickResponse},
};

pub(super) async fn execute_element_click(
    handle: ActionHandle,
    params: &ActClickParams,
    element: &ActClickElementTarget,
    recording: Option<&RecordingBackend>,
    timing: DoubleClickTiming,
    started: Instant,
) -> Result<ActClickResponse, ErrorData> {
    if !params.use_invoke_pattern {
        let screen_point = element_center(&element.element_id)?;
        trace_element_click_outcome(element, 0, "coordinate_direct", Some(screen_point));
        let actions = coordinate_click_actions(params, screen_point);
        let backend_used = if let Some(recording) = recording {
            record::execute_recording(recording, &actions, params.clicks, timing).await?;
            backend_used_name(params.backend).to_owned()
        } else {
            match record::execute_actor_actions(handle, actions, timing).await {
                Ok(()) => backend_used_name(params.backend).to_owned(),
                Err(error) if should_try_hwnd_message_fallback(&error) => {
                    post_element_window_message_click(params, element, screen_point, timing).await?
                }
                Err(error) => return Err(error),
            }
        };
        return Ok(ActClickResponse {
            ok: true,
            used_invoke_pattern: false,
            backend_used,
            press_hold_ms: params.hold_ms,
            double_click_window_ms: timing.window_ms,
            inter_click_delay_ms: timing.inter_click_delay_ms,
            elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        });
    }

    let mut state = EmitState::new();
    let mut used_invoke_pattern = false;
    let mut backend_used = "software";
    for click_index in 0..params.clicks {
        let outcome = if let Some(recording) = recording {
            click_element_or_fallback(&element.element_id, recording, &mut state, params.button)
        } else {
            let backend = synapse_action::backend::software::SoftwareBackend::new();
            click_element_or_fallback(&element.element_id, &backend, &mut state, params.button)
        }
        .map_err(|error| action_error_to_mcp(&error))?;

        match outcome {
            ElementClickOutcome::Invoked => {
                trace_element_click_outcome(element, click_index, "invoked", None);
                used_invoke_pattern = true;
                backend_used = "uia";
            }
            ElementClickOutcome::Toggled => {
                trace_element_click_outcome(element, click_index, "toggled", None);
                used_invoke_pattern = true;
                backend_used = "uia";
            }
            ElementClickOutcome::CoordinateFallback(plan) => {
                trace_element_click_outcome(
                    element,
                    click_index,
                    "coordinate_fallback",
                    Some(plan.screen_point),
                );
                backend_used = "software";
            }
        }

        if click_index + 1 < params.clicks {
            sleep(Duration::from_millis(u64::from(
                timing.inter_click_delay_ms,
            )))
            .await;
        }
    }

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern,
        backend_used: backend_used.to_owned(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: timing.window_ms,
        inter_click_delay_ms: timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

fn coordinate_click_actions(params: &ActClickParams, screen_point: Point) -> Vec<Action> {
    let mut actions = Vec::with_capacity(usize::from(params.clicks) + 1);
    actions.push(Action::MouseMove {
        to: MouseTarget::Screen {
            point: screen_point,
        },
        curve: params.velocity_profile.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend: params.backend,
    });
    for _ in 0..params.clicks {
        actions.push(Action::MouseButton {
            button: params.button,
            action: ButtonAction::Press,
            hold_ms: params.hold_ms,
            backend: params.backend,
        });
    }
    actions
}

fn should_try_hwnd_message_fallback(error: &ErrorData) -> bool {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(serde_json::Value::as_str)
        == Some(error_codes::ACTION_BACKEND_UNAVAILABLE)
}

#[cfg(windows)]
async fn post_element_window_message_click(
    params: &ActClickParams,
    element: &ActClickElementTarget,
    screen_point: Point,
    timing: DoubleClickTiming,
) -> Result<String, ErrorData> {
    let readback =
        windows_hwnd_message_click_readback(&element.element_id, screen_point, params.button)
            .map_err(|error| action_error_to_mcp(&error))?;
    for click_index in 0..params.clicks {
        post_mouse_message(readback.hwnd, WM_MOUSEMOVE, 0, readback.client_point)
            .map_err(|error| action_error_to_mcp(&error))?;
        let (down_message, up_message, down_wparam) =
            mouse_button_messages(params.button).map_err(|error| action_error_to_mcp(&error))?;
        post_mouse_message(
            readback.hwnd,
            down_message,
            down_wparam,
            readback.client_point,
        )
        .map_err(|error| action_error_to_mcp(&error))?;
        sleep(Duration::from_millis(u64::from(params.hold_ms))).await;
        post_mouse_message(readback.hwnd, up_message, 0, readback.client_point)
            .map_err(|error| action_error_to_mcp(&error))?;

        tracing::info!(
            code = "M2_ACT_CLICK_ELEMENT_HWND_MESSAGE_FALLBACK",
            kind = "act_click",
            element_id = %element.element_id,
            root_hwnd = readback.root_hwnd,
            target_hwnd = readback.hwnd,
            target_class = %readback.class_name,
            screen_x = screen_point.x,
            screen_y = screen_point.y,
            client_x = readback.client_point.x,
            client_y = readback.client_point.y,
            click_number = u32::from(click_index) + 1,
            button = ?params.button,
            "readback=window_message tool=act_click element_click_after"
        );

        if click_index + 1 < params.clicks {
            sleep(Duration::from_millis(u64::from(
                timing.inter_click_delay_ms,
            )))
            .await;
        }
    }
    Ok("software_window_message".to_owned())
}

#[cfg(not(windows))]
async fn post_element_window_message_click(
    _params: &ActClickParams,
    element: &ActClickElementTarget,
    _screen_point: Point,
    _timing: DoubleClickTiming,
) -> Result<String, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!(
            "act_click element target {} HWND message fallback requires Windows",
            element.element_id
        ),
    }))
}

#[cfg(windows)]
fn element_center(element_id: &synapse_core::ElementId) -> Result<Point, ErrorData> {
    let rect = synapse_a11y::element_bounding_rect(element_id).map_err(|err| {
        action_error_to_mcp(&ActionError::ElementNotResolved {
            detail: format!("act_click element {element_id} could not be resolved: {err}"),
        })
    })?;

    if rect.w <= 0 || rect.h <= 0 {
        return Err(action_error_to_mcp(&ActionError::TargetInvalid {
            detail: format!("act_click element bbox is empty or inverted: {rect:?}"),
        }));
    }

    let x = i64::from(rect.x) + i64::from(rect.w) / 2;
    let y = i64::from(rect.y) + i64::from(rect.h) / 2;
    Ok(Point {
        x: i32::try_from(x).map_err(|err| {
            action_error_to_mcp(&ActionError::TargetInvalid {
                detail: format!("act_click element bbox center x overflowed i32: {err}"),
            })
        })?,
        y: i32::try_from(y).map_err(|err| {
            action_error_to_mcp(&ActionError::TargetInvalid {
                detail: format!("act_click element bbox center y overflowed i32: {err}"),
            })
        })?,
    })
}

#[cfg(not(windows))]
fn element_center(element_id: &synapse_core::ElementId) -> Result<Point, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!(
            "act_click element target {element_id} requires Windows UI Automation bbox resolution"
        ),
    }))
}

fn trace_element_click_outcome(
    element: &ActClickElementTarget,
    click_index: u8,
    outcome: &'static str,
    fallback_screen_point: Option<Point>,
) {
    tracing::info!(
        code = "M2_ACT_CLICK_ELEMENT_READBACK",
        kind = "act_click",
        element_id = %element.element_id,
        click_number = u32::from(click_index) + 1,
        outcome,
        fallback_screen_x = fallback_screen_point.map(|point| point.x),
        fallback_screen_y = fallback_screen_point.map(|point| point.y),
        "readback=action_backend tool=act_click element_click_after"
    );
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct HwndMessageClickReadback {
    root_hwnd: i64,
    hwnd: i64,
    class_name: String,
    client_point: Point,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct WindowCandidate {
    hwnd: HWND,
    rect: RECT,
    class_name: String,
}

#[cfg(windows)]
struct ChildEnumContext {
    point: Point,
    candidates: Vec<WindowCandidate>,
}

#[cfg(windows)]
fn windows_hwnd_message_click_readback(
    element_id: &synapse_core::ElementId,
    screen_point: Point,
    button: MouseButton,
) -> Result<HwndMessageClickReadback, ActionError> {
    let _ = mouse_button_messages(button)?;
    let root_hwnd = element_id
        .parts()
        .map_err(|error| ActionError::TargetInvalid {
            detail: format!("act_click element id {element_id} could not be parsed: {error}"),
        })?
        .hwnd;
    let root = hwnd_from_i64(root_hwnd)?;
    if !unsafe { IsWindow(Some(root)) }.as_bool() {
        return Err(ActionError::ElementNotResolved {
            detail: format!("act_click root hwnd 0x{root_hwnd:x} is not a live window"),
        });
    }

    let target = best_hwnd_for_screen_point(root, screen_point)?;
    let mut client = WinPoint {
        x: screen_point.x,
        y: screen_point.y,
    };
    if !unsafe { ScreenToClient(target.hwnd, &raw mut client) }.as_bool() {
        return Err(ActionError::BackendUnavailable {
            detail: format!(
                "ScreenToClient failed for act_click target hwnd 0x{:x} at screen point {screen_point:?}",
                hwnd_to_i64(target.hwnd)
            ),
        });
    }

    let client_point = Point {
        x: client.x,
        y: client.y,
    };
    let _ = mouse_lparam(client_point)?;
    Ok(HwndMessageClickReadback {
        root_hwnd,
        hwnd: hwnd_to_i64(target.hwnd),
        class_name: target.class_name,
        client_point,
    })
}

#[cfg(windows)]
fn best_hwnd_for_screen_point(root: HWND, point: Point) -> Result<WindowCandidate, ActionError> {
    let root_rect = window_rect(root)?;
    if !rect_contains_point(&root_rect, point) {
        return Err(ActionError::TargetInvalid {
            detail: format!(
                "act_click element center {point:?} is outside root hwnd 0x{:x} rect {:?}",
                hwnd_to_i64(root),
                rect_tuple(&root_rect)
            ),
        });
    }

    let mut context = ChildEnumContext {
        point,
        candidates: Vec::new(),
    };
    let context_ptr = (&raw mut context).cast::<c_void>();
    let _ = unsafe {
        EnumChildWindows(
            Some(root),
            Some(enum_child_containing_point),
            LPARAM(context_ptr as isize),
        )
    };

    context
        .candidates
        .into_iter()
        .min_by_key(|candidate| rect_area(&candidate.rect))
        .or_else(|| {
            Some(WindowCandidate {
                hwnd: root,
                rect: root_rect,
                class_name: window_class_name(root),
            })
        })
        .ok_or_else(|| ActionError::ElementNotResolved {
            detail: format!(
                "act_click could not find root or child hwnd containing point {point:?}"
            ),
        })
}

#[cfg(windows)]
unsafe extern "system" fn enum_child_containing_point(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = unsafe { &mut *(lparam.0 as *mut ChildEnumContext) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool()
        && let Ok(rect) = window_rect(hwnd)
        && rect_contains_point(&rect, context.point)
        && rect_area(&rect) > 0
    {
        context.candidates.push(WindowCandidate {
            hwnd,
            rect,
            class_name: window_class_name(hwnd),
        });
    }
    BOOL(1)
}

#[cfg(windows)]
fn post_mouse_message(
    hwnd: i64,
    message: u32,
    wparam: usize,
    client_point: Point,
) -> Result<(), ActionError> {
    let hwnd = hwnd_from_i64(hwnd)?;
    let lparam = mouse_lparam(client_point)?;
    unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), lparam) }.map_err(|error| {
        ActionError::BackendUnavailable {
            detail: format!(
                "PostMessageW act_click mouse message 0x{message:x} failed for hwnd 0x{:x} client_point={client_point:?}: {error}",
                hwnd_to_i64(hwnd)
            ),
        }
    })
}

#[cfg(windows)]
fn mouse_button_messages(button: MouseButton) -> Result<(u32, u32, usize), ActionError> {
    match button {
        MouseButton::Left => Ok((WM_LBUTTONDOWN, WM_LBUTTONUP, 0x0001)),
        MouseButton::Right => Ok((WM_RBUTTONDOWN, WM_RBUTTONUP, 0x0002)),
        MouseButton::Middle => Ok((WM_MBUTTONDOWN, WM_MBUTTONUP, 0x0010)),
        MouseButton::X1 | MouseButton::X2 => Err(ActionError::BackendUnavailable {
            detail: format!(
                "act_click HWND message fallback supports left/right/middle buttons only, got {button:?}"
            ),
        }),
    }
}

#[cfg(windows)]
fn mouse_lparam(client_point: Point) -> Result<LPARAM, ActionError> {
    let x = i16::try_from(client_point.x).map_err(|error| ActionError::TargetInvalid {
        detail: format!(
            "act_click client x {} cannot fit a WM_* mouse lParam i16: {error}",
            client_point.x
        ),
    })?;
    let y = i16::try_from(client_point.y).map_err(|error| ActionError::TargetInvalid {
        detail: format!(
            "act_click client y {} cannot fit a WM_* mouse lParam i16: {error}",
            client_point.y
        ),
    })?;
    let packed = (u32::from(u16::from_ne_bytes(y.to_ne_bytes())) << 16)
        | u32::from(u16::from_ne_bytes(x.to_ne_bytes()));
    Ok(LPARAM(isize::try_from(packed).unwrap_or(isize::MAX)))
}

#[cfg(windows)]
fn window_rect(hwnd: HWND) -> Result<RECT, ActionError> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.map_err(|error| {
        ActionError::ElementNotResolved {
            detail: format!(
                "GetWindowRect failed for act_click hwnd 0x{:x}: {error}",
                hwnd_to_i64(hwnd)
            ),
        }
    })?;
    Ok(rect)
}

#[cfg(windows)]
fn rect_contains_point(rect: &RECT, point: Point) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

#[cfg(windows)]
fn rect_area(rect: &RECT) -> i64 {
    let width = i64::from(rect.right.saturating_sub(rect.left).max(0));
    let height = i64::from(rect.bottom.saturating_sub(rect.top).max(0));
    width.saturating_mul(height)
}

#[cfg(windows)]
fn rect_tuple(rect: &RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

#[cfg(windows)]
fn window_class_name(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
}

#[cfg(windows)]
fn hwnd_from_i64(hwnd: i64) -> Result<HWND, ActionError> {
    if hwnd == 0 {
        return Err(ActionError::TargetInvalid {
            detail: "act_click element root hwnd is null".to_owned(),
        });
    }
    Ok(HWND(hwnd as isize as *mut c_void))
}

#[cfg(windows)]
fn hwnd_to_i64(hwnd: HWND) -> i64 {
    hwnd.0 as isize as i64
}

#[cfg(test)]
mod tests {
    use synapse_core::{AimCurve, AimNaturalParams, Backend, ButtonAction, MouseButton};

    use super::*;
    use crate::m2::click::schema::{
        ActClickTarget, ClickVelocityProfile, default_click_button, default_click_duration_ms,
        default_click_hold_ms,
    };

    #[test]
    fn direct_coordinate_element_click_uses_move_then_requested_presses() {
        let params = ActClickParams {
            target: ActClickTarget::Element(ActClickElementTarget {
                element_id: synapse_core::ElementId::parse("0x1000:0000002a00000001")
                    .expect("synthetic element id must be valid"),
            }),
            button: default_click_button(),
            clicks: 2,
            modifiers: Vec::new(),
            velocity_profile: ClickVelocityProfile::Natural,
            duration_ms: default_click_duration_ms(),
            hold_ms: default_click_hold_ms(),
            backend: Backend::Software,
            use_invoke_pattern: false,
            deprecated_curve_alias_used: false,
        };
        let screen_point = Point { x: 320, y: 240 };
        let before = "screen_point=(320,240), clicks=2";

        let after = coordinate_click_actions(&params, screen_point);

        assert_eq!(after.len(), 3);
        assert!(matches!(
            after[0],
            Action::MouseMove {
                to: MouseTarget::Screen {
                    point: Point { x: 320, y: 240 }
                },
                curve: AimCurve::Natural {
                    params: AimNaturalParams::FAST
                },
                duration_ms: 50,
                backend: Backend::Software,
            }
        ));
        for action in &after[1..] {
            assert!(matches!(
                action,
                Action::MouseButton {
                    button: MouseButton::Left,
                    action: ButtonAction::Press,
                    hold_ms: 120,
                    backend: Backend::Software,
                }
            ));
        }
        println!("readback=act_click_element_coordinate_direct before={before} after={after:?}");
    }
}
