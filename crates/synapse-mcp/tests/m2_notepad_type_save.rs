#[cfg(windows)]
use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{Duration, Instant},
};

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use serde::de::DeserializeOwned;
#[cfg(windows)]
use serde_json::{Value, json};
#[cfg(windows)]
use synapse_core::{AccessibleNode, ElementId, Observation, Rect, UiaPattern};
#[cfg(windows)]
use synapse_test_utils::{fixtures::launch_notepad, stdio_mcp_client::StdioMcpClient};
#[cfg(windows)]
use tempfile::TempDir;

#[cfg(windows)]
static WINDOWS_NOTEPAD_FSV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[cfg(windows)]
const DEMO_FILE_NAME: &str = "synapse-m2-demo.txt";
#[cfg(windows)]
const DEMO_TEXT_TO_TYPE: &str = "Hello world.\nThis is Synapse.";
#[cfg(windows)]
const EXPECTED_DISK_TEXT: &str = "Hello world.\r\nThis is Synapse.";

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires an interactive Windows desktop with Notepad and UIA"]
async fn notepad_type_save_writes_byte_correct_file_fsv() -> anyhow::Result<()> {
    let _notepad_lock = WINDOWS_NOTEPAD_FSV_LOCK.lock().await;
    let target_path = std::env::temp_dir().join(DEMO_FILE_NAME);
    let cleanup = FileCleanup(target_path.clone());
    let stale_before_exists = target_path.exists();
    let stale_before_len = if stale_before_exists {
        Some(std::fs::metadata(&target_path)?.len())
    } else {
        None
    };
    println!(
        "source_of_truth=disk edge=stale_preexisting before_path={} before_exists={} before_len={:?}",
        target_path.display(),
        stale_before_exists,
        stale_before_len
    );
    if target_path.exists() {
        std::fs::remove_file(&target_path)
            .with_context(|| format!("remove stale demo file {}", target_path.display()))?;
    }
    println!(
        "source_of_truth=disk edge=happy before_path={} before_exists={}",
        target_path.display(),
        target_path.exists()
    );

    let log_dir = TempDir::new()?;
    let handle = launch_notepad()?;
    let hwnd = handle.hwnd();
    let pid = handle.pid();
    let pid_preexisting = handle.pid_preexisting();
    println!(
        "source_of_truth=NotepadHandle edge=ownership after_hwnd=0x{hwnd:x} after_pid={pid} pid_preexisting={pid_preexisting}"
    );
    let mut client = StdioMcpClient::launch_and_init_with_log_dir(Some(log_dir.path())).await?;

    let editor_id = editor_from_uia_snapshot(hwnd)?;
    match synapse_a11y::focus_window(hwnd) {
        Ok(()) => println!(
            "source_of_truth=synapse_a11y::focus_window edge=window after=ok hwnd=0x{hwnd:x}"
        ),
        Err(error) => {
            println!("source_of_truth=synapse_a11y::focus_window edge=window after_error={error}")
        }
    }
    let editor = synapse_a11y::re_resolve(&editor_id)
        .with_context(|| format!("re-resolve Notepad editor element {editor_id}"))?;
    editor
        .set_focus()
        .with_context(|| format!("set UIA focus on Notepad editor element {editor_id}"))?;
    println!(
        "source_of_truth=synapse_a11y::UIElement::set_focus edge=editor after=ok element_id={editor_id}"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("source_of_truth=editor_element edge=click before_element_id={editor_id}");
    let click = client
        .tools_call(
            "act_click",
            json!({"target": {"element_id": editor_id.to_string()}, "use_invoke_pattern": true}),
        )
        .await?;
    let click_response: ActClickWireResponse = structured(&click)?;
    println!(
        "source_of_truth=mcp_act_click edge=editor_focus after=ok:{} used_invoke_pattern:{} backend_used:{} elapsed_ms:{}",
        click_response.ok,
        click_response.used_invoke_pattern,
        click_response.backend_used,
        click_response.elapsed_ms
    );
    assert!(click_response.ok);

    tokio::time::sleep(Duration::from_millis(200)).await;
    let observe_before = observe(&mut client).await?;
    println!(
        "source_of_truth=mcp_observe edge=before_type hwnd=0x{hwnd:x} pid={pid} title={:?} focused_role={:?} element_count={}",
        observe_before.foreground.window_title,
        observe_before
            .focused
            .as_ref()
            .map(|focused| focused.role.as_str()),
        observe_before.elements.len()
    );
    assert_eq!(observe_before.foreground.hwnd, hwnd);
    assert_eq!(observe_before.foreground.pid, pid);

    let typed = client
        .tools_call(
            "act_type",
            json!({
                "text": DEMO_TEXT_TO_TYPE,
                "dynamics": "linear",
                "linear_ms_per_char": 30
            }),
        )
        .await?;
    let typed_response: ActTypeWireResponse = structured(&typed)?;
    println!(
        "source_of_truth=mcp_act_type edge=demo_text after=ok:{} chars_typed:{} elapsed_ms:{} expected_chars:{}",
        typed_response.ok,
        typed_response.chars_typed,
        typed_response.elapsed_ms,
        DEMO_TEXT_TO_TYPE.chars().count()
    );
    assert!(typed_response.ok);
    assert_eq!(
        typed_response.chars_typed as usize,
        DEMO_TEXT_TO_TYPE.chars().count()
    );

    let observe_after_type = observe(&mut client).await?;
    println!(
        "source_of_truth=mcp_observe edge=after_type focused_role={:?} focused_value_len={:?}",
        observe_after_type
            .focused
            .as_ref()
            .map(|focused| focused.role.as_str()),
        observe_after_type
            .focused
            .as_ref()
            .and_then(|focused| focused.value.as_ref())
            .map(String::len)
    );

    println!("source_of_truth=mcp_act_press edge=save_chord before=keys:[ctrl,s]");
    let save = client
        .tools_call("act_press", json!({"keys": ["ctrl", "s"], "hold_ms": 33}))
        .await?;
    let save_response: ActPressWireResponse = structured(&save)?;
    println!(
        "source_of_truth=mcp_act_press edge=save_chord after=ok:{} keys_pressed:{} backend_used:{} elapsed_ms:{}",
        save_response.ok,
        save_response.keys_pressed,
        save_response.backend_used,
        save_response.elapsed_ms
    );
    assert!(save_response.ok);
    assert_eq!(save_response.keys_pressed, 2);

    tokio::time::sleep(Duration::from_millis(500)).await;
    let observe_save_dialog = observe(&mut client).await?;
    println!(
        "source_of_truth=mcp_observe edge=save_dialog after_title={:?} process={:?} focused_role={:?}",
        observe_save_dialog.foreground.window_title,
        observe_save_dialog.foreground.process_name,
        observe_save_dialog
            .focused
            .as_ref()
            .map(|focused| focused.role.as_str())
    );

    let save_path_text = target_path.to_string_lossy().into_owned();
    println!(
        "source_of_truth=mcp_act_type edge=filename before_path={} before_len={}",
        save_path_text,
        save_path_text.chars().count()
    );
    let filename = client
        .tools_call(
            "act_type",
            json!({
                "text": save_path_text,
                "dynamics": "linear",
                "linear_ms_per_char": 20
            }),
        )
        .await?;
    let filename_response: ActTypeWireResponse = structured(&filename)?;
    println!(
        "source_of_truth=mcp_act_type edge=filename after=ok:{} chars_typed:{} elapsed_ms:{}",
        filename_response.ok, filename_response.chars_typed, filename_response.elapsed_ms
    );
    assert!(filename_response.ok);

    println!("source_of_truth=mcp_act_press edge=confirm_save before=keys:[enter]");
    let confirm = client
        .tools_call("act_press", json!({"keys": ["enter"], "hold_ms": 33}))
        .await?;
    let confirm_response: ActPressWireResponse = structured(&confirm)?;
    println!(
        "source_of_truth=mcp_act_press edge=confirm_save after=ok:{} keys_pressed:{} backend_used:{} elapsed_ms:{}",
        confirm_response.ok,
        confirm_response.keys_pressed,
        confirm_response.backend_used,
        confirm_response.elapsed_ms
    );
    assert!(confirm_response.ok);

    let after_text = wait_for_file_text(&target_path, Duration::from_secs(5))?;
    let first_50 = after_text.chars().take(50).collect::<String>();
    println!(
        "source_of_truth=disk edge=happy after_bytes={}:{}",
        after_text.len(),
        first_50.escape_debug()
    );
    assert_eq!(after_text, EXPECTED_DISK_TEXT);

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    let contains_act_type = logs.contains("tool.invocation kind=act_type");
    let contains_act_press = logs.contains("tool.invocation kind=act_press");
    println!(
        "source_of_truth=daemon_log edge=happy after_bytes={} contains_act_type={} contains_act_press={}",
        logs.len(),
        contains_act_type,
        contains_act_press
    );
    assert!(contains_act_type);
    assert!(contains_act_press);

    handle.close()?;
    println!("source_of_truth=NotepadHandle::close edge=happy after=closed pid={pid}");
    std::fs::remove_file(&target_path)
        .with_context(|| format!("cleanup demo file {}", target_path.display()))?;
    println!(
        "source_of_truth=disk edge=cleanup after_exists={}",
        target_path.exists()
    );
    assert!(!target_path.exists());
    std::mem::forget(cleanup);
    Ok(())
}

#[cfg(windows)]
struct FileCleanup(PathBuf);

#[cfg(windows)]
impl Drop for FileCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(windows)]
async fn observe(client: &mut StdioMcpClient) -> anyhow::Result<Observation> {
    let response = client.tools_call("observe", json!({})).await?;
    structured(&response)
}

