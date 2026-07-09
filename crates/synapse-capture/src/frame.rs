use std::time::Instant;

use synapse_core::Rect;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DxgiFormat {
    Bgra8,
    Bgra8Srgb,
    Rgba8,
    Rgba8Srgb,
    Rgba16F,
    Rgb10A2,
    Rgb10XrA2,
    Unknown(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedFrameBuffer {
    pub bytes: Vec<u8>,
    pub row_stride_bytes: usize,
    pub bytes_per_pixel: u8,
}

#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub pixels: CapturedFrameBuffer,
    pub width: u32,
    pub height: u32,
    pub format: DxgiFormat,
    pub captured_at: Instant,
    pub frame_seq: u64,
    pub dirty_region: Option<Rect>,
}

#[cfg(windows)]
pub struct CapturedSoftwareBitmap {
    pub region: Rect,
    pub bitmap: windows::Graphics::Imaging::SoftwareBitmap,
}

/// Raw BGRA pixels for a screen region.
///
/// This is a plain data struct (no Windows-specific types), so it is available
/// on every platform: non-Windows `screen_region_to_bgra_bitmap` returns
/// `Err(GraphicsApiUnsupported)` rather than a value, but the `synapse-mcp`
/// callers still reference the type at compile time. Contrast
/// `CapturedSoftwareBitmap`, which wraps a `WinRT` `SoftwareBitmap` and is
/// therefore Windows-only.
#[derive(Clone, Debug)]
pub struct CapturedBgraBitmap {
    pub region: Rect,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct CapturedWindowBgraBitmap {
    pub bitmap: CapturedBgraBitmap,
    pub capture_backend: &'static str,
    pub capture_attempts: u32,
    pub capture_retry_count: u32,
    pub capture_elapsed_ms: u64,
    pub capture_retry_backoff_ms: u64,
}

// Note: a `CapturedFrame::synthetic(..)` constructor used to exist here for
// non-Windows builds. It fabricated placeholder frames so the capture loop on
// Linux/macOS appeared to succeed while feeding mock pixels into perception. It
// was removed deliberately: non-Windows builds now fail loudly in
// `platform::non_windows` instead of producing fake frames. See
// `crates/synapse-capture/src/platform/non_windows.rs`.
