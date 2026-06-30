//! `capture_gif` MCP tool (#1339).
//!
//! Claude-in-Chrome ships `gif_creator` to record interactions as shareable
//! GIFs. Synapse had `demo_record_*` (UIA JSONL for profile authoring) and
//! terminal asciicast, but no VISUAL screen/browser GIF recorder.
//!
//! `capture_gif` records the bound window (or an explicit HWND, or the browser
//! window behind a CDP target) by capturing periodic passive per-window WGC
//! frames over a recording window, then encodes an animated GIF. It is a single
//! synchronous call — no dangling recording state machine — and reports captured
//! vs requested frame counts so there are never silent frame drops. WGC captures
//! occluded/background windows, so recording does not require foreground.

use std::time::{Duration, Instant};

use image::{
    Delay, Frame, RgbaImage,
    codecs::gif::{GifEncoder, Repeat},
};
use rmcp::{RoleServer, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ErrorData, Json, Parameters, SessionTarget, SynapseService, tool, tool_router};
use crate::m1::mcp_error;

const DEFAULT_DURATION_MS: u64 = 3_000;
const MAX_DURATION_MS: u64 = 60_000;
const DEFAULT_INTERVAL_MS: u64 = 500;
const MIN_INTERVAL_MS: u64 = 100;
const DEFAULT_MAX_LONG_EDGE: u32 = 800;
const FRAME_TIMEOUT_MS: u64 = 1_500;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureGifParams {
    /// Output `.gif` file path. Must be absolute.
    pub path: String,
    /// Total recording window in milliseconds (default 3000, capped at 60000).
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Delay between captured frames in milliseconds (default 500, min 100). Also
    /// the per-frame playback delay encoded into the GIF.
    #[serde(default)]
    pub interval_ms: Option<u64>,
    /// Window HWND to record. Defaults to this session's bound target window (the
    /// native window, or the browser window behind a CDP tab target).
    #[serde(default)]
    pub window_hwnd: Option<i64>,
    /// Downscale (aspect-preserving) so each frame's longest edge never exceeds
    /// this. Default 800; keeps the GIF small. Set 0 to disable.
    #[serde(default)]
    pub max_long_edge: Option<u32>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureGifResponse {
    pub path: String,
    pub frames_captured: usize,
    pub frames_requested: usize,
    pub width: u32,
    pub height: u32,
    pub native_width: u32,
    pub native_height: u32,
    pub interval_ms: u64,
    pub duration_ms: u64,
    pub elapsed_ms: u64,
    pub bytes_written: u64,
    pub capture_backend: String,
    pub window_hwnd: i64,
}

#[tool_router(router = capture_gif_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Record the bound window as an animated GIF (Claude gif_creator parity, #1339). Captures periodic passive per-window WGC frames of the session's bound target window — or an explicit window_hwnd, or the browser window behind a CDP tab target — over duration_ms at interval_ms, then encodes an animated GIF to an absolute .gif path. WGC captures occluded/background windows, so recording needs no foreground. Frames are downscaled aspect-preserving to max_long_edge (default 800). Single synchronous call (no recording state machine); reports frames_captured vs frames_requested so frame drops are never silent, and fails loud on zero frames or encode failure. Use capture_screenshot for a single still."
    )]
    pub async fn capture_gif(
        &self,
        params: Parameters<CaptureGifParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<CaptureGifResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "capture_gif",
            "tool.invocation kind=capture_gif"
        );

        let duration_ms = params
            .duration_ms
            .unwrap_or(DEFAULT_DURATION_MS)
            .clamp(MIN_INTERVAL_MS, MAX_DURATION_MS);
        let interval_ms = params
            .interval_ms
            .unwrap_or(DEFAULT_INTERVAL_MS)
            .max(MIN_INTERVAL_MS);
        let max_long_edge = params.max_long_edge.unwrap_or(DEFAULT_MAX_LONG_EDGE);

        let window_hwnd = self.capture_gif_resolve_window(params.window_hwnd, &request_context)?;

        let output_path = std::path::PathBuf::from(&params.path);
        if !output_path.is_absolute() {
            return Err(mcp_error(
                synapse_core::error_codes::TOOL_PARAMS_INVALID,
                format!("capture_gif path must be absolute: {:?}", params.path),
            ));
        }
        if output_path.exists() && !params.overwrite {
            return Err(mcp_error(
                synapse_core::error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "capture_gif refuses to overwrite existing file without overwrite=true: {:?}",
                    params.path
                ),
            ));
        }

        let frames_requested = usize::try_from(duration_ms / interval_ms)
            .unwrap_or(1)
            .max(1);
        let started = Instant::now();
        let mut native_dims: Option<(u32, u32)> = None;
        let mut target_dims: Option<(u32, u32)> = None;
        let mut frames: Vec<RgbaImage> = Vec::with_capacity(frames_requested);
        let mut capture_backend = "graphics_capture_window_bgra".to_owned();
        let mut last_error: Option<String> = None;

        while started.elapsed().as_millis() < u128::from(duration_ms) {
            let frame_started = Instant::now();
            match synapse_capture::window_full_frame_to_bgra_bitmap(window_hwnd, FRAME_TIMEOUT_MS) {
                Ok(captured) => {
                    capture_backend = captured.capture_backend.to_owned();
                    let bitmap = captured.bitmap;
                    if native_dims.is_none() {
                        native_dims = Some((bitmap.width, bitmap.height));
                        target_dims = Some(capture_gif_target_dims(
                            bitmap.width,
                            bitmap.height,
                            max_long_edge,
                        ));
                    }
                    let rgba = bgra_to_rgba(&bitmap.bytes, bitmap.width, bitmap.height)?;
                    let (tw, th) = target_dims.unwrap_or((bitmap.width, bitmap.height));
                    let frame = if (tw, th) == (bitmap.width, bitmap.height) {
                        rgba
                    } else {
                        image::imageops::resize(
                            &rgba,
                            tw,
                            th,
                            image::imageops::FilterType::Triangle,
                        )
                    };
                    frames.push(frame);
                }
                Err(error) => {
                    last_error = Some(error.to_string());
                }
            }
            // Pace to interval_ms, accounting for capture time.
            let spent = frame_started.elapsed();
            if let Some(remaining) = Duration::from_millis(interval_ms).checked_sub(spent) {
                tokio::time::sleep(remaining).await;
            }
        }

        if frames.is_empty() {
            return Err(mcp_error(
                synapse_core::error_codes::ACTION_NO_OBSERVED_DELTA,
                format!(
                    "capture_gif captured 0 frames of hwnd {window_hwnd:#x} in {duration_ms} ms; last capture error: {}",
                    last_error.unwrap_or_else(|| "none".to_owned())
                ),
            ));
        }

        let (tw, th) = target_dims.unwrap_or((1, 1));
        let (nw, nh) = native_dims.unwrap_or((tw, th));
        let frames_captured = frames.len();

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    format!("capture_gif could not create output directory: {error}"),
                )
            })?;
        }
        let file = std::fs::File::create(&output_path).map_err(|error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                format!("capture_gif could not create {:?}: {error}", params.path),
            )
        })?;
        let delay = Delay::from_numer_denom_ms(u32::try_from(interval_ms).unwrap_or(500), 1);
        {
            let mut encoder = GifEncoder::new_with_speed(file, 10);
            encoder.set_repeat(Repeat::Infinite).map_err(|error| {
                mcp_error(
                    synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                    format!("capture_gif set_repeat failed: {error}"),
                )
            })?;
            for rgba in frames {
                let frame = Frame::from_parts(rgba, 0, 0, delay);
                encoder.encode_frame(frame).map_err(|error| {
                    mcp_error(
                        synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                        format!("capture_gif GIF frame encode failed: {error}"),
                    )
                })?;
            }
        }

        let bytes_written = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(duration_ms);
        tracing::info!(
            code = "CAPTURE_GIF_RECORDED",
            hwnd = window_hwnd,
            frames_captured,
            frames_requested,
            width = tw,
            height = th,
            bytes_written,
            elapsed_ms,
            "readback=capture_gif outcome=encoded"
        );

        Ok(Json(CaptureGifResponse {
            path: output_path.to_string_lossy().into_owned(),
            frames_captured,
            frames_requested,
            width: tw,
            height: th,
            native_width: nw,
            native_height: nh,
            interval_ms,
            duration_ms,
            elapsed_ms,
            bytes_written,
            capture_backend,
            window_hwnd,
        }))
    }

    fn capture_gif_resolve_window(
        &self,
        explicit: Option<i64>,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<i64, ErrorData> {
        if let Some(hwnd) = explicit {
            return Ok(hwnd);
        }
        let session_id = super::context::mcp_session_id_from_request_context(request_context)?;
        let target = self.session_target(session_id.as_deref())?;
        match target {
            Some(SessionTarget::Window { hwnd }) => Ok(hwnd),
            Some(SessionTarget::Cdp { window_hwnd, .. }) => Ok(window_hwnd),
            None => Err(mcp_error(
                synapse_core::error_codes::TARGET_NOT_SET,
                "capture_gif requires a window_hwnd or a bound session target (set_target)",
            )),
        }
    }
}

