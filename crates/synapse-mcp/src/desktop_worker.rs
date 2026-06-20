use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::ValueEnum;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use synapse_core::{AccessibleSubtree, ForegroundContext, Rect, error_codes};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DesktopWorkerOp {
    Context,
    Snapshot,
    Capture,
}

impl DesktopWorkerOp {
    const fn as_arg(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Snapshot => "snapshot",
            Self::Capture => "capture",
        }
    }
}

#[derive(Debug)]
pub(crate) struct DesktopWorkerCli {
    pub op: Option<DesktopWorkerOp>,
    pub hwnd: Option<i64>,
    pub region: Option<String>,
    pub client_region: bool,
    pub depth: Option<u32>,
    pub json_path: Option<PathBuf>,
    pub bgra_path: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct HiddenDesktopCapture {
    pub context: ForegroundContext,
    pub bitmap: synapse_capture::CapturedBgraBitmap,
    pub capture_backend: &'static str,
    pub capture_region: Rect,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HiddenDesktopSnapshot {
    pub context: ForegroundContext,
    pub tree: AccessibleSubtree,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkerEnvelope {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<WorkerPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerPayload {
    Context {
        context: ForegroundContext,
    },
    Snapshot {
        context: ForegroundContext,
        tree: AccessibleSubtree,
    },
    Capture {
        context: ForegroundContext,
        region: Rect,
        width: u32,
        height: u32,
        capture_backend: String,
        bgra_bytes: u64,
    },
}

#[cfg(windows)]
pub(crate) fn run_worker_from_cli(args: DesktopWorkerCli) -> anyhow::Result<ExitCode> {
    let json_path = args
        .json_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--desktop-worker-json is required"))?;
    let envelope = match run_worker_operation(&args) {
        Ok(payload) => WorkerEnvelope {
            ok: true,
            payload: Some(payload),
            error_code: None,
            error_detail: None,
        },
        Err(error) => error,
    };
    write_worker_envelope(&json_path, &envelope)?;
    Ok(if envelope.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

#[cfg(not(windows))]
pub(crate) fn run_worker_from_cli(_args: DesktopWorkerCli) -> anyhow::Result<ExitCode> {
    anyhow::bail!("desktop-worker mode is only supported on Windows")
}

#[cfg(windows)]
fn run_worker_operation(args: &DesktopWorkerCli) -> Result<WorkerPayload, WorkerEnvelope> {
    let op = args.op.ok_or_else(|| worker_param_error("missing_op"))?;
    let hwnd = args
        .hwnd
        .ok_or_else(|| worker_param_error("missing_hwnd"))?;
    synapse_capture::init_process_dpi_awareness()
        .map_err(|error| worker_error(error.code(), error.to_string()))?;
    match op {
        DesktopWorkerOp::Context => {
            worker_context(hwnd).map(|context| WorkerPayload::Context { context })
        }
        DesktopWorkerOp::Snapshot => {
            let depth = args.depth.unwrap_or(2).min(16);
            let context = worker_context(hwnd)?;
            let tree = synapse_a11y::snapshot_window_from_hwnd(hwnd, depth)
                .map_err(|error| worker_error(error.code(), error.to_string()))?;
            Ok(WorkerPayload::Snapshot { context, tree })
        }
        DesktopWorkerOp::Capture => {
            let bgra_path = args
                .bgra_path
                .as_ref()
                .ok_or_else(|| worker_param_error("missing_bgra_path"))?;
            let requested_region = args.region.as_deref().map(parse_rect).transpose()?;
            let context = worker_context(hwnd)?;
            let region = worker_capture_region(hwnd, requested_region, args.client_region)?;
            let captured = synapse_capture::window_region_to_bgra_bitmap_printwindow(hwnd, region)
                .map_err(|error| worker_error(error.code(), error.to_string()))?;
            fs::write(bgra_path, &captured.bitmap.bytes).map_err(|error| {
                worker_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!("desktop worker could not write BGRA bytes: {error}"),
                )
            })?;
            Ok(WorkerPayload::Capture {
                context,
                region: captured.bitmap.region,
                width: captured.bitmap.width,
                height: captured.bitmap.height,
                capture_backend: captured.capture_backend.to_owned(),
                bgra_bytes: captured.bitmap.bytes.len() as u64,
            })
        }
    }
}

#[cfg(windows)]
fn worker_context(hwnd: i64) -> Result<ForegroundContext, WorkerEnvelope> {
    synapse_capture::validate_hwnd(hwnd).map_err(|error| {
        worker_error(
            error_codes::TARGET_WINDOW_NOT_FOUND,
            format!("hidden desktop hwnd {hwnd:#x} is not a live window: {error}"),
        )
    })?;
    synapse_a11y::foreground_context(hwnd)
        .map_err(|error| worker_error(error.code(), error.to_string()))
}

#[cfg(windows)]
fn worker_capture_region(
    hwnd: i64,
    region: Option<Rect>,
    client_region: bool,
) -> Result<Rect, WorkerEnvelope> {
    let Some(region) = region else {
        return synapse_capture::window_capture_region(hwnd)
            .map_err(|error| worker_error(error.code(), error.to_string()));
    };
    if client_region {
        return synapse_capture::client_region_to_window_region(hwnd, region)
            .map_err(|error| worker_error(error.code(), error.to_string()));
    }
    Ok(region)
}

#[cfg(windows)]
fn parse_rect(raw: &str) -> Result<Rect, WorkerEnvelope> {
    let parts = raw
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            worker_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("invalid --desktop-worker-region {raw:?}: {error}"),
            )
        })?;
    if parts.len() != 4 {
        return Err(worker_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("--desktop-worker-region must be x,y,w,h: {raw:?}"),
        ));
    }
    let rect = Rect {
        x: parts[0],
        y: parts[1],
        w: parts[2],
        h: parts[3],
    };
    if rect.w <= 0 || rect.h <= 0 {
        return Err(worker_error(
            error_codes::CAPTURE_TARGET_INVALID,
            format!("empty desktop worker capture region {rect:?}"),
        ));
    }
    Ok(rect)
}

fn worker_param_error(reason: &str) -> WorkerEnvelope {
    worker_error(
        error_codes::TOOL_PARAMS_INVALID,
        format!("desktop worker parameter error: {reason}"),
    )
}

fn worker_error(code: &'static str, detail: impl Into<String>) -> WorkerEnvelope {
    WorkerEnvelope {
        ok: false,
        payload: None,
        error_code: Some(code.to_owned()),
        error_detail: Some(detail.into()),
    }
}

fn write_worker_envelope(path: &Path, envelope: &WorkerEnvelope) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encoded = serde_json::to_vec_pretty(envelope)?;
    fs::write(path, encoded)?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn hidden_desktop_window_context(
    desktop_name: &str,
    hwnd: i64,
) -> Result<ForegroundContext, rmcp::ErrorData> {
    match run_worker(
        desktop_name,
        DesktopWorkerOp::Context,
        hwnd,
        None,
        false,
        None,
    )? {
        WorkerPayload::Context { context } => Ok(context),
        payload => Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("desktop worker returned unexpected context payload: {payload:?}"),
        )),
    }
}

