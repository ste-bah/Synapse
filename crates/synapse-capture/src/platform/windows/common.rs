use std::ffi::c_void;

use windows::Win32::Foundation::HWND;

use crate::CaptureError;

pub(super) fn capture_unsupported<E: std::fmt::Display>(err: E) -> CaptureError {
    CaptureError::GraphicsApiUnsupported {
        detail: err.to_string(),
    }
}

pub(super) fn hwnd_from_i64(hwnd: i64) -> Result<HWND, CaptureError> {
    let native = synapse_core::win32_hwnd::hwnd_from_wire(hwnd).ok_or_else(|| {
        CaptureError::TargetInvalid {
            detail: format!(
                "HWND wire value {hwnd} is outside the canonical Win32 USER-handle range 1..=4294967295"
            ),
        }
    })?;
    Ok(HWND(native as *mut c_void))
}
