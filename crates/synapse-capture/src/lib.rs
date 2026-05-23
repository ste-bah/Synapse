#![allow(unsafe_code)]

#[cfg(not(windows))]
use std::time::Duration;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use crossbeam::channel::{self, Receiver, Sender, TrySendError};
use synapse_core::{Point, Rect, error_codes};

pub const CAPTURE_CHANNEL_CAPACITY: usize = 2;
pub const FRAMES_DROPPED_METRIC: &str = "synapse_capture_frames_dropped_total";
const THREAD_PRIORITY_UNKNOWN: i32 = i32::MIN;
const THREAD_PRIORITY_UNSUPPORTED: i32 = i32::MIN + 1;
const THREAD_PRIORITY_TIME_CRITICAL: i32 = i32::MAX;

#[cfg(windows)]
pub type D3d11Texture = windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

#[cfg(not(windows))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct D3d11Texture;

#[derive(Debug)]
pub struct SendablePtr<T>(T);

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Send for SendablePtr<T> {}
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T> Sync for SendablePtr<T> {}

impl<T> SendablePtr<T> {
    #[must_use]
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }

    #[must_use]
    pub const fn get(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> Clone for SendablePtr<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

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

#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub texture: SendablePtr<D3d11Texture>,
    pub width: u32,
    pub height: u32,
    pub format: DxgiFormat,
    pub captured_at: Instant,
    pub frame_seq: u64,
    pub dirty_region: Option<Rect>,
}

