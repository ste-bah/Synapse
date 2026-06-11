use std::{cell::RefCell, ffi::c_void, mem::size_of, slice, time::Duration};

use synapse_core::Rect;
use windows::{
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Storage::Streams::DataWriter,
    Win32::Graphics::{
        Direct3D11::{
            D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
            D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11Resource, ID3D11Texture2D,
        },
        Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
            SRCCOPY, SelectObject,
        },
    },
    Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
    Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
    Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetMenu, GetWindowLongW, GetWindowPlacement,
        GetWindowRect, IsIconic, PW_RENDERFULLCONTENT, WINDOW_EX_STYLE, WINDOW_STYLE,
        WINDOWPLACEMENT,
    },
    core::Interface as _,
};

use crate::{
    CaptureBackendPreference, CaptureConfig, CaptureError, CaptureTarget, CapturedBgraBitmap,
    CapturedFrame, CapturedSoftwareBitmap, CapturedWindowBgraBitmap, DxgiFormat,
    spawn_capture_loop,
};

use super::common::{capture_unsupported, hwnd_from_i64};

thread_local! {
    static SCREEN_CAPTURE_SCRATCH: RefCell<Option<GdiCaptureScratch>> = const { RefCell::new(None) };
}

pub fn captured_frame_region_to_software_bitmap(
    frame: &CapturedFrame,
    region: Rect,
) -> Result<CapturedSoftwareBitmap, CaptureError> {
    let region = clamp_region_to_frame(frame, region)?;
    let bytes = copy_region_bgra(frame, region)?;
    let bitmap = software_bitmap_from_bgra(&bytes, region.w, region.h)?;
    Ok(CapturedSoftwareBitmap { region, bitmap })
}

pub fn captured_frame_region_to_bgra_bitmap(
    frame: &CapturedFrame,
    region: Rect,
) -> Result<CapturedBgraBitmap, CaptureError> {
    let region = clamp_region_to_frame(frame, region)?;
    let bytes = copy_region_bgra(frame, region)?;
    Ok(CapturedBgraBitmap {
        region,
        width: u32::try_from(region.w).unwrap_or_default(),
        height: u32::try_from(region.h).unwrap_or_default(),
        bytes,
    })
}

pub fn screen_region_to_software_bitmap(
    region: Rect,
) -> Result<CapturedSoftwareBitmap, CaptureError> {
    validate_bitmap_region(region)?;
    let bytes = copy_screen_region_bgra(region)?;
    let bitmap = software_bitmap_from_bgra(&bytes, region.w, region.h)?;
    Ok(CapturedSoftwareBitmap { region, bitmap })
}

pub fn screen_region_to_bgra_bitmap(region: Rect) -> Result<CapturedBgraBitmap, CaptureError> {
    validate_bitmap_region(region)?;
    let bytes = copy_screen_region_bgra(region)?;
    Ok(CapturedBgraBitmap {
        region,
        width: u32::try_from(region.w).unwrap_or_default(),
        height: u32::try_from(region.h).unwrap_or_default(),
        bytes,
    })
}

pub fn window_region_to_bgra_bitmap(
    hwnd: i64,
    region: Rect,
    timeout_ms: u64,
) -> Result<CapturedWindowBgraBitmap, CaptureError> {
    validate_bitmap_region(region)?;
    match graphics_capture_window_region_to_bgra_bitmap(hwnd, region, timeout_ms) {
        Ok(bitmap) if !is_all_zero_bgra(&bitmap.bytes) => Ok(CapturedWindowBgraBitmap {
            bitmap,
            capture_backend: "graphics_capture_window_bgra",
        }),
        Ok(_bitmap) => {
            tracing::error!(
                code = "CAPTURE_WGC_WINDOW_ALL_ZERO_PRINTWINDOW_DISABLED",
                hwnd,
                region = ?region,
                "WGC window capture returned all-zero pixels; refusing target-reentering PrintWindow fallback"
            );
            Err(CaptureError::PrintWindowDisabled {
                detail: format!(
                    "WGC window capture for hwnd {hwnd:#x} region {region:?} returned all-zero pixels; PrintWindow fallback is disabled because Windows asks the target process to handle WM_PRINT/WM_PRINTCLIENT and can surface app-visible failures"
                ),
            })
        }
        Err(wgc_error) => {
            tracing::error!(
                code = "CAPTURE_WGC_WINDOW_FAILED_PRINTWINDOW_DISABLED",
                hwnd,
                region = ?region,
                error = %wgc_error,
                "WGC window capture failed; refusing target-reentering PrintWindow fallback"
            );
            Err(CaptureError::PrintWindowDisabled {
                detail: format!(
                    "WGC window capture failed for hwnd {hwnd:#x} region {region:?}: {wgc_error}; PrintWindow fallback is disabled because Windows asks the target process to handle WM_PRINT/WM_PRINTCLIENT and can surface app-visible failures"
                ),
            })
        }
    }
}

