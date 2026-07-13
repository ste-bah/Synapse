use synapse_core::Point;
use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{ClientToScreen, ScreenToClient},
};

use crate::CaptureError;

use super::common::hwnd_from_i64;

pub fn screen_to_window(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    let hwnd = hwnd_from_i64(hwnd)?;
    let mut raw = POINT {
        x: point.x,
        y: point.y,
    };
    if unsafe { ScreenToClient(hwnd, std::ptr::addr_of_mut!(raw)) }.as_bool() {
        Ok(Point { x: raw.x, y: raw.y })
    } else {
        Err(CaptureError::TargetInvalid {
            detail: "ScreenToClient failed".to_owned(),
        })
    }
}

pub fn window_to_screen(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    let hwnd = hwnd_from_i64(hwnd)?;
    let mut raw = POINT {
        x: point.x,
        y: point.y,
    };
    if unsafe { ClientToScreen(hwnd, std::ptr::addr_of_mut!(raw)) }.as_bool() {
        Ok(Point { x: raw.x, y: raw.y })
    } else {
        Err(CaptureError::TargetInvalid {
            detail: "ClientToScreen failed".to_owned(),
        })
    }
}