#[cfg(not(windows))]
pub(crate) fn hidden_desktop_window_context(
    _desktop_name: &str,
    _hwnd: i64,
) -> Result<ForegroundContext, rmcp::ErrorData> {
    Err(crate::m1::mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "hidden desktop workers are only supported on Windows",
    ))
}

#[cfg(windows)]
pub(crate) fn hidden_desktop_window_snapshot(
    desktop_name: &str,
    hwnd: i64,
    depth: u32,
) -> Result<HiddenDesktopSnapshot, rmcp::ErrorData> {
    match run_worker(
        desktop_name,
        DesktopWorkerOp::Snapshot,
        hwnd,
        None,
        false,
        Some(depth),
    )? {
        WorkerPayload::Snapshot { context, tree } => Ok(HiddenDesktopSnapshot { context, tree }),
        payload => Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("desktop worker returned unexpected snapshot payload: {payload:?}"),
        )),
    }
}

#[cfg(windows)]
pub(crate) fn hidden_desktop_window_hwnds(desktop_name: &str) -> Result<Vec<i64>, rmcp::ErrorData> {
    use windows::{
        Win32::{
            Foundation::{LPARAM, SetLastError, WIN32_ERROR},
            System::StationsAndDesktops::{
                CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_ENUMERATE, DESKTOP_READ_CONTROL,
                DESKTOP_READOBJECTS, EnumDesktopWindows, OpenDesktopW,
            },
        },
        core::{BOOL, PCWSTR},
    };

    struct Search {
        hwnds: Vec<i64>,
    }

    unsafe extern "system" fn enum_window(
        hwnd: windows::Win32::Foundation::HWND,
        lparam: LPARAM,
    ) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        search.hwnds.push(hwnd.0 as isize as i64);
        BOOL(1)
    }

    let desktop_wide = wide_null(desktop_name);
    let access = DESKTOP_ENUMERATE.0 | DESKTOP_READOBJECTS.0 | DESKTOP_READ_CONTROL.0;
    let handle = unsafe {
        OpenDesktopW(
            PCWSTR(desktop_wide.as_ptr()),
            DESKTOP_CONTROL_FLAGS::default(),
            false,
            access,
        )
    }
    .map_err(|error| {
        crate::m1::mcp_error(
            error_codes::ACTION_TARGET_INVALID,
            format!("hidden desktop HWND enumeration failed to open {desktop_name:?}: {error}"),
        )
    })?;

    let mut search = Search { hwnds: Vec::new() };
    unsafe {
        SetLastError(WIN32_ERROR(0));
    }
    let enum_result = unsafe {
        EnumDesktopWindows(
            Some(handle),
            Some(enum_window),
            LPARAM((&raw mut search).cast::<core::ffi::c_void>() as isize),
        )
    };
    let close_result = unsafe { CloseDesktop(handle) };
    if let Err(error) = close_result {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("hidden desktop HWND enumeration failed to close {desktop_name:?}: {error}"),
        ));
    }
    if let Err(error) = enum_result {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("EnumDesktopWindows failed for hidden desktop {desktop_name:?}: {error}"),
        ));
    }
    search.hwnds.sort_unstable();
    search.hwnds.dedup();
    Ok(search.hwnds)
}