pub fn window_region_to_bgra_bitmap_printwindow(
    hwnd: i64,
    region: Rect,
) -> Result<CapturedWindowBgraBitmap, CaptureError> {
    validate_bitmap_region(region)?;
    let hwnd_value = hwnd;
    let hwnd = hwnd_from_i64(hwnd);
    let (window_width, window_height) = window_capture_extent(hwnd)?;
    validate_region_inside_window(region, window_width, window_height)?;
    let bytes = printwindow_region_bgra(hwnd, hwnd_value, region, window_width, window_height)?;
    if is_all_zero_bgra(&bytes) {
        tracing::warn!(
            code = synapse_core::error_codes::CAPTURE_PRINTWINDOW_BLACK,
            hwnd = hwnd_value,
            region = ?region,
            "PrintWindow returned all-zero pixels"
        );
        return Err(CaptureError::PrintWindowBlack {
            detail: format!(
                "PrintWindow returned all-zero pixels for hwnd {hwnd_value:#x} region {region:?}; target likely does not render through WM_PRINT/WM_PRINTCLIENT"
            ),
        });
    }
    Ok(CapturedWindowBgraBitmap {
        bitmap: CapturedBgraBitmap {
            region,
            width: u32::try_from(region.w).unwrap_or_default(),
            height: u32::try_from(region.h).unwrap_or_default(),
            bytes,
        },
        capture_backend: "printwindow",
    })
}

pub fn window_capture_region(hwnd: i64) -> Result<Rect, CaptureError> {
    let hwnd = hwnd_from_i64(hwnd);
    let (w, h) = window_capture_extent(hwnd)?;
    let region = Rect { x: 0, y: 0, w, h };
    validate_bitmap_region(region)?;
    Ok(region)
}

pub fn client_region_to_window_region(hwnd: i64, region: Rect) -> Result<Rect, CaptureError> {
    validate_bitmap_region(region)?;
    let hwnd = hwnd_from_i64(hwnd);

    if unsafe { IsIconic(hwnd) }.as_bool() {
        let (window_width, window_height) = window_capture_extent(hwnd)?;
        let Some((offset_x, offset_y)) =
            minimized_client_offset_in_window_bitmap(hwnd, window_width, window_height)?
        else {
            return Err(CaptureError::TargetInvalid {
                detail: "minimized target has no client extent for region conversion".to_owned(),
            });
        };
        let window_region = Rect {
            x: region.x.saturating_add(offset_x),
            y: region.y.saturating_add(offset_y),
            w: region.w,
            h: region.h,
        };
        validate_region_inside_window(window_region, window_width, window_height)?;
        return Ok(window_region);
    }

    let mut window_rect = windows::Win32::Foundation::RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut window_rect) }.map_err(capture_unsupported)?;
    let window_width = window_rect.right.saturating_sub(window_rect.left);
    let window_height = window_rect.bottom.saturating_sub(window_rect.top);
    let mut client_origin = windows::Win32::Foundation::POINT { x: 0, y: 0 };
    if !unsafe { windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &raw mut client_origin) }
        .as_bool()
    {
        return Err(CaptureError::TargetInvalid {
            detail: "ClientToScreen failed while converting screenshot region".to_owned(),
        });
    }
    let offset_x = client_origin.x.saturating_sub(window_rect.left);
    let offset_y = client_origin.y.saturating_sub(window_rect.top);
    let window_region = Rect {
        x: region.x.saturating_add(offset_x),
        y: region.y.saturating_add(offset_y),
        w: region.w,
        h: region.h,
    };
    validate_region_inside_window(window_region, window_width, window_height)?;
    Ok(window_region)
}