#[cfg(windows)]
fn editor_from_uia_snapshot(hwnd: i64) -> anyhow::Result<ElementId> {
    let window = synapse_a11y::window_from_hwnd(hwnd)
        .with_context(|| format!("resolve Notepad hwnd 0x{hwnd:x}"))?;
    let subtree = synapse_a11y::snapshot(&window, 4)
        .with_context(|| format!("snapshot Notepad hwnd 0x{hwnd:x}"))?;
    let target = subtree
        .nodes
        .iter()
        .filter(|node| node.enabled && node.bbox.w > 4 && node.bbox.h > 4)
        .max_by_key(|node| editor_score(node))
        .cloned()
        .with_context(|| {
            format!(
                "Notepad snapshot did not contain an editor-like element; nodes={}",
                summarize_nodes(&subtree.nodes)
            )
        })?;
    println!(
        "source_of_truth=synapse_a11y::snapshot edge=editor_fallback hwnd=0x{hwnd:x} node_count={} target={} role={:?} name={:?} bbox={:?} patterns={:?}",
        subtree.nodes.len(),
        target.element_id,
        target.role,
        target.name,
        target.bbox,
        target.patterns
    );
    Ok(target.element_id)
}

#[cfg(windows)]
fn is_editor_like(role: &str, patterns: &[UiaPattern]) -> bool {
    let role = role.to_ascii_lowercase();
    patterns.contains(&UiaPattern::Value)
        || patterns.contains(&UiaPattern::Text)
        || role.contains("edit")
        || role.contains("document")
        || role.contains("text")
}