#[cfg(not(windows))]
pub(crate) fn hidden_desktop_window_hwnds(
    _desktop_name: &str,
) -> Result<Vec<i64>, rmcp::ErrorData> {
    Err(crate::m1::mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "hidden desktop HWND enumeration is only supported on Windows",
    ))
}

#[cfg(not(windows))]
pub(crate) fn hidden_desktop_window_snapshot(
    _desktop_name: &str,
    _hwnd: i64,
    _depth: u32,
) -> Result<HiddenDesktopSnapshot, rmcp::ErrorData> {
    Err(crate::m1::mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "hidden desktop workers are only supported on Windows",
    ))
}

#[cfg(windows)]
pub(crate) fn hidden_desktop_window_capture(
    desktop_name: &str,
    hwnd: i64,
    region: Option<Rect>,
    client_region: bool,
) -> Result<HiddenDesktopCapture, rmcp::ErrorData> {
    let temp = WorkerTempPaths::new(true)?;
    let payload = run_worker_with_paths(
        desktop_name,
        DesktopWorkerOp::Capture,
        hwnd,
        region,
        client_region,
        None,
        &temp,
    )?;
    let WorkerPayload::Capture {
        context,
        region,
        width,
        height,
        capture_backend,
        bgra_bytes,
    } = payload
    else {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("desktop worker returned unexpected capture payload: {payload:?}"),
        ));
    };
    let bgra_path = temp.bgra_path.as_ref().ok_or_else(|| {
        crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "desktop worker capture temp path was not allocated",
        )
    })?;
    let bytes = fs::read(bgra_path).map_err(|error| {
        crate::m1::mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "desktop worker BGRA readback failed for {}: {error}",
                bgra_path.display()
            ),
        )
    })?;
    if bytes.len() as u64 != bgra_bytes {
        return Err(crate::m1::mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "desktop worker BGRA length mismatch: metadata={bgra_bytes} actual={}",
                bytes.len()
            ),
        ));
    }
    let capture_backend = match capture_backend.as_str() {
        "printwindow" => "printwindow",
        other => {
            return Err(crate::m1::mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("desktop worker returned unsupported capture backend {other:?}"),
            ));
        }
    };
    Ok(HiddenDesktopCapture {
        context,
        bitmap: synapse_capture::CapturedBgraBitmap {
            region,
            width,
            height,
            bytes,
        },
        capture_backend,
        capture_region: region,
    })
}

#[cfg(not(windows))]
pub(crate) fn hidden_desktop_window_capture(
    _desktop_name: &str,
    _hwnd: i64,
    _region: Option<Rect>,
    _client_region: bool,
) -> Result<HiddenDesktopCapture, rmcp::ErrorData> {
    Err(crate::m1::mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "hidden desktop workers are only supported on Windows",
    ))
}