fn graphics_capture_window_region_to_bgra_bitmap(
    hwnd: i64,
    region: Rect,
    timeout_ms: u64,
) -> Result<CapturedBgraBitmap, CaptureError> {
    let timeout = Duration::from_millis(timeout_ms.max(1));
    let handle = spawn_capture_loop(CaptureConfig {
        target: CaptureTarget::Window { hwnd },
        min_update_interval_ms: 16,
        cursor_visible: false,
        secondary_windows: false,
        dirty_region_only: false,
        backend_preference: CaptureBackendPreference::GraphicsCaptureApi,
    })?;
    let receiver = handle.receiver();
    let frame = match receiver.recv_timeout(timeout) {
        Ok(frame) => frame,
        Err(error) => {
            let stop_result = handle.stop();
            return Err(match stop_result {
                Ok(()) => CaptureError::ThreadFailed {
                    detail: format!(
                        "timed out after {timeout_ms} ms waiting for WGC window frame: {error}"
                    ),
                },
                Err(stop_error) => stop_error,
            });
        }
    };
    let result = captured_frame_region_to_bgra_bitmap(&frame, region);
    let stop_result = handle.stop();
    match (result, stop_result) {
        (Ok(bitmap), Ok(())) => Ok(bitmap),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

fn minimized_client_offset_in_window_bitmap(
    hwnd: windows::Win32::Foundation::HWND,
    full_width: i32,
    full_height: i32,
) -> Result<Option<(i32, i32)>, CaptureError> {
    let mut client_rect = windows::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(hwnd, &raw mut client_rect) }.map_err(capture_unsupported)?;
    let client_width = client_rect.right.saturating_sub(client_rect.left);
    let client_height = client_rect.bottom.saturating_sub(client_rect.top);
    if client_width <= 0 || client_height <= 0 {
        return minimized_non_client_offset_from_style(hwnd);
    }

    let non_client_x = full_width.saturating_sub(client_width).max(0);
    let non_client_y = full_height.saturating_sub(client_height).max(0);
    let offset_x = non_client_x / 2;
    let offset_y = non_client_y.saturating_sub(offset_x).max(0);
    Ok(Some((offset_x, offset_y)))
}

fn minimized_non_client_offset_from_style(
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<Option<(i32, i32)>, CaptureError> {
    let style_bits = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
    let ex_style_bits = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
    let menu = unsafe { GetMenu(hwnd) };
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let mut client_rect = windows::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: 100,
        bottom: 100,
    };
    unsafe {
        AdjustWindowRectExForDpi(
            &raw mut client_rect,
            WINDOW_STYLE(style_bits as u32),
            !menu.is_invalid(),
            WINDOW_EX_STYLE(ex_style_bits as u32),
            dpi,
        )
    }
    .map_err(capture_unsupported)?;
    Ok(Some((
        client_rect.left.saturating_neg().max(0),
        client_rect.top.saturating_neg().max(0),
    )))
}

fn window_capture_extent(
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<(i32, i32), CaptureError> {
    let mut window_rect = windows::Win32::Foundation::RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut window_rect) }.map_err(capture_unsupported)?;
    let window_width = window_rect.right.saturating_sub(window_rect.left);
    let window_height = window_rect.bottom.saturating_sub(window_rect.top);
    if !unsafe { IsIconic(hwnd) }.as_bool() {
        return Ok((window_width, window_height));
    }

    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(hwnd, &raw mut placement) }.map_err(capture_unsupported)?;
    let normal_width = placement
        .rcNormalPosition
        .right
        .saturating_sub(placement.rcNormalPosition.left);
    let normal_height = placement
        .rcNormalPosition
        .bottom
        .saturating_sub(placement.rcNormalPosition.top);
    if normal_width > 0 && normal_height > 0 {
        return Ok((normal_width, normal_height));
    }
    Ok((window_width, window_height))
}

