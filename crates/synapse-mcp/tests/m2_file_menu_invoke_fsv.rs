//! #170 — Win-only FSV E2E: invoke Notepad's `File` menu via `act_click` with
//! `use_invoke_pattern: true` and read back `ExpandCollapsePattern.expand_state`
//! to confirm it flipped to `Expanded` with no observable cursor motion.
//!
//! This test does not depend on `launch_notepad()` (and therefore does not
//! require an `Untitled - Notepad` window) because Win11 22H2+ packaged Notepad
//! aggressively restores the last session's tabs and exhibits focus-stealing-
//! prevention behaviour that makes a fresh `Untitled` tab unreliable to obtain
//! from a child process. Any visible Notepad window has the same `File` menu;
//! we use whichever one we can find or spawn.

use anyhow::Context;
use serde_json::{Value, json};
#[cfg(windows)]
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[cfg(windows)]
use synapse_a11y::ExpandState;
#[cfg(windows)]
use synapse_core::{AccessibleNode, Point, UiaPattern};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[cfg(windows)]
static WINDOWS_NOTEPAD_FILE_MENU_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

#[cfg(windows)]
const INVOKE_LATENCY_BUDGET_MS: u32 = 25;
#[cfg(windows)]
const NOTEPAD_WAIT: Duration = Duration::from_secs(8);
#[cfg(windows)]
const POLL: Duration = Duration::from_millis(75);

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires an interactive Windows desktop with Notepad and UIA"]
#[allow(clippy::too_many_lines)] // FSV body intentionally keeps every step in order
async fn act_click_invoke_pattern_file_menu_expands_without_cursor_motion_fsv()
-> anyhow::Result<()> {
    let _notepad_lock = WINDOWS_NOTEPAD_FILE_MENU_LOCK.lock().await;
    let log_dir = TempDir::new()?;

    // Find an existing visible Notepad window, or spawn a fresh one and wait
    // for any window owned by a Notepad process to appear.
    let (hwnd, pid, launched_child) = find_or_spawn_notepad_window()?;
    println!(
        "source_of_truth=notepad_source edge=file_menu_invoke after=hwnd:0x{hwnd:x} pid:{pid} launched_fresh:{}",
        launched_child.is_some()
    );

    if let Err(error) = synapse_a11y::focus_window(hwnd) {
        println!(
            "source_of_truth=synapse_a11y::focus_window edge=file_menu_invoke after_error={error}"
        );
    } else {
        println!(
            "source_of_truth=synapse_a11y::focus_window edge=file_menu_invoke after=ok hwnd=0x{hwnd:x}"
        );
    }

    // Packaged Notepad's MenuBar is virtualised out of the ControlView TreeWalker
    // until the user has interacted with it. Press Alt to populate the tree
    // without expanding any menu, then snapshot.
    let mut client = StdioMcpClient::launch_and_init_with_log_dir(Some(log_dir.path())).await?;
    let _ = client
        .tools_call("act_press", json!({"keys": ["alt"]}))
        .await?;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let window = synapse_a11y::window_from_hwnd(hwnd)
        .with_context(|| format!("resolve Notepad hwnd 0x{hwnd:x}"))?;
    let subtree = synapse_a11y::snapshot(&window, 6)
        .with_context(|| format!("snapshot Notepad hwnd 0x{hwnd:x}"))?;
    let file_menu_node = find_file_menu_node(&subtree.nodes).with_context(|| {
        format!(
            "Notepad snapshot did not contain a `File` ExpandCollapse node; nodes={}",
            summarize_nodes(&subtree.nodes)
        )
    })?;
    println!(
        "source_of_truth=synapse_a11y::snapshot edge=file_menu_invoke before=pid:{pid} hwnd:0x{hwnd:x} node_count:{} target:{} role:{:?} name:{:?} bbox:{:?} patterns:{:?}",
        subtree.nodes.len(),
        file_menu_node.element_id,
        file_menu_node.role,
        file_menu_node.name,
        file_menu_node.bbox,
        file_menu_node.patterns
    );

    let element_id_str = file_menu_node.element_id.to_string();
    let file_menu_element = synapse_a11y::re_resolve(&file_menu_node.element_id)
        .with_context(|| format!("re-resolve File menu element {element_id_str}"))?;
    let before_state = synapse_a11y::expand_state_of(&file_menu_element)
        .with_context(|| format!("read pre-invoke ExpandCollapse state of {element_id_str}"))?;
    let before_cursor = synapse_action::backend::software::cursor_position()?;
    println!(
        "source_of_truth=uia_expand edge=file_menu_invoke before=state:{before_state:?} cursor:{before_cursor:?}"
    );
    if before_state == ExpandState::Expanded {
        anyhow::bail!(
            "File menu was already Expanded before the test ran ({before_state:?}); cannot prove the invoke flipped state"
        );
    }

    let click = client
        .tools_call(
            "act_click",
            json!({
                "target": {"element_id": element_id_str},
                "use_invoke_pattern": true,
            }),
        )
        .await?;
    let click_response: ClickWireResponse = structured(&click)?;
    let after_cursor = synapse_action::backend::software::cursor_position()?;
    println!(
        "source_of_truth=mcp_act_click edge=file_menu_invoke after=ok:{} used_invoke_pattern:{} backend_used:{} elapsed_ms:{} cursor:{after_cursor:?}",
        click_response.ok,
        click_response.used_invoke_pattern,
        click_response.backend_used,
        click_response.elapsed_ms,
    );
    assert!(click_response.ok, "act_click on File menu should succeed");

    // Re-look up File menu by name after invoke — the menu Flyout may have made
    // the runtime id change, so we look up by name+ExpandCollapse again.
    let subtree_after = synapse_a11y::snapshot(&window, 6)
        .context("post-invoke File menu snapshot")?;
    let file_after = find_file_menu_node(&subtree_after.nodes)
        .context("post-invoke snapshot missing File menu node")?;
    let file_after_element = synapse_a11y::re_resolve(&file_after.element_id)
        .context("re-resolve File menu post-invoke")?;
    let after_state = synapse_a11y::expand_state_of(&file_after_element)
        .context("read post-invoke ExpandCollapse state")?;
    println!(
        "source_of_truth=uia_expand edge=file_menu_invoke after=state:{after_state:?} before_state:{before_state:?} cursor:{after_cursor:?}"
    );

    assert_eq!(
        after_state,
        ExpandState::Expanded,
        "ExpandCollapseState should be Expanded after act_click(use_invoke_pattern=true); got {after_state:?} (was {before_state:?})"
    );
    assert_no_cursor_motion(before_cursor, after_cursor)?;
    assert!(
        click_response.elapsed_ms <= INVOKE_LATENCY_BUDGET_MS * 4,
        "elapsed {} ms exceeds 4× InvokePattern budget ({}ms)",
        click_response.elapsed_ms,
        INVOKE_LATENCY_BUDGET_MS
    );

    // Cleanup: collapse the menu (Esc), shut MCP, only kill the Notepad we spawned.
    let _ = client
        .tools_call("act_press", json!({"keys": ["escape"]}))
        .await?;
    assert!(client.shutdown().await?.success());
    if let Some(mut child) = launched_child {
        // We spawned this process; do a best-effort kill of the launcher PID.
        let _ = child.kill();
        let _ = child.wait();
        println!(
            "source_of_truth=Notepad_launched_child edge=file_menu_invoke after=killed"
        );
    } else {
        println!(
            "source_of_truth=Notepad_pre_existing edge=file_menu_invoke after=preserved pid={pid}"
        );
    }
    Ok(())
}