fn capture_gif_target_dims(width: u32, height: u32, max_long_edge: u32) -> (u32, u32) {
    if max_long_edge == 0 || (width <= max_long_edge && height <= max_long_edge) {
        return (width.max(1), height.max(1));
    }
    let long = width.max(height);
    let scale = f64::from(max_long_edge) / f64::from(long);
    let tw = ((f64::from(width) * scale).round() as u32).max(1);
    let th = ((f64::from(height) * scale).round() as u32).max(1);
    (tw, th)
}

fn bgra_to_rgba(bgra: &[u8], width: u32, height: u32) -> Result<RgbaImage, ErrorData> {
    let expected = (width as usize) * (height as usize) * 4;
    if bgra.len() < expected {
        return Err(mcp_error(
            synapse_core::error_codes::TOOL_INTERNAL_ERROR,
            format!(
                "capture_gif frame byte length {} smaller than {width}x{height}x4={expected}",
                bgra.len()
            ),
        ));
    }
    let mut rgba = vec![0u8; expected];
    for (dst, src) in rgba.chunks_exact_mut(4).zip(bgra.chunks_exact(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
        mcp_error(
            synapse_core::error_codes::TOOL_INTERNAL_ERROR,
            "capture_gif could not build RGBA frame buffer",
        )
    })
}