fn software_bitmap_from_bgra(
    bytes: &[u8],
    width: i32,
    height: i32,
) -> Result<SoftwareBitmap, CaptureError> {
    let writer = DataWriter::new().map_err(capture_unsupported)?;
    writer.WriteBytes(bytes).map_err(capture_unsupported)?;
    let buffer = writer.DetachBuffer().map_err(capture_unsupported)?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width,
        height,
        BitmapAlphaMode::Ignore,
    )
    .map_err(capture_unsupported)
}
fn copy_region_bgra(frame: &CapturedFrame, region: Rect) -> Result<Vec<u8>, CaptureError> {
    let convert_rgba_to_bgra = match frame.format {
        DxgiFormat::Bgra8 | DxgiFormat::Bgra8Srgb => false,
        DxgiFormat::Rgba8 | DxgiFormat::Rgba8Srgb => true,
        other => {
            return Err(CaptureError::GraphicsApiUnsupported {
                detail: format!("OCR bitmap copy does not support frame format {other:?}"),
            });
        }
    };

    let width = u32::try_from(region.w).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let height = u32::try_from(region.h).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let texture = frame.texture.get();
    let texture_desc = texture_desc(texture);
    validate_region_inside_texture(region, texture_desc.Width, texture_desc.Height)?;
    let staging = create_staging_texture(texture, width, height)?;
    let context = unsafe { texture.GetDevice() }
        .and_then(|device| unsafe { device.GetImmediateContext() })
        .map_err(capture_unsupported)?;
    let source: ID3D11Resource = texture.cast().map_err(capture_unsupported)?;
    let target: ID3D11Resource = staging.cast().map_err(capture_unsupported)?;
    let source_left = u32::try_from(region.x).map_err(|err| CaptureError::TargetInvalid {
        detail: format!("invalid source region x {}: {err}", region.x),
    })?;
    let source_top = u32::try_from(region.y).map_err(|err| CaptureError::TargetInvalid {
        detail: format!("invalid source region y {}: {err}", region.y),
    })?;
    let source_right = u32::try_from(region.x.saturating_add(region.w)).map_err(|err| {
        CaptureError::TargetInvalid {
            detail: format!("invalid source region right edge for {region:?}: {err}"),
        }
    })?;
    let source_bottom = u32::try_from(region.y.saturating_add(region.h)).map_err(|err| {
        CaptureError::TargetInvalid {
            detail: format!("invalid source region bottom edge for {region:?}: {err}"),
        }
    })?;
    let source_box = D3D11_BOX {
        left: source_left,
        top: source_top,
        front: 0,
        right: source_right,
        bottom: source_bottom,
        back: 1,
    };
    unsafe {
        context.CopySubresourceRegion(&target, 0, 0, 0, 0, &source, 0, Some(&raw const source_box));
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&target, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }
        .map_err(capture_unsupported)?;
    let bytes = copy_mapped_rows(&mapped, width, height, convert_rgba_to_bgra);
    unsafe {
        context.Unmap(&target, 0);
    }
    bytes
}

fn texture_desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        texture.GetDesc(&raw mut desc);
    }
    desc
}