impl CapturedFrame {
    #[cfg(not(windows))]
    #[allow(clippy::default_constructed_unit_structs)]
    #[must_use]
    pub fn synthetic(frame_seq: u64, width: u32, height: u32) -> Self {
        Self {
            texture: SendablePtr::new(D3d11Texture::default()),
            width,
            height,
            format: DxgiFormat::Bgra8,
            captured_at: Instant::now(),
            frame_seq,
            dirty_region: None,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CaptureBackend {
    GraphicsCaptureApi,
    DxgiDuplication,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CaptureBackendPreference {
    Auto,
    GraphicsCaptureApi,
    DxgiDuplication,
}

impl CaptureBackendPreference {
    #[must_use]
    pub fn from_force_dxgi_value(value: Option<&str>) -> Self {
        capture_backend_from_env(value)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CaptureTarget {
    #[default]
    Primary,
    Monitor {
        monitor_index: u32,
    },
    Window {
        hwnd: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureConfig {
    pub target: CaptureTarget,
    pub min_update_interval_ms: u64,
    pub cursor_visible: bool,
    pub secondary_windows: bool,
    pub dirty_region_only: bool,
    pub backend_preference: CaptureBackendPreference,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            target: CaptureTarget::Primary,
            min_update_interval_ms: 16,
            cursor_visible: true,
            secondary_windows: true,
            dirty_region_only: true,
            backend_preference: CaptureBackendPreference::Auto,
        }
    }
}

impl CaptureConfig {
    #[must_use]
    pub fn with_env_backend(mut self) -> Self {
        self.backend_preference = CaptureBackendPreference::from_force_dxgi_value(
            std::env::var("SYNAPSE_CAPTURE_FORCE_DXGI").ok().as_deref(),
        );
        self
    }

    #[must_use]
    pub const fn selected_backend(&self) -> CaptureBackend {
        resolved_backend(self.backend_preference)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCaptureTarget {
    pub target: CaptureTarget,
    pub backend: CaptureBackend,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("CAPTURE_GRAPHICS_API_UNSUPPORTED: {detail}")]
    GraphicsApiUnsupported { detail: String },
    #[error("CAPTURE_TARGET_LOST: {detail}")]
    TargetLost { detail: String },
    #[error("CAPTURE_TARGET_INVALID: {detail}")]
    TargetInvalid { detail: String },
    #[error("CAPTURE_NO_DIRTY_REGIONS")]
    NoDirtyRegions,
    #[error("CAPTURE_THREAD_FAILED: {detail}")]
    ThreadFailed { detail: String },
}

impl CaptureError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::GraphicsApiUnsupported { .. } => error_codes::CAPTURE_GRAPHICS_API_UNSUPPORTED,
            Self::TargetLost { .. } => error_codes::CAPTURE_TARGET_LOST,
            Self::TargetInvalid { .. } => error_codes::CAPTURE_TARGET_INVALID,
            Self::NoDirtyRegions => error_codes::CAPTURE_NO_DIRTY_REGIONS,
            Self::ThreadFailed { .. } => "CAPTURE_THREAD_FAILED",
        }
    }
}

#[derive(Debug)]
pub struct CaptureStats {
    frames_captured: AtomicU64,
    frames_dropped: AtomicU64,
    thread_priority: AtomicI32,
}

impl Default for CaptureStats {
    fn default() -> Self {
        Self {
            frames_captured: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            thread_priority: AtomicI32::new(THREAD_PRIORITY_UNKNOWN),
        }
    }
}

impl CaptureStats {
    #[must_use]
    pub fn frames_captured(&self) -> u64 {
        self.frames_captured.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn frames_dropped(&self) -> u64 {
        self.frames_dropped.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn thread_priority(&self) -> CaptureThreadPriority {
        decode_thread_priority(self.thread_priority.load(Ordering::Relaxed))
    }

    fn increment_captured(&self) {
        self.frames_captured.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_dropped(&self) {
        self.frames_dropped.fetch_add(1, Ordering::Relaxed);
        synapse_telemetry::metrics::counter!(FRAMES_DROPPED_METRIC).increment(1);
    }

    fn set_thread_priority(&self, priority: CaptureThreadPriority) {
        self.thread_priority
            .store(encode_thread_priority(priority), Ordering::Relaxed);
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CaptureThreadPriority {
    TimeCritical,
    Other(i32),
    Unsupported,
    Unknown,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DpiAwarenessStatus {
    Set,
    AlreadySet,
    Unsupported,
}

#[derive(Debug)]
pub struct CaptureHandle {
    rx: Receiver<CapturedFrame>,
    stop: Arc<AtomicBool>,
    stats: Arc<CaptureStats>,
    join: Option<JoinHandle<Result<(), CaptureError>>>,
    target: ResolvedCaptureTarget,
}

impl CaptureHandle {
    #[must_use]
    pub fn receiver(&self) -> Receiver<CapturedFrame> {
        self.rx.clone()
    }

    #[must_use]
    pub fn stats(&self) -> Arc<CaptureStats> {
        self.stats.clone()
    }

    #[must_use]
    pub const fn target(&self) -> &ResolvedCaptureTarget {
        &self.target
    }

    #[must_use]
    pub fn is_stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Requests shutdown and joins the capture thread.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if the capture thread panicked or the backend
    /// reports a terminal capture error during shutdown.
    pub fn stop(mut self) -> Result<(), CaptureError> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            join.join().map_err(|_err| CaptureError::ThreadFailed {
                detail: "capture thread panicked".to_owned(),
            })??;
        }
        Ok(())
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
pub struct CaptureController {
    active: Option<CaptureHandle>,
    generation: u64,
}

impl CaptureController {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: None,
            generation: 0,
        }
    }

    /// Stops any active session and starts a new capture target.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if the old session cannot be stopped or the new
    /// target cannot be resolved/opened.
    pub fn switch_to(&mut self, config: CaptureConfig) -> Result<u64, CaptureError> {
        if let Some(handle) = self.active.take() {
            handle.stop()?;
        }

        let handle = spawn_capture_loop(config)?;
        self.generation = self.generation.saturating_add(1);
        self.active = Some(handle);
        Ok(self.generation)
    }

    #[must_use]
    pub const fn active(&self) -> Option<&CaptureHandle> {
        self.active.as_ref()
    }
}

pub fn register_capture_metrics() {
    synapse_telemetry::metrics::describe_counter!(
        FRAMES_DROPPED_METRIC,
        "Frames dropped by the bounded capture channel"
    );
}

/// Resolves and validates a capture target without starting capture.
///
/// # Errors
///
/// Returns [`CaptureError`] when the target is invalid for the current platform.
pub fn resolve_capture_target(
    config: &CaptureConfig,
) -> Result<ResolvedCaptureTarget, CaptureError> {
    validate_target(&config.target)?;
    Ok(ResolvedCaptureTarget {
        target: config.target.clone(),
        backend: config.selected_backend(),
    })
}

/// Starts a capture loop using the configured target and backend.
///
/// # Errors
///
/// Returns [`CaptureError`] when the target is invalid or the capture thread
/// cannot be spawned.
pub fn spawn_capture_loop(config: CaptureConfig) -> Result<CaptureHandle, CaptureError> {
    register_capture_metrics();
    let target = resolve_capture_target(&config)?;
    let (tx, rx) = channel::bounded(CAPTURE_CHANNEL_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(CaptureStats::default());
    let ctx = CaptureThreadContext {
        tx,
        rx: rx.clone(),
        stop: stop.clone(),
        stats: stats.clone(),
    };
    let thread_config = config;
    let join = thread::Builder::new()
        .name("synapse-capture".to_owned())
        .spawn(move || run_capture_thread(thread_config, ctx))
        .map_err(|err| CaptureError::ThreadFailed {
            detail: err.to_string(),
        })?;

    Ok(CaptureHandle {
        rx,
        stop,
        stats,
        join: Some(join),
        target,
    })
}

/// Converts a screen-coordinate point to client/window coordinates.
///
/// # Errors
///
/// Returns [`CaptureError`] when the HWND is invalid or the OS coordinate
/// conversion fails.
pub fn screen_to_window(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    validate_hwnd(hwnd)?;
    screen_to_window_impl(point, hwnd)
}

/// Converts a client/window-coordinate point to screen coordinates.
///
/// # Errors
///
/// Returns [`CaptureError`] when the HWND is invalid or the OS coordinate
/// conversion fails.
pub fn window_to_screen(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    validate_hwnd(hwnd)?;
    window_to_screen_impl(point, hwnd)
}

#[must_use]
pub const fn screen_to_window_with_origin(point: Point, window_origin: Point) -> Point {
    Point {
        x: point.x - window_origin.x,
        y: point.y - window_origin.y,
    }
}

#[must_use]
pub const fn window_to_screen_with_origin(point: Point, window_origin: Point) -> Point {
    Point {
        x: point.x + window_origin.x,
        y: point.y + window_origin.y,
    }
}

/// Initializes per-monitor-v2 DPI awareness for accurate physical-pixel math.
///
/// # Errors
///
/// Returns [`CaptureError`] when Windows rejects the DPI-awareness call for a
/// reason other than "already set".
pub fn init_process_dpi_awareness() -> Result<DpiAwarenessStatus, CaptureError> {
    init_process_dpi_awareness_impl()
}

#[must_use]
#[allow(clippy::missing_const_for_fn)]
pub fn is_per_monitor_v2_dpi_aware() -> bool {
    is_per_monitor_v2_dpi_aware_impl()
}

#[must_use]
#[allow(clippy::missing_const_for_fn)]
pub fn current_thread_priority() -> CaptureThreadPriority {
    current_thread_priority_impl()
}

#[derive(Clone)]
struct CaptureThreadContext {
    tx: Sender<CapturedFrame>,
    rx: Receiver<CapturedFrame>,
    stop: Arc<AtomicBool>,
    stats: Arc<CaptureStats>,
}

fn run_capture_thread(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    set_capture_thread_priority()?;
    ctx.stats.set_thread_priority(current_thread_priority());
    match config.backend_preference {
        CaptureBackendPreference::Auto => match run_graphics_capture(config.clone(), ctx.clone()) {
            Ok(()) => Ok(()),
            Err(err) if should_fallback_to_dxgi(config.backend_preference, &err) => {
                tracing::warn!(
                    code = "CAPTURE_GRAPHICS_API_UNSUPPORTED",
                    fallback_backend = ?backend_after_fallback(config.backend_preference, &err),
                    error = %err,
                    "graphics capture unsupported; falling back to dxgi duplication"
                );
                run_dxgi_capture(config, ctx)
            }
            Err(err) => Err(err),
        },
        CaptureBackendPreference::GraphicsCaptureApi => run_graphics_capture(config, ctx),
        CaptureBackendPreference::DxgiDuplication => run_dxgi_capture(config, ctx),
    }
}

const fn resolved_backend(preference: CaptureBackendPreference) -> CaptureBackend {
    match preference {
        CaptureBackendPreference::Auto | CaptureBackendPreference::GraphicsCaptureApi => {
            CaptureBackend::GraphicsCaptureApi
        }
        CaptureBackendPreference::DxgiDuplication => CaptureBackend::DxgiDuplication,
    }
}

fn capture_backend_from_env(value: Option<&str>) -> CaptureBackendPreference {
    match value {
        Some("1" | "true" | "TRUE" | "yes" | "YES") => CaptureBackendPreference::DxgiDuplication,
        _ => CaptureBackendPreference::Auto,
    }
}

const fn backend_after_fallback(
    preference: CaptureBackendPreference,
    err: &CaptureError,
) -> CaptureBackend {
    match (preference, err) {
        (CaptureBackendPreference::Auto, CaptureError::GraphicsApiUnsupported { .. }) => {
            CaptureBackend::DxgiDuplication
        }
        _ => resolved_backend(preference),
    }
}

const fn should_fallback_to_dxgi(preference: CaptureBackendPreference, err: &CaptureError) -> bool {
    matches!(
        (preference, err),
        (
            CaptureBackendPreference::Auto,
            CaptureError::GraphicsApiUnsupported { .. }
        )
    )
}

const fn encode_thread_priority(priority: CaptureThreadPriority) -> i32 {
    match priority {
        CaptureThreadPriority::TimeCritical => THREAD_PRIORITY_TIME_CRITICAL,
        CaptureThreadPriority::Unsupported => THREAD_PRIORITY_UNSUPPORTED,
        CaptureThreadPriority::Unknown => THREAD_PRIORITY_UNKNOWN,
        CaptureThreadPriority::Other(value) => value,
    }
}

const fn decode_thread_priority(value: i32) -> CaptureThreadPriority {
    match value {
        THREAD_PRIORITY_TIME_CRITICAL => CaptureThreadPriority::TimeCritical,
        THREAD_PRIORITY_UNSUPPORTED => CaptureThreadPriority::Unsupported,
        THREAD_PRIORITY_UNKNOWN => CaptureThreadPriority::Unknown,
        other => CaptureThreadPriority::Other(other),
    }
}

fn push_frame(ctx: &CaptureThreadContext, frame: CapturedFrame) -> Result<(), CaptureError> {
    ctx.stats.increment_captured();
    match ctx.tx.try_send(frame) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(frame)) => {
            let _ = ctx.rx.try_recv();
            ctx.stats.increment_dropped();
            ctx.tx
                .try_send(frame)
                .map_err(|err| CaptureError::ThreadFailed {
                    detail: err.to_string(),
                })
        }
        Err(TrySendError::Disconnected(_frame)) => Err(CaptureError::ThreadFailed {
            detail: "capture receiver disconnected".to_owned(),
        }),
    }
}

fn validate_target(target: &CaptureTarget) -> Result<(), CaptureError> {
    match target {
        CaptureTarget::Primary | CaptureTarget::Monitor { .. } => Ok(()),
        CaptureTarget::Window { hwnd } => validate_hwnd(*hwnd),
    }
}

fn validate_hwnd(hwnd: i64) -> Result<(), CaptureError> {
    if hwnd <= 0 {
        return Err(CaptureError::TargetInvalid {
            detail: format!("invalid HWND {hwnd}"),
        });
    }

    validate_hwnd_impl(hwnd)
}

#[cfg(not(windows))]
#[allow(clippy::needless_pass_by_value)]
fn run_graphics_capture(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    run_synthetic_capture_loop(&config, &ctx)
}

#[cfg(not(windows))]
#[allow(clippy::needless_pass_by_value)]
fn run_dxgi_capture(config: CaptureConfig, ctx: CaptureThreadContext) -> Result<(), CaptureError> {
    run_synthetic_capture_loop(&config, &ctx)
}

#[cfg(not(windows))]
fn run_synthetic_capture_loop(
    config: &CaptureConfig,
    ctx: &CaptureThreadContext,
) -> Result<(), CaptureError> {
    let interval = Duration::from_millis(config.min_update_interval_ms.max(1));
    let mut frame_seq = 0_u64;
    while !ctx.stop.load(Ordering::Relaxed) {
        push_frame(ctx, CapturedFrame::synthetic(frame_seq, 1920, 1080))?;
        frame_seq = frame_seq.saturating_add(1);
        thread::sleep(interval);
    }
    Ok(())
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn screen_to_window_impl(point: Point, _hwnd: i64) -> Result<Point, CaptureError> {
    Ok(point)
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn window_to_screen_impl(point: Point, _hwnd: i64) -> Result<Point, CaptureError> {
    Ok(point)
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn init_process_dpi_awareness_impl() -> Result<DpiAwarenessStatus, CaptureError> {
    Ok(DpiAwarenessStatus::Unsupported)
}

#[cfg(not(windows))]
const fn is_per_monitor_v2_dpi_aware_impl() -> bool {
    false
}

#[cfg(not(windows))]
const fn current_thread_priority_impl() -> CaptureThreadPriority {
    CaptureThreadPriority::Unsupported
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn set_capture_thread_priority() -> Result<(), CaptureError> {
    Ok(())
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
fn validate_hwnd_impl(_hwnd: i64) -> Result<(), CaptureError> {
    Ok(())
}

#[cfg(windows)]
fn run_graphics_capture(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    windows_impl::run_graphics_capture(config, ctx)
}

#[cfg(windows)]
fn run_dxgi_capture(config: CaptureConfig, ctx: CaptureThreadContext) -> Result<(), CaptureError> {
    windows_impl::run_dxgi_capture(config, ctx)
}

#[cfg(windows)]
fn screen_to_window_impl(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    windows_impl::screen_to_window(point, hwnd)
}

#[cfg(windows)]
fn window_to_screen_impl(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    windows_impl::window_to_screen(point, hwnd)
}

#[cfg(windows)]
fn init_process_dpi_awareness_impl() -> Result<DpiAwarenessStatus, CaptureError> {
    windows_impl::init_process_dpi_awareness()
}

#[cfg(windows)]
fn is_per_monitor_v2_dpi_aware_impl() -> bool {
    windows_impl::is_per_monitor_v2_dpi_aware()
}

#[cfg(windows)]
fn current_thread_priority_impl() -> CaptureThreadPriority {
    windows_impl::current_thread_priority()
}

#[cfg(windows)]
fn set_capture_thread_priority() -> Result<(), CaptureError> {
    windows_impl::set_capture_thread_priority()
}

#[cfg(windows)]
fn validate_hwnd_impl(hwnd: i64) -> Result<(), CaptureError> {
    windows_impl::validate_hwnd(hwnd)
}

#[cfg(windows)]
mod windows_impl {
    use std::{ffi::c_void, thread, time::Duration};

    use synapse_core::{Point, Rect};
    use windows::Win32::{
        Foundation::{E_ACCESSDENIED, HWND, POINT},
        Graphics::Gdi::{ClientToScreen, ScreenToClient},
        System::Threading::{
            GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        },
        UI::{
            HiDpi::{
                AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
            },
            WindowsAndMessaging::IsWindow,
        },
    };
    use windows_capture::{
        capture::{Context, GraphicsCaptureApiHandler},
        dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error as DxgiError},
        frame::{DirtyRegion, Frame},
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{
            ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
            MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
        },
        window::Window,
    };

    use super::{
        CaptureConfig, CaptureError, CaptureTarget, CaptureThreadContext, CaptureThreadPriority,
        CapturedFrame, DpiAwarenessStatus, DxgiFormat, SendablePtr, push_frame,
    };

    pub fn run_graphics_capture(
        config: CaptureConfig,
        ctx: CaptureThreadContext,
    ) -> Result<(), CaptureError> {
        match config.target.clone() {
            CaptureTarget::Primary => {
                let monitor = Monitor::primary().map_err(capture_unsupported)?;
                start_graphics_capture_with_item(monitor, config, ctx)
            }
            CaptureTarget::Monitor { monitor_index } => {
                let monitor =
                    Monitor::from_index(usize::try_from(monitor_index.saturating_add(1)).map_err(
                        |err| CaptureError::TargetInvalid {
                            detail: err.to_string(),
                        },
                    )?)
                    .map_err(|err| CaptureError::TargetInvalid {
                        detail: err.to_string(),
                    })?;
                start_graphics_capture_with_item(monitor, config, ctx)
            }
            CaptureTarget::Window { hwnd } => {
                validate_hwnd(hwnd)?;
                let window = Window::from_raw_hwnd(hwnd as *mut c_void);
                start_graphics_capture_with_item(window, config, ctx)
            }
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn run_dxgi_capture(
        config: CaptureConfig,
        ctx: CaptureThreadContext,
    ) -> Result<(), CaptureError> {
        let monitor = match config.target {
            CaptureTarget::Primary => Monitor::primary().map_err(capture_unsupported)?,
            CaptureTarget::Monitor { monitor_index } => {
                Monitor::from_index(usize::try_from(monitor_index.saturating_add(1)).map_err(
                    |err| CaptureError::TargetInvalid {
                        detail: err.to_string(),
                    },
                )?)
                .map_err(|err| CaptureError::TargetInvalid {
                    detail: err.to_string(),
                })?
            }
            CaptureTarget::Window { .. } => {
                return Err(CaptureError::TargetInvalid {
                    detail: "DXGI duplication supports monitor targets only".to_owned(),
                });
            }
        };
        let mut api = DxgiDuplicationApi::new(monitor).map_err(|err| dxgi_error(&err))?;
        let timeout_ms = u32::try_from(config.min_update_interval_ms.max(1)).unwrap_or(u32::MAX);
        let mut frame_seq = 0_u64;

        while !ctx.stop.load(std::sync::atomic::Ordering::Relaxed) {
            match api.acquire_next_frame(timeout_ms) {
                Ok(frame) => {
                    let captured = CapturedFrame {
                        texture: SendablePtr::new(frame.texture().clone()),
                        width: frame.width(),
                        height: frame.height(),
                        format: dxgi_format(frame.format()),
                        captured_at: std::time::Instant::now(),
                        frame_seq,
                        dirty_region: None,
                    };
                    push_frame(&ctx, captured)?;
                    frame_seq = frame_seq.saturating_add(1);
                }
                Err(DxgiError::Timeout) => {
                    thread::sleep(Duration::from_millis(config.min_update_interval_ms.max(1)));
                }
                Err(DxgiError::AccessLost) => {
                    return Err(CaptureError::TargetLost {
                        detail: "DXGI output duplication access lost".to_owned(),
                    });
                }
                Err(err) => return Err(dxgi_error(&err)),
            }
        }

        Ok(())
    }

    pub fn screen_to_window(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
        let hwnd = hwnd_from_i64(hwnd);
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
        let hwnd = hwnd_from_i64(hwnd);
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

    pub fn init_process_dpi_awareness() -> Result<DpiAwarenessStatus, CaptureError> {
        match unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } {
            Ok(()) => Ok(DpiAwarenessStatus::Set),
            Err(err) if err.code() == E_ACCESSDENIED => Ok(DpiAwarenessStatus::AlreadySet),
            Err(err) => Err(CaptureError::ThreadFailed {
                detail: err.to_string(),
            }),
        }
    }

    pub fn is_per_monitor_v2_dpi_aware() -> bool {
        unsafe {
            AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
        }
        .as_bool()
    }

    pub fn current_thread_priority() -> CaptureThreadPriority {
        let priority = unsafe { GetThreadPriority(GetCurrentThread()) };
        if priority == THREAD_PRIORITY_TIME_CRITICAL.0 {
            CaptureThreadPriority::TimeCritical
        } else {
            CaptureThreadPriority::Other(priority)
        }
    }

    pub fn set_capture_thread_priority() -> Result<(), CaptureError> {
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) }.map_err(
            |err| CaptureError::ThreadFailed {
                detail: err.to_string(),
            },
        )
    }

    pub fn validate_hwnd(hwnd: i64) -> Result<(), CaptureError> {
        let hwnd = hwnd_from_i64(hwnd);
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            Ok(())
        } else {
            Err(CaptureError::TargetInvalid {
                detail: "HWND is not a live window".to_owned(),
            })
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn start_graphics_capture_with_item<T>(
        item: T,
        config: CaptureConfig,
        ctx: CaptureThreadContext,
    ) -> Result<(), CaptureError>
    where
        T: TryInto<windows_capture::settings::GraphicsCaptureItemType>,
    {
        let settings = Settings::new(
            item,
            if config.cursor_visible {
                CursorCaptureSettings::WithCursor
            } else {
                CursorCaptureSettings::WithoutCursor
            },
            DrawBorderSettings::WithoutBorder,
            if config.secondary_windows {
                SecondaryWindowSettings::Include
            } else {
                SecondaryWindowSettings::Exclude
            },
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(
                config.min_update_interval_ms.max(1),
            )),
            if config.dirty_region_only {
                DirtyRegionSettings::ReportAndRender
            } else {
                DirtyRegionSettings::Default
            },
            ColorFormat::Bgra8,
            GraphicsHandlerFlags { ctx },
        );
        GraphicsHandler::start(settings).map_err(|err| CaptureError::ThreadFailed {
            detail: err.to_string(),
        })
    }

    struct GraphicsHandlerFlags {
        ctx: CaptureThreadContext,
    }

    struct GraphicsHandler {
        ctx: CaptureThreadContext,
        frame_seq: u64,
    }

    impl GraphicsCaptureApiHandler for GraphicsHandler {
        type Flags = GraphicsHandlerFlags;
        type Error = CaptureError;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self {
                ctx: ctx.flags.ctx,
                frame_seq: 0,
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            if self.ctx.stop.load(std::sync::atomic::Ordering::Relaxed) {
                control.stop();
                return Ok(());
            }

            let captured = CapturedFrame {
                texture: SendablePtr::new(frame.as_raw_texture().clone()),
                width: frame.width(),
                height: frame.height(),
                format: match frame.color_format() {
                    ColorFormat::Bgra8 => DxgiFormat::Bgra8,
                    ColorFormat::Rgba8 => DxgiFormat::Rgba8,
                    ColorFormat::Rgba16F => DxgiFormat::Rgba16F,
                },
                captured_at: std::time::Instant::now(),
                frame_seq: self.frame_seq,
                dirty_region: union_dirty_regions(&frame.dirty_regions().unwrap_or_default()),
            };
            push_frame(&self.ctx, captured)?;
            self.frame_seq = self.frame_seq.saturating_add(1);
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            self.ctx
                .stop
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    fn capture_unsupported<E: std::fmt::Display>(err: E) -> CaptureError {
        CaptureError::GraphicsApiUnsupported {
            detail: err.to_string(),
        }
    }

    fn dxgi_error(err: &DxgiError) -> CaptureError {
        CaptureError::GraphicsApiUnsupported {
            detail: err.to_string(),
        }
    }

    const fn dxgi_format(format: DxgiDuplicationFormat) -> DxgiFormat {
        match format {
            DxgiDuplicationFormat::Rgba16F => DxgiFormat::Rgba16F,
            DxgiDuplicationFormat::Rgb10A2 => DxgiFormat::Rgb10A2,
            DxgiDuplicationFormat::Rgb10XrA2 => DxgiFormat::Rgb10XrA2,
            DxgiDuplicationFormat::Rgba8 => DxgiFormat::Rgba8,
            DxgiDuplicationFormat::Rgba8Srgb => DxgiFormat::Rgba8Srgb,
            DxgiDuplicationFormat::Bgra8 => DxgiFormat::Bgra8,
            DxgiDuplicationFormat::Bgra8Srgb => DxgiFormat::Bgra8Srgb,
        }
    }

    fn union_dirty_regions(regions: &[DirtyRegion]) -> Option<Rect> {
        let first = regions.first()?;
        let mut left = first.x;
        let mut top = first.y;
        let mut right = first.x.saturating_add(first.width);
        let mut bottom = first.y.saturating_add(first.height);

        for region in &regions[1..] {
            left = left.min(region.x);
            top = top.min(region.y);
            right = right.max(region.x.saturating_add(region.width));
            bottom = bottom.max(region.y.saturating_add(region.height));
        }

        Some(Rect {
            x: left,
            y: top,
            w: right.saturating_sub(left),
            h: bottom.saturating_sub(top),
        })
    }

    #[allow(clippy::missing_const_for_fn)]
    fn hwnd_from_i64(hwnd: i64) -> HWND {
        HWND(hwnd as *mut c_void)
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, thread, time::Duration};

    use proptest::prelude::*;
    use synapse_core::Point;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(not(windows))]
    #[test]
    fn captured_frame_synthetic_shape_is_stable() {
        let frame = CapturedFrame::synthetic(42, 1920, 1080);

        assert_eq!(frame.width, 1920);
        assert_eq!(frame.height, 1080);
        assert_eq!(frame.frame_seq, 42);
        assert_eq!(frame.format, DxgiFormat::Bgra8);
        assert!(frame.dirty_region.is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn captured_frame_drop_loop_is_raii_safe_for_synthetic_texture() {
        for seq in 0..1_000 {
            let _frame = CapturedFrame::synthetic(seq, 16, 16);
        }
    }

    #[cfg(windows)]
    #[test]
    fn captured_frame_drop_loop_queries_d3d_texture() -> Result<(), CaptureError> {
        use windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC;

        let _guard = CAPTURE_LOCK
            .lock()
            .unwrap_or_else(|err| panic!("capture lock poisoned: {err}"));
        let handle = spawn_capture_loop(CaptureConfig {
            min_update_interval_ms: 16,
            dirty_region_only: false,
            ..CaptureConfig::default()
        })?;
        let rx = handle.receiver();
        let mut queried = 0_u32;

        for _ in 0..1_000 {
            let frame = rx.recv_timeout(Duration::from_secs(5)).map_err(|err| {
                CaptureError::ThreadFailed {
                    detail: err.to_string(),
                }
            })?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                frame.texture.get().GetDesc(std::ptr::addr_of_mut!(desc));
            }
            if queried == 0 || queried == 999 {
                println!(
                    "d3d_query frame_seq={} desc_width={} desc_height={} frame_width={} frame_height={}",
                    frame.frame_seq, desc.Width, desc.Height, frame.width, frame.height
                );
            }
            assert_eq!(desc.Width, frame.width);
            assert_eq!(desc.Height, frame.height);
            queried = queried.saturating_add(1);
        }

        let stats = handle.stats();
        println!(
            "after d3d_drop_loop queried={} captured={} dropped={} priority={:?}",
            queried,
            stats.frames_captured(),
            stats.frames_dropped(),
            stats.thread_priority()
        );
        handle.stop()?;
        assert_eq!(queried, 1_000);
        Ok(())
    }

    #[test]
    fn force_dxgi_env_value_selects_dxgi_backend() {
        let config = CaptureConfig {
            backend_preference: CaptureBackendPreference::from_force_dxgi_value(Some("1")),
            ..CaptureConfig::default()
        };
        assert_eq!(config.selected_backend(), CaptureBackend::DxgiDuplication);
    }

    #[test]
    fn force_dxgi_env_var_selects_dxgi_backend() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|err| panic!("env lock poisoned: {err}"));
        let previous = std::env::var_os("SYNAPSE_CAPTURE_FORCE_DXGI");
        println!(
            "before env_dxgi previous={:?} selected_backend={:?}",
            previous,
            CaptureConfig::default().selected_backend()
        );

        // SAFETY: this test serializes access with ENV_LOCK and restores the
        // prior value before returning.
        unsafe {
            std::env::set_var("SYNAPSE_CAPTURE_FORCE_DXGI", "1");
        }
        let config = CaptureConfig::default().with_env_backend();
        println!(
            "after env_dxgi value=1 selected_backend={:?}",
            config.selected_backend()
        );
        assert_eq!(config.selected_backend(), CaptureBackend::DxgiDuplication);

        // SAFETY: same ENV_LOCK serialization as above.
        unsafe {
            match previous {
                Some(value) => std::env::set_var("SYNAPSE_CAPTURE_FORCE_DXGI", value),
                None => std::env::remove_var("SYNAPSE_CAPTURE_FORCE_DXGI"),
            }
        }
    }

    #[test]
    fn auto_backend_falls_back_only_for_graphics_unsupported() {
        let unsupported = CaptureError::GraphicsApiUnsupported {
            detail: "synthetic unsupported".to_owned(),
        };
        println!(
            "before fallback preference={:?} error_code={}",
            CaptureBackendPreference::Auto,
            unsupported.code()
        );
        assert!(should_fallback_to_dxgi(
            CaptureBackendPreference::Auto,
            &unsupported
        ));
        assert_eq!(
            backend_after_fallback(CaptureBackendPreference::Auto, &unsupported),
            CaptureBackend::DxgiDuplication
        );
        println!(
            "after fallback effective_backend={:?}",
            backend_after_fallback(CaptureBackendPreference::Auto, &unsupported)
        );

        let invalid = CaptureError::TargetInvalid {
            detail: "bad hwnd".to_owned(),
        };
        assert!(!should_fallback_to_dxgi(
            CaptureBackendPreference::Auto,
            &invalid
        ));
    }

    #[test]
    fn invalid_hwnd_surfaces_capture_target_invalid() {
        let config = CaptureConfig {
            target: CaptureTarget::Window { hwnd: 0 },
            ..CaptureConfig::default()
        };
        println!("before invalid_hwnd target={:?}", config.target);

        let err = resolve_capture_target(&config)
            .err()
            .unwrap_or_else(|| panic!("invalid hwnd should fail"));
        println!("after invalid_hwnd error_code={}", err.code());
        assert_eq!(err.code(), error_codes::CAPTURE_TARGET_INVALID);
    }

    #[test]
    fn target_lost_error_surfaces_code() {
        let err = CaptureError::TargetLost {
            detail: "synthetic target loss".to_owned(),
        };
        println!("target_lost error_code={}", err.code());
        assert_eq!(err.code(), error_codes::CAPTURE_TARGET_LOST);
    }

    #[test]
    fn capture_channel_capacity_is_exactly_two_and_drops_oldest() -> Result<(), CaptureError> {
        let _guard = CAPTURE_LOCK
            .lock()
            .unwrap_or_else(|err| panic!("capture lock poisoned: {err}"));
        let handle = spawn_capture_loop(CaptureConfig {
            min_update_interval_ms: 1,
            dirty_region_only: false,
            ..CaptureConfig::default()
        })?;
        let stats = handle.stats();
        println!(
            "before slow_consumer captured={} dropped={} channel_len={}",
            stats.frames_captured(),
            stats.frames_dropped(),
            handle.receiver().len()
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline
            && (stats.frames_captured() <= 2 || stats.frames_dropped() == 0)
        {
            thread::sleep(Duration::from_millis(10));
        }

        println!(
            "after slow_consumer captured={} dropped={} channel_len={}",
            stats.frames_captured(),
            stats.frames_dropped(),
            handle.receiver().len()
        );
        assert!(stats.frames_captured() > 2);
        assert!(stats.frames_dropped() > 0);
        assert_eq!(CAPTURE_CHANNEL_CAPACITY, 2);
        assert!(handle.receiver().len() <= CAPTURE_CHANNEL_CAPACITY);
        handle.stop()
    }

    #[test]
    fn capture_thread_priority_is_recorded() -> Result<(), CaptureError> {
        let _guard = CAPTURE_LOCK
            .lock()
            .unwrap_or_else(|err| panic!("capture lock poisoned: {err}"));
        let handle = spawn_capture_loop(CaptureConfig {
            min_update_interval_ms: 1,
            ..CaptureConfig::default()
        })?;
        let stats = handle.stats();
        println!("before priority_readback={:?}", stats.thread_priority());
        thread::sleep(Duration::from_millis(20));
        let priority = stats.thread_priority();
        println!("after priority_readback={priority:?}");
        if cfg!(windows) {
            assert_eq!(priority, CaptureThreadPriority::TimeCritical);
        } else {
            assert_eq!(priority, CaptureThreadPriority::Unsupported);
        }
        handle.stop()
    }

    #[test]
    fn coordinate_transform_manual_edge_cases_round_trip() {
        let cases = [
            (Point { x: 0, y: 0 }, Point { x: 0, y: 0 }),
            (
                Point {
                    x: 100_000,
                    y: -100_000,
                },
                Point {
                    x: -10_000,
                    y: 10_000,
                },
            ),
            (
                Point {
                    x: -100_000,
                    y: 100_000,
                },
                Point {
                    x: 10_000,
                    y: -10_000,
                },
            ),
        ];

        for (point, origin) in cases {
            println!("before transform point={point:?} origin={origin:?}");
            let screen = window_to_screen_with_origin(point, origin);
            let round_trip = screen_to_window_with_origin(screen, origin);
            println!("after transform screen={screen:?} round_trip={round_trip:?}");
            assert_eq!(round_trip, point);
        }
    }

    #[test]
    fn switching_capture_target_stops_previous_session() -> Result<(), CaptureError> {
        let _guard = CAPTURE_LOCK
            .lock()
            .unwrap_or_else(|err| panic!("capture lock poisoned: {err}"));
        let mut controller = CaptureController::new();
        assert_eq!(controller.switch_to(CaptureConfig::default())?, 1);
        let first_stop = controller.active().map_or_else(
            || panic!("capture handle should be active"),
            |handle| handle.stop.clone(),
        );
        assert_eq!(
            controller.switch_to(CaptureConfig {
                target: CaptureTarget::Monitor { monitor_index: 0 },
                ..CaptureConfig::default()
            })?,
            2
        );
        assert!(first_stop.load(Ordering::Relaxed));
        Ok(())
    }

    proptest! {
        #[test]
        fn coordinate_transform_origin_round_trip(
            x in -100_000_i32..100_000,
            y in -100_000_i32..100_000,
            ox in -10_000_i32..10_000,
            oy in -10_000_i32..10_000,
        ) {
            let point = Point { x, y };
            let origin = Point { x: ox, y: oy };
            let screen = window_to_screen_with_origin(point, origin);
            prop_assert_eq!(screen_to_window_with_origin(screen, origin), point);
        }
    }

    #[test]
    fn dpi_awareness_is_noop_off_windows() -> Result<(), CaptureError> {
        if cfg!(windows) {
            return Ok(());
        }

        assert_eq!(
            init_process_dpi_awareness()?,
            DpiAwarenessStatus::Unsupported
        );
        assert_eq!(
            current_thread_priority(),
            CaptureThreadPriority::Unsupported
        );
        Ok(())
    }

    #[test]
    fn dpi_awareness_readback_matches_platform() -> Result<(), CaptureError> {
        let before = is_per_monitor_v2_dpi_aware();
        let status = init_process_dpi_awareness()?;
        let after = is_per_monitor_v2_dpi_aware();
        println!("dpi_readback before={before} status={status:?} after={after}");
        if cfg!(windows) {
            assert!(after);
        } else {
            assert_eq!(status, DpiAwarenessStatus::Unsupported);
            assert!(!after);
        }
        Ok(())
    }
}