#[cfg(windows)]
fn find_or_spawn_notepad_window() -> anyhow::Result<(i64, u32, Option<std::process::Child>)> {
    if let Some((hwnd, pid)) = find_visible_notepad_window()? {
        return Ok((hwnd, pid, None));
    }
    let child = Command::new(notepad_exe_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn notepad.exe")?;
    let start = Instant::now();
    while start.elapsed() <= NOTEPAD_WAIT {
        if let Some((hwnd, pid)) = find_visible_notepad_window()? {
            return Ok((hwnd, pid, Some(child)));
        }
        thread::sleep(POLL);
    }
    anyhow::bail!("no visible Notepad window appeared within {NOTEPAD_WAIT:?} after spawn")
}

#[cfg(windows)]
fn find_visible_notepad_window() -> anyhow::Result<Option<(i64, u32)>> {
    let contexts = synapse_a11y::visible_top_level_window_contexts()?;
    Ok(contexts
        .into_iter()
        .find(|ctx| ctx.process_name.eq_ignore_ascii_case("Notepad.exe"))
        .map(|ctx| (ctx.hwnd, ctx.pid)))
}

#[cfg(windows)]
fn notepad_exe_path() -> std::path::PathBuf {
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let candidate = std::path::PathBuf::from(system_root)
            .join("System32")
            .join("notepad.exe");
        if candidate.exists() {
            return candidate;
        }
    }
    std::path::PathBuf::from("notepad.exe")
}

#[cfg(windows)]
fn find_file_menu_node(nodes: &[AccessibleNode]) -> Option<AccessibleNode> {
    nodes
        .iter()
        .find(|node| {
            node.enabled
                && node.name.eq_ignore_ascii_case("File")
                && node.patterns.contains(&UiaPattern::ExpandCollapse)
        })
        .cloned()
        .or_else(|| {
            nodes
                .iter()
                .find(|node| {
                    node.enabled
                        && node.name.to_ascii_lowercase().contains("file")
                        && node.patterns.contains(&UiaPattern::ExpandCollapse)
                })
                .cloned()
        })
}

#[cfg(windows)]
fn assert_no_cursor_motion(before: Point, after: Point) -> anyhow::Result<()> {
    if before != after {
        anyhow::bail!(
            "cursor moved during InvokePattern-only click: before={before:?} after={after:?}"
        );
    }
    Ok(())
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

#[derive(serde::Deserialize)]
struct ClickWireResponse {
    ok: bool,
    used_invoke_pattern: bool,
    backend_used: String,
    elapsed_ms: u32,
}

fn structured<T: serde::de::DeserializeOwned>(resp: &Value) -> anyhow::Result<T> {
    let structured = resp
        .get("structuredContent")
        .context("structuredContent missing on tool response")?;
    Ok(serde_json::from_value(structured.clone())?)
}

#[cfg(not(windows))]
#[allow(dead_code)]
const _: () = {
    let _ = StdioMcpClient::launch_and_init_with_log_dir;
    let _: Option<TempDir> = None;
};