fn create_staging_texture(
    texture: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, CaptureError> {
    let mut desc = texture_desc(texture);
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
    desc.MiscFlags = 0;
    desc.SampleDesc.Count = 1;
    desc.SampleDesc.Quality = 0;

    let device = unsafe { texture.GetDevice() }.map_err(capture_unsupported)?;
    let mut staging = None;
    unsafe { device.CreateTexture2D(&raw const desc, None, Some(&raw mut staging)) }
        .map_err(capture_unsupported)?;
    staging.ok_or_else(|| CaptureError::GraphicsApiUnsupported {
        detail: "CreateTexture2D returned no staging texture".to_owned(),
    })
}

fn copy_mapped_rows(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    width: u32,
    height: u32,
    convert_rgba_to_bgra: bool,
) -> Result<Vec<u8>, CaptureError> {
    let row_len = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid OCR bitmap width {width}"),
        })?;
    let height = usize::try_from(height).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let byte_len = row_len
        .checked_mul(height)
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid OCR bitmap dimensions {width}x{height}"),
        })?;
    let row_pitch =
        usize::try_from(mapped.RowPitch).map_err(|err| CaptureError::GraphicsApiUnsupported {
            detail: err.to_string(),
        })?;
    if mapped.pData.is_null() {
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: "D3D11 Map returned null pData for BGRA readback".to_owned(),
        });
    }
    if row_pitch < row_len {
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: format!(
                "D3D11 Map returned row pitch {row_pitch} smaller than row byte length {row_len}"
            ),
        });
    }
    let mut output = vec![0_u8; byte_len];
    let base = mapped.pData.cast::<u8>();
    for row in 0..height {
        let source_offset =
            row.checked_mul(row_pitch)
                .ok_or_else(|| CaptureError::GraphicsApiUnsupported {
                    detail: format!(
                        "D3D11 mapped row offset overflow for row {row}, pitch {row_pitch}"
                    ),
                })?;
        let source =
            unsafe { slice::from_raw_parts(base.add(source_offset).cast_const(), row_len) };
        let start = row.saturating_mul(row_len);
        output[start..start + row_len].copy_from_slice(source);
    }
    if convert_rgba_to_bgra {
        for pixel in output.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(output)
}

fn validate_region_inside_texture(
    region: Rect,
    texture_width: u32,
    texture_height: u32,
) -> Result<(), CaptureError> {
    validate_bitmap_region(region)?;
    if region.x < 0 || region.y < 0 {
        return Err(CaptureError::TargetInvalid {
            detail: format!("source region {region:?} has negative coordinates"),
        });
    }
    let right = u32::try_from(region.x.saturating_add(region.w)).map_err(|err| {
        CaptureError::TargetInvalid {
            detail: format!("invalid source region right edge for {region:?}: {err}"),
        }
    })?;
    let bottom = u32::try_from(region.y.saturating_add(region.h)).map_err(|err| {
        CaptureError::TargetInvalid {
            detail: format!("invalid source region bottom edge for {region:?}: {err}"),
        }
    })?;
    if right > texture_width || bottom > texture_height {
        return Err(CaptureError::TargetInvalid {
            detail: format!(
                "source region {region:?} exceeds D3D texture bounds {texture_width}x{texture_height}"
            ),
        });
    }
    Ok(())
}

fn copy_screen_region_bgra(region: Rect) -> Result<Vec<u8>, CaptureError> {
    let width = u32::try_from(region.w).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let height = u32::try_from(region.h).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let byte_len = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid screen capture region {region:?}"),
        })?;
    let screen_dc = unsafe { GetDC(None) };
    if screen_dc.is_invalid() {
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: "GetDC returned null".to_owned(),
        });
    }
    let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
    if memory_dc.is_invalid() {
        let _ = unsafe { ReleaseDC(None, screen_dc) };
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: "CreateCompatibleDC returned null".to_owned(),
        });
    }
    let result = SCREEN_CAPTURE_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let needs_recreate = scratch
            .as_ref()
            .is_none_or(|scratch| !scratch.matches(width, height, byte_len));
        if needs_recreate {
            *scratch = Some(GdiCaptureScratch::new(
                screen_dc, memory_dc, width, height, byte_len,
            )?);
        } else {
            let _ = unsafe { DeleteDC(memory_dc) };
        }
        let scratch = scratch
            .as_ref()
            .ok_or_else(|| CaptureError::GraphicsApiUnsupported {
                detail: "screen capture scratch buffer was not initialized".to_owned(),
            })?;
        let bitblt = unsafe {
            BitBlt(
                scratch.memory_dc,
                0,
                0,
                region.w,
                region.h,
                Some(screen_dc),
                region.x,
                region.y,
                SRCCOPY,
            )
        };
        bitblt.map_err(capture_unsupported)?;
        Ok(unsafe { slice::from_raw_parts(scratch.bits.cast::<u8>(), byte_len) }.to_vec())
    });
    let _ = unsafe { ReleaseDC(None, screen_dc) };
    result
}