#[cfg(windows)]
fn run_worker(
    desktop_name: &str,
    op: DesktopWorkerOp,
    hwnd: i64,
    region: Option<Rect>,
    client_region: bool,
    depth: Option<u32>,
) -> Result<WorkerPayload, rmcp::ErrorData> {
    let temp = WorkerTempPaths::new(matches!(op, DesktopWorkerOp::Capture))?;
    run_worker_with_paths(desktop_name, op, hwnd, region, client_region, depth, &temp)
}

#[cfg(windows)]
fn run_worker_with_paths(
    desktop_name: &str,
    op: DesktopWorkerOp,
    hwnd: i64,
    region: Option<Rect>,
    client_region: bool,
    depth: Option<u32>,
    temp: &WorkerTempPaths,
) -> Result<WorkerPayload, rmcp::ErrorData> {
    launch_worker_process(desktop_name, op, hwnd, region, client_region, depth, temp)?;
    let envelope: WorkerEnvelope = read_json(&temp.json_path)?;
    if !envelope.ok {
        let code = envelope
            .error_code
            .as_deref()
            .map(worker_code_static)
            .unwrap_or(error_codes::TOOL_INTERNAL_ERROR);
        return Err(crate::m1::mcp_error(
            code,
            envelope
                .error_detail
                .unwrap_or_else(|| "desktop worker failed without detail".to_owned()),
        ));
    }
    envelope.payload.ok_or_else(|| {
        crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            "desktop worker succeeded without payload",
        )
    })
}

#[cfg(windows)]
fn launch_worker_process(
    desktop_name: &str,
    op: DesktopWorkerOp,
    hwnd: i64,
    region: Option<Rect>,
    client_region: bool,
    depth: Option<u32>,
    temp: &WorkerTempPaths,
) -> Result<(), rmcp::ErrorData> {
    use windows::{
        Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{
                CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess,
                PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
            },
        },
        core::{PCWSTR, PWSTR},
    };

    const WORKER_TIMEOUT_MS: u32 = 10_000;

    let exe = std::env::current_exe().map_err(|error| {
        crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("desktop worker could not resolve current executable: {error}"),
        )
    })?;
    let mut args = vec![
        exe.to_string_lossy().into_owned(),
        "--mode".to_owned(),
        "desktop-worker".to_owned(),
        "--desktop-worker-op".to_owned(),
        op.as_arg().to_owned(),
        "--desktop-worker-hwnd".to_owned(),
        hwnd.to_string(),
        "--desktop-worker-json".to_owned(),
        temp.json_path.to_string_lossy().into_owned(),
    ];
    if let Some(region) = region {
        args.push("--desktop-worker-region".to_owned());
        args.push(format!(
            "{},{},{},{}",
            region.x, region.y, region.w, region.h
        ));
    }
    if client_region {
        args.push("--desktop-worker-client-region".to_owned());
    }
    if let Some(depth) = depth {
        args.push("--desktop-worker-depth".to_owned());
        args.push(depth.to_string());
    }
    if let Some(bgra_path) = temp.bgra_path.as_ref() {
        args.push("--desktop-worker-bgra".to_owned());
        args.push(bgra_path.to_string_lossy().into_owned());
    }
    let command_line = args
        .iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let mut command_line_wide = wide_null(&command_line);
    let desktop_wide = wide_null(desktop_name);
    let startup_info = STARTUPINFOW {
        cb: u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(u32::MAX),
        lpDesktop: PWSTR(desktop_wide.as_ptr().cast_mut()),
        ..Default::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(command_line_wide.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &raw const startup_info,
            &raw mut process_info,
        )
    }
    .map_err(|error| {
        crate::m1::mcp_error(
            error_codes::ACTION_TARGET_INVALID,
            format!("desktop worker CreateProcessW failed for desktop {desktop_name:?}: {error}"),
        )
    })?;

    let wait = unsafe { WaitForSingleObject(process_info.hProcess, WORKER_TIMEOUT_MS) };
    if wait == WAIT_TIMEOUT {
        let _ = unsafe { TerminateProcess(process_info.hProcess, 1) };
        let _ = unsafe { CloseHandle(process_info.hThread) };
        let _ = unsafe { CloseHandle(process_info.hProcess) };
        return Err(crate::m1::mcp_error(
            error_codes::A11Y_UIA_WORKER_TIMEOUT,
            format!(
                "desktop worker timed out after {WORKER_TIMEOUT_MS} ms for desktop {desktop_name:?}"
            ),
        ));
    }
    let mut exit_code = 0_u32;
    let _ = unsafe { GetExitCodeProcess(process_info.hProcess, &raw mut exit_code) };
    let _ = unsafe { CloseHandle(process_info.hThread) };
    let _ = unsafe { CloseHandle(process_info.hProcess) };
    if wait != WAIT_OBJECT_0 {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("desktop worker wait failed for desktop {desktop_name:?}: wait={wait:?}"),
        ));
    }
    if exit_code != 0 && !temp.json_path.exists() {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!(
                "desktop worker exited {exit_code} without JSON readback for desktop {desktop_name:?}"
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, rmcp::ErrorData> {
    let bytes = fs::read(path).map_err(|error| {
        crate::m1::mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "desktop worker JSON readback failed for {}: {error}",
                path.display()
            ),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        crate::m1::mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!(
                "desktop worker JSON decode failed for {}: {error}",
                path.display()
            ),
        )
    })
}