#[cfg(windows)]
fn editor_score(node: &AccessibleNode) -> (bool, bool, bool, u32, i64) {
    let area = rect_area(node.bbox);
    (
        node.patterns.contains(&UiaPattern::Value),
        node.patterns.contains(&UiaPattern::Text),
        is_editor_like(&node.role, &node.patterns),
        node.depth,
        area,
    )
}

#[cfg(windows)]
fn rect_area(rect: Rect) -> i64 {
    i64::from(rect.w).saturating_mul(i64::from(rect.h))
}

#[cfg(windows)]
fn wait_for_file_text(path: &Path, timeout: Duration) -> anyhow::Result<String> {
    let start = Instant::now();
    let mut last_error = None;
    while start.elapsed() <= timeout {
        match std::fs::read_to_string(path) {
            Ok(text) => return Ok(text),
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(last_error.map_or_else(
        || anyhow::anyhow!("{} did not appear before timeout", path.display()),
        |error| anyhow::anyhow!("{} not readable before timeout: {error}", path.display()),
    ))
}

#[cfg(windows)]
fn summarize_nodes(nodes: &[AccessibleNode]) -> String {
    nodes
        .iter()
        .map(|node| {
            format!(
                "{{id:{},role:{:?},name:{:?},enabled:{},bbox:{:?},patterns:{:?}}}",
                node.element_id, node.role, node.name, node.enabled, node.bbox, node.patterns
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(windows)]
fn read_logs(path: &Path) -> anyhow::Result<String> {
    let mut logs = String::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            logs.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    Ok(logs)
}

#[cfg(windows)]
fn structured<T: DeserializeOwned>(resp: &Value) -> anyhow::Result<T> {
    serde_json::from_value(resp["structuredContent"].clone()).context("decode structuredContent")
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct ActClickWireResponse {
    ok: bool,
    used_invoke_pattern: bool,
    backend_used: String,
    elapsed_ms: u32,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct ActTypeWireResponse {
    ok: bool,
    chars_typed: u32,
    elapsed_ms: u32,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct ActPressWireResponse {
    ok: bool,
    keys_pressed: u32,
    elapsed_ms: u32,
    backend_used: String,
}