fn validate_region_inside_window(
    region: Rect,
    window_width: i32,
    window_height: i32,
) -> Result<(), CaptureError> {
    validate_bitmap_region(region)?;
    if region.x < 0
        || region.y < 0
        || region.x.saturating_add(region.w) > window_width
        || region.y.saturating_add(region.h) > window_height
    {
        return Err(CaptureError::TargetInvalid {
            detail: format!(
                "window capture region {region:?} is outside window bitmap bounds {window_width}x{window_height}"
            ),
        });
    }
    Ok(())
}

fn printwindow_region_bgra(
    hwnd: windows::Win32::Foundation::HWND,
    hwnd_value: i64,
    region: Rect,
    window_width: i32,
    window_height: i32,
) -> Result<Vec<u8>, CaptureError> {
    let full_width = u32::try_from(window_width).map_err(|err| CaptureError::TargetInvalid {
        detail: format!("invalid PrintWindow bitmap width {window_width}: {err}"),
    })?;
    let full_height = u32::try_from(window_height).map_err(|err| CaptureError::TargetInvalid {
        detail: format!("invalid PrintWindow bitmap height {window_height}: {err}"),
    })?;
    let full_byte_len = usize::try_from(full_width)
        .ok()
        .and_then(|w| {
            usize::try_from(full_height)
                .ok()
                .and_then(|h| w.checked_mul(h))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid PrintWindow bitmap dimensions {window_width}x{window_height}"),
        })?;
    let window_dc = unsafe { GetDC(Some(hwnd)) };
    if window_dc.is_invalid() {
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: format!("GetDC returned null for hwnd {hwnd_value:#x}"),
        });
    }
    let memory_dc = unsafe { CreateCompatibleDC(Some(window_dc)) };
    if memory_dc.is_invalid() {
        let _ = unsafe { ReleaseDC(Some(hwnd), window_dc) };
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: format!("CreateCompatibleDC returned null for hwnd {hwnd_value:#x}"),
        });
    }
    let scratch = match GdiCaptureScratch::new(
        window_dc,
        memory_dc,
        full_width,
        full_height,
        full_byte_len,
    ) {
        Ok(scratch) => scratch,
        Err(error) => {
            let _ = unsafe { ReleaseDC(Some(hwnd), window_dc) };
            return Err(error);
        }
    };
    let printed = unsafe {
        PrintWindow(
            hwnd,
            scratch.memory_dc,
            PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT),
        )
    };
    let _ = unsafe { ReleaseDC(Some(hwnd), window_dc) };
    if !printed.as_bool() {
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: format!(
                "PrintWindow returned false for hwnd {hwnd_value:#x}; last_error={:?}",
                unsafe { windows::Win32::Foundation::GetLastError() }
            ),
        });
    }
    let full_bytes =
        unsafe { slice::from_raw_parts(scratch.bits.cast::<u8>(), full_byte_len) }.to_vec();
    copy_bgra_region_from_bytes(&full_bytes, full_width, region)
}