#[cfg(windows)]
fn quote_windows_arg(arg: &str) -> String {
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
struct WorkerTempPaths {
    json_path: PathBuf,
    bgra_path: Option<PathBuf>,
}

#[cfg(windows)]
impl WorkerTempPaths {
    fn new(include_bgra: bool) -> Result<Self, rmcp::ErrorData> {
        let dir = std::env::temp_dir().join("synapse-desktop-worker");
        fs::create_dir_all(&dir).map_err(|error| {
            crate::m1::mcp_error(
                error_codes::STORAGE_WRITE_FAILED,
                format!(
                    "could not create desktop worker temp dir {}: {error}",
                    dir.display()
                ),
            )
        })?;
        let id = Uuid::new_v4();
        Ok(Self {
            json_path: dir.join(format!("{id}.json")),
            bgra_path: include_bgra.then(|| dir.join(format!("{id}.bgra"))),
        })
    }
}

#[cfg(windows)]
impl Drop for WorkerTempPaths {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.json_path);
        if let Some(path) = self.bgra_path.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
}

fn worker_code_static(code: &str) -> &'static str {
    match code {
        error_codes::ACTION_TARGET_INVALID => error_codes::ACTION_TARGET_INVALID,
        error_codes::A11Y_NOT_AVAILABLE => error_codes::A11Y_NOT_AVAILABLE,
        error_codes::A11Y_NO_FOREGROUND => error_codes::A11Y_NO_FOREGROUND,
        error_codes::A11Y_UIA_WORKER_TIMEOUT => error_codes::A11Y_UIA_WORKER_TIMEOUT,
        error_codes::CAPTURE_GRAPHICS_API_UNSUPPORTED => {
            error_codes::CAPTURE_GRAPHICS_API_UNSUPPORTED
        }
        error_codes::CAPTURE_PRINTWINDOW_BLACK => error_codes::CAPTURE_PRINTWINDOW_BLACK,
        error_codes::CAPTURE_PRINTWINDOW_DISABLED => error_codes::CAPTURE_PRINTWINDOW_DISABLED,
        error_codes::CAPTURE_TARGET_INVALID => error_codes::CAPTURE_TARGET_INVALID,
        error_codes::CAPTURE_TARGET_LOST => error_codes::CAPTURE_TARGET_LOST,
        error_codes::OBSERVE_INTERNAL => error_codes::OBSERVE_INTERNAL,
        error_codes::STORAGE_CORRUPTED => error_codes::STORAGE_CORRUPTED,
        error_codes::STORAGE_READ_FAILED => error_codes::STORAGE_READ_FAILED,
        error_codes::STORAGE_WRITE_FAILED => error_codes::STORAGE_WRITE_FAILED,
        error_codes::TARGET_WINDOW_NOT_FOUND => error_codes::TARGET_WINDOW_NOT_FOUND,
        error_codes::TOOL_PARAMS_INVALID => error_codes::TOOL_PARAMS_INVALID,
        _ => error_codes::TOOL_INTERNAL_ERROR,
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn desktop_worker_parse_rect_rejects_empty_capture_region_as_capture_invalid() {
        let error = parse_rect("0,0,0,10").expect_err("empty region should fail");

        assert_eq!(
            error.error_code.as_deref(),
            Some(error_codes::CAPTURE_TARGET_INVALID)
        );
        assert!(
            error
                .error_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("empty desktop worker capture region"))
        );
    }

    #[test]
    fn desktop_worker_parse_rect_rejects_malformed_region_as_params_invalid() {
        let error = parse_rect("0,0,10").expect_err("malformed region should fail");

        assert_eq!(
            error.error_code.as_deref(),
            Some(error_codes::TOOL_PARAMS_INVALID)
        );
    }
}