fn copy_bgra_region_from_bytes(
    full_bytes: &[u8],
    full_width: u32,
    region: Rect,
) -> Result<Vec<u8>, CaptureError> {
    let width = usize::try_from(region.w).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let height = usize::try_from(region.h).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let full_row_len = usize::try_from(full_width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid PrintWindow full width {full_width}"),
        })?;
    let row_len = width
        .checked_mul(4)
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid PrintWindow crop width {}", region.w),
        })?;
    let byte_len = row_len
        .checked_mul(height)
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid PrintWindow crop region {region:?}"),
        })?;
    let source_x = usize::try_from(region.x).map_err(|err| CaptureError::TargetInvalid {
        detail: format!("invalid PrintWindow crop x {}: {err}", region.x),
    })?;
    let source_y = usize::try_from(region.y).map_err(|err| CaptureError::TargetInvalid {
        detail: format!("invalid PrintWindow crop y {}: {err}", region.y),
    })?;
    let source_x_bytes = source_x
        .checked_mul(4)
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid PrintWindow crop x {}", region.x),
        })?;
    let mut output = vec![0_u8; byte_len];
    for row in 0..height {
        let source_offset = source_y
            .checked_add(row)
            .and_then(|source_row| source_row.checked_mul(full_row_len))
            .and_then(|source_row_offset| source_row_offset.checked_add(source_x_bytes))
            .ok_or_else(|| CaptureError::TargetInvalid {
                detail: format!("PrintWindow source offset overflow for {region:?}"),
            })?;
        let source_end =
            source_offset
                .checked_add(row_len)
                .ok_or_else(|| CaptureError::TargetInvalid {
                    detail: format!("PrintWindow source end overflow for {region:?}"),
                })?;
        if source_end > full_bytes.len() {
            return Err(CaptureError::TargetInvalid {
                detail: format!(
                    "PrintWindow source region {region:?} exceeds captured byte length {}",
                    full_bytes.len()
                ),
            });
        }
        let target_offset = row.saturating_mul(row_len);
        output[target_offset..target_offset + row_len]
            .copy_from_slice(&full_bytes[source_offset..source_end]);
    }
    Ok(output)
}

fn is_all_zero_bgra(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0)
}

struct GdiCaptureScratch {
    width: u32,
    height: u32,
    byte_len: usize,
    memory_dc: HDC,
    bitmap: HBITMAP,
    old_object: HGDIOBJ,
    bits: *mut c_void,
}

impl GdiCaptureScratch {
    fn new(
        screen_dc: HDC,
        memory_dc: HDC,
        width: u32,
        height: u32,
        byte_len: usize,
    ) -> Result<Self, CaptureError> {
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(u32::MAX),
                biWidth: i32::try_from(width).unwrap_or(i32::MAX),
                biHeight: -i32::try_from(height).unwrap_or(i32::MAX),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: u32::try_from(byte_len).unwrap_or(u32::MAX),
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = unsafe {
            CreateDIBSection(
                Some(screen_dc),
                &raw const bitmap_info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
        }
        .map_err(capture_unsupported)?;
        if bits.is_null() {
            let _ = unsafe { DeleteObject(HGDIOBJ::from(bitmap)) };
            let _ = unsafe { DeleteDC(memory_dc) };
            return Err(CaptureError::GraphicsApiUnsupported {
                detail: "CreateDIBSection returned no bitmap bits".to_owned(),
            });
        }
        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        if old_object.is_invalid() {
            let _ = unsafe { DeleteObject(HGDIOBJ::from(bitmap)) };
            let _ = unsafe { DeleteDC(memory_dc) };
            return Err(CaptureError::GraphicsApiUnsupported {
                detail: "SelectObject failed for screen capture bitmap".to_owned(),
            });
        }
        Ok(Self {
            width,
            height,
            byte_len,
            memory_dc,
            bitmap,
            old_object,
            bits,
        })
    }

    const fn matches(&self, width: u32, height: u32, byte_len: usize) -> bool {
        self.width == width && self.height == height && self.byte_len == byte_len
    }
}

impl Drop for GdiCaptureScratch {
    fn drop(&mut self) {
        let _ = unsafe { SelectObject(self.memory_dc, self.old_object) };
        let _ = unsafe { DeleteObject(HGDIOBJ::from(self.bitmap)) };
        let _ = unsafe { DeleteDC(self.memory_dc) };
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_void;

    use windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE;

    use super::{copy_bgra_region_from_bytes, copy_mapped_rows, validate_region_inside_texture};
    use crate::CaptureError;
    use synapse_core::Rect;

    #[test]
    fn mapped_row_copy_rejects_null_pointer() {
        let mapped = D3D11_MAPPED_SUBRESOURCE {
            pData: std::ptr::null_mut(),
            RowPitch: 16,
            DepthPitch: 16,
        };

        let error = copy_mapped_rows(&mapped, 4, 1, false).unwrap_err();

        assert!(matches!(error, CaptureError::GraphicsApiUnsupported { .. }));
        assert!(error.to_string().contains("null pData"));
    }

    #[test]
    fn mapped_row_copy_rejects_short_pitch() {
        let mut bytes = [0_u8; 8];
        let mapped = D3D11_MAPPED_SUBRESOURCE {
            pData: bytes.as_mut_ptr().cast::<c_void>(),
            RowPitch: 3,
            DepthPitch: 8,
        };

        let error = copy_mapped_rows(&mapped, 1, 1, false).unwrap_err();

        assert!(matches!(error, CaptureError::GraphicsApiUnsupported { .. }));
        assert!(error.to_string().contains("row pitch 3"));
    }

    #[test]
    fn mapped_row_copy_honors_pitch_padding_and_bgra_order() {
        let mut bytes = [1_u8, 2, 3, 4, 99, 99, 5, 6, 7, 8, 88, 88];
        let mapped = D3D11_MAPPED_SUBRESOURCE {
            pData: bytes.as_mut_ptr().cast::<c_void>(),
            RowPitch: 6,
            DepthPitch: 12,
        };

        let output = copy_mapped_rows(&mapped, 1, 2, false).unwrap();

        assert_eq!(output, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn texture_region_validation_rejects_source_box_overflow() {
        let error = validate_region_inside_texture(
            Rect {
                x: 8,
                y: 31,
                w: 940,
                h: 330,
            },
            940,
            330,
        )
        .unwrap_err();

        assert!(matches!(error, CaptureError::TargetInvalid { .. }));
        assert!(error.to_string().contains("exceeds D3D texture bounds"));
    }

    #[test]
    fn printwindow_region_copy_extracts_bgra_crop() {
        let full = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, //
            13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        ];

        let crop = copy_bgra_region_from_bytes(
            &full,
            3,
            Rect {
                x: 1,
                y: 0,
                w: 2,
                h: 2,
            },
        )
        .expect("crop succeeds");

        assert_eq!(
            crop,
            vec![
                5, 6, 7, 8, 9, 10, 11, 12, //
                17, 18, 19, 20, 21, 22, 23, 24,
            ]
        );
    }
}

fn validate_bitmap_region(region: Rect) -> Result<(), CaptureError> {
    if region.w <= 0 || region.h <= 0 {
        return Err(CaptureError::TargetInvalid {
            detail: format!("empty bitmap capture region {region:?}"),
        });
    }
    Ok(())
}

fn clamp_region_to_frame(frame: &CapturedFrame, region: Rect) -> Result<Rect, CaptureError> {
    if region.w <= 0 || region.h <= 0 {
        return Err(CaptureError::TargetInvalid {
            detail: format!("empty OCR capture region {region:?}"),
        });
    }
    let frame_w = i64::from(frame.width);
    let frame_h = i64::from(frame.height);
    let left = i64::from(region.x).clamp(0, frame_w);
    let top = i64::from(region.y).clamp(0, frame_h);
    let right = i64::from(region.x)
        .saturating_add(i64::from(region.w))
        .clamp(0, frame_w);
    let bottom = i64::from(region.y)
        .saturating_add(i64::from(region.h))
        .clamp(0, frame_h);
    if right <= left || bottom <= top {
        return Err(CaptureError::TargetInvalid {
            detail: format!("OCR capture region {region:?} is outside frame bounds"),
        });
    }
    Ok(Rect {
        x: i32::try_from(left).unwrap_or(i32::MAX),
        y: i32::try_from(top).unwrap_or(i32::MAX),
        w: i32::try_from(right - left).unwrap_or(i32::MAX),
        h: i32::try_from(bottom - top).unwrap_or(i32::MAX),
    })
}
