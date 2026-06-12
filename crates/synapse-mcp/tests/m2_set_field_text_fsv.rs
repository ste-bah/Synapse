//! Full State Verification for `act_set_field_text` + `observe
//! include:["interactable"]` (#882) against a REAL Chrome (CDP web tier, and
//! the no-CDP Chromium foreground tier) and a REAL Notepad (native background
//! tier).
//!
//! Source of Truth per tier:
//! - web tier: the live DOM node value, read back separately via the
//!   observation pipeline's AX value (`elements[].value`) — a different read
//!   path than the tool's own `cdp_node_value` verification.
//! - foreground/native tiers: the UIA ValuePattern / Win32 window text
//!   readback, read separately through `synapse_a11y::element_value`.
//!
//! Every test prints before/after state for each action and is `#[ignore]`
//! because it needs an interactive Windows desktop.

#![cfg(windows)]

use std::{path::PathBuf, time::Duration};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::{AccessibleNode, Observation, UiaPattern};
use synapse_test_utils::{fixtures::launch_notepad, stdio_mcp_client::StdioMcpClient};
use tempfile::TempDir;

const CHROME_DEBUG_PORT: u16 = 9777;
const FORM_TITLE: &str = "Synapse 882 FSV Form";
const HAPPY_TITLE_TEXT: &str = "FSV-882 fresh title: youtube upload";
const COMPOSER_TEXT: &str = "FSV-882 composer replacement for a contenteditable host";

struct ChromeFixture {
    child: std::process::Child,
    _profile_dir: TempDir,
    _form_dir: TempDir,
}

impl Drop for ChromeFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn form_html() -> String {
    let noise: String = (0..30)
        .map(|index| {
            format!("<p>structural noise paragraph {index} that an agent never clicks</p>\n")
        })
        .collect();
    format!(
        r##"<!doctype html>
<html><head><title>{FORM_TITLE}</title></head>
<body>
<h1>Upload details</h1>
{noise}
<label>Title <input id="title" aria-label="Video title" value="old stale title"></label>
<label>Description <textarea id="desc" aria-label="Video description">old stale description</textarea></label>
<div id="composer" contenteditable="true" role="textbox" aria-multiline="true" aria-label="Composer">old composer text</div>
<button id="publish">Publish</button>
<a href="#help">Help</a>
</body></html>"##
    )
}

fn chrome_path() -> anyhow::Result<PathBuf> {
    for candidate in [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    anyhow::bail!("chrome.exe not found in default install locations");
}

fn launch_chrome_with_form() -> anyhow::Result<ChromeFixture> {
    let form_dir = TempDir::new()?;
    let form_path = form_dir.path().join("synapse-882-fsv-form.html");
    std::fs::write(&form_path, form_html())?;
    let profile_dir = TempDir::new()?;
    // Chrome 136+ refuses --remote-debugging-port on the default profile; a
    // dedicated --user-data-dir is required.
    let child = std::process::Command::new(chrome_path()?)
        .arg(format!("--remote-debugging-port={CHROME_DEBUG_PORT}"))
        .arg(format!("--user-data-dir={}", profile_dir.path().display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-fre")
        .arg("--window-size=1200,900")
        .arg(form_path.display().to_string())
        .spawn()
        .context("spawn chrome")?;
    Ok(ChromeFixture {
        child,
        _profile_dir: profile_dir,
        _form_dir: form_dir,
    })
}

fn focus_chrome_form_window(pid: u32) -> anyhow::Result<i64> {
    let title_regex = regex::Regex::new(&regex::escape(FORM_TITLE))?;
    let context = synapse_test_utils::fixtures::wait_for_window_title_regex(pid, &title_regex)?;
    synapse_a11y::focus_window_with_intent(
        context.hwnd,
        synapse_a11y::ForegroundActivationIntent::OperatorRequested {
            caller: "m2_set_field_text_fsv",
        },
    )?;
    Ok(context.hwnd)
}

async fn observe_with(client: &mut StdioMcpClient, args: Value) -> anyhow::Result<Observation> {
    let response = client.tools_call("observe", args).await?;
    structured(&response)
}

fn structured<T: DeserializeOwned>(resp: &Value) -> anyhow::Result<T> {
    serde_json::from_value(resp["structuredContent"].clone()).context("decode structuredContent")
}

fn element_named<'nodes>(
    nodes: &'nodes [AccessibleNode],
    role: &str,
    name_substring: &str,
) -> anyhow::Result<&'nodes AccessibleNode> {
    nodes
        .iter()
        .find(|node| {
            node.role.eq_ignore_ascii_case(role)
                && node
                    .name
                    .to_ascii_lowercase()
                    .contains(&name_substring.to_ascii_lowercase())
        })
        .with_context(|| {
            format!(
                "no {role} element with name containing {name_substring:?}; nodes={:?}",
                nodes
                    .iter()
                    .map(|node| (node.role.clone(), node.name.clone()))
                    .collect::<Vec<_>>()
            )
        })
}

async fn observed_value(
    client: &mut StdioMcpClient,
    role: &str,
    name_substring: &str,
) -> anyhow::Result<Option<String>> {
    let observation = observe_with(client, json!({"include": ["interactable"]})).await?;
    let node = element_named(&observation.elements, role, name_substring)?;
    Ok(node.value.clone())
}

#[tokio::test]
#[ignore = "requires an interactive Windows desktop with Chrome installed"]
#[allow(clippy::too_many_lines)]
async fn set_field_text_web_tier_and_interactable_observe_fsv_live() -> anyhow::Result<()> {
    let chrome = launch_chrome_with_form()?;
    let chrome_pid = chrome.child.id();
    let hwnd = focus_chrome_form_window(chrome_pid)?;
    println!(
        "readback=chrome edge=launch after_hwnd=0x{hwnd:x} pid={chrome_pid} port={CHROME_DEBUG_PORT}"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let port_value = CHROME_DEBUG_PORT.to_string();
    let mut client = StdioMcpClient::launch_and_init_with_env(
        None,
        &[("SYNAPSE_CDP_PORTS", port_value.as_str())],
    )
    .await?;

    // ---- observe include:["interactable"]: lean shape, suppressed diagnostics
    let full = observe_with(&mut client, json!({})).await?;
    let lean = observe_with(&mut client, json!({"include": ["interactable"]})).await?;
    println!(
        "readback=observe edge=interactable before=full_elements:{} full_tokens:{} after=lean_elements:{} lean_tokens:{} lean_roles={:?}",
        full.elements.len(),
        full.diagnostics.size_estimate_tokens,
        lean.elements.len(),
        lean.diagnostics.size_estimate_tokens,
        lean.elements
            .iter()
            .map(|node| (node.role.clone(), node.name.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        lean.diagnostics.input_backends.is_none(),
        "interactable observe must suppress input_backends"
    );
    assert!(
        lean.diagnostics.capture_config.is_none() && lean.diagnostics.capture_runtime.is_none(),
        "interactable observe must suppress capture diagnostics blocks"
    );
    assert!(
        lean.diagnostics.cdp.is_none(),
        "interactable observe must suppress the cdp diagnostics block"
    );
    assert!(
        lean.diagnostics.size_estimate_tokens < full.diagnostics.size_estimate_tokens,
        "interactable observe must be smaller than the default observe"
    );
    for node in &lean.elements {
        assert!(
            synapse_perception::is_interactable_node(node),
            "non-interactable node leaked through the filter: {} {:?}",
            node.role,
            node.name
        );
    }
    let title_node = element_named(&lean.elements, "textbox", "Video title")?.clone();
    let desc_node = element_named(&lean.elements, "textbox", "Video description")?.clone();
    let composer_node = element_named(&lean.elements, "textbox", "Composer")?.clone();
    let button_node = element_named(&lean.elements, "button", "Publish")?.clone();
    assert!(
        title_node.element_id.to_string().contains("cdcd"),
        "web nodes must carry CDP element ids; got {}",
        title_node.element_id
    );

    // ---- happy path: replace the prefilled title
    println!(
        "readback=set_field_text edge=happy before_value={:?} requested={HAPPY_TITLE_TEXT:?}",
        title_node.value
    );
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": title_node.element_id.to_string(), "text": HAPPY_TITLE_TEXT}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!("readback=set_field_text edge=happy after_response={body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["backend_tier_used"], "cdp");
    assert_eq!(body["required_foreground"], false);
    assert_eq!(body["source_of_truth"], "cdp_node.value");
    assert_eq!(body["changed"], true);
    let separate = observed_value(&mut client, "textbox", "Video title").await?;
    println!("readback=set_field_text edge=happy separate_observe_value={separate:?}");
    assert_eq!(separate.as_deref(), Some(HAPPY_TITLE_TEXT));

    // ---- edge 1: empty text clears a prefilled textarea
    println!(
        "readback=set_field_text edge=empty before_value={:?} requested=\"\"",
        desc_node.value
    );
    assert!(
        desc_node
            .value
            .as_deref()
            .unwrap_or("")
            .contains("old stale")
    );
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": desc_node.element_id.to_string(), "text": ""}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!("readback=set_field_text edge=empty after_response={body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["after_len"], 0);
    let separate = observed_value(&mut client, "textbox", "Video description").await?;
    println!("readback=set_field_text edge=empty separate_observe_value={separate:?}");
    assert!(separate.as_deref().unwrap_or("").is_empty());

    // ---- edge 2: long replacement (2000 chars) lands byte-correct
    let long_text: String = "synapse-882-long-segment ".repeat(80);
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": desc_node.element_id.to_string(), "text": long_text}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!(
        "readback=set_field_text edge=long after=ok:{} requested_len:{} after_len:{}",
        body["ok"], body["requested_len"], body["after_len"]
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["after_len"], json!(long_text.chars().count()));
    let separate = observed_value(&mut client, "textbox", "Video description").await?;
    assert_eq!(separate.as_deref(), Some(long_text.as_str()));

    // ---- edge 3: non-editable target fails closed, field untouched
    let error = client
        .tools_call_error(
            "act_set_field_text",
            json!({"element_id": button_node.element_id.to_string(), "text": "must not land"}),
        )
        .await?;
    println!("readback=set_field_text edge=non_editable after_error={error}");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("not an editable web node"),
        "non-editable refusal must name the select-all readback; got {message}"
    );
    let title_after_refusal = observed_value(&mut client, "textbox", "Video title").await?;
    println!("readback=set_field_text edge=non_editable title_still={title_after_refusal:?}");
    assert_eq!(title_after_refusal.as_deref(), Some(HAPPY_TITLE_TEXT));

    // ---- contenteditable composer (the LinkedIn-composer shape)
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": composer_node.element_id.to_string(), "text": COMPOSER_TEXT}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!("readback=set_field_text edge=contenteditable after_response={body}");
    assert_eq!(body["ok"], true);
    let separate = observed_value(&mut client, "textbox", "Composer").await?;
    println!("readback=set_field_text edge=contenteditable separate_observe_value={separate:?}");
    assert_eq!(separate.as_deref(), Some(COMPOSER_TEXT));

    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}

/// Chromium UIA editable WITHOUT CDP (the YouTube/LinkedIn session shape from
/// #882): the daemon cannot reach a debug endpoint, so `act_set_field_text`
/// must take the leased foreground tier — scroll the field into view, click
/// it, Ctrl+A, type, then verify with a separate UIA value readback.
#[tokio::test]
#[ignore = "requires an interactive Windows desktop with Chrome installed; takes the real foreground"]
async fn set_field_text_chromium_foreground_tier_fsv_live() -> anyhow::Result<()> {
    let form_dir = TempDir::new()?;
    let form_path = form_dir.path().join("synapse-882-fsv-foreground.html");
    std::fs::write(&form_path, form_html())?;
    let profile_dir = TempDir::new()?;
    // No --remote-debugging-port: CDP is intentionally unreachable.
    let child = std::process::Command::new(chrome_path()?)
        .arg(format!("--user-data-dir={}", profile_dir.path().display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--window-size=1200,900")
        .arg(form_path.display().to_string())
        .spawn()
        .context("spawn chrome without debug port")?;
    let mut chrome = ChromeFixture {
        child,
        _profile_dir: profile_dir,
        _form_dir: form_dir,
    };
    let hwnd = focus_chrome_form_window(chrome.child.id())?;
    println!("readback=chrome edge=launch_no_cdp after_hwnd=0x{hwnd:x}");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let mut client = StdioMcpClient::launch_and_init().await?;
    let lean = observe_with(&mut client, json!({"include": ["interactable"]})).await?;
    println!(
        "readback=observe edge=uia_interactable after_roles={:?}",
        lean.elements
            .iter()
            .map(|node| (node.role.clone(), node.name.clone()))
            .collect::<Vec<_>>()
    );
    let title_node = element_named(&lean.elements, "edit", "Video title")?.clone();
    assert!(
        !title_node.element_id.to_string().contains("cdcd"),
        "without a debug port the title field must be a UIA element; got {}",
        title_node.element_id
    );

    let replacement = "FSV-882 foreground tier replacement";
    println!(
        "readback=set_field_text edge=foreground_happy before_value={:?} requested={replacement:?}",
        title_node.value
    );
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": title_node.element_id.to_string(), "text": replacement}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!("readback=set_field_text edge=foreground_happy after_response={body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["backend_tier_used"], "foreground_keys");
    assert_eq!(body["required_foreground"], true);
    assert_eq!(body["method"], "foreground_click_select_all_type");
    // Separate UIA Source-of-Truth read (not the tool's own readback):
    let after = synapse_a11y::element_value(&title_node.element_id)?;
    println!(
        "readback=set_field_text edge=foreground_happy separate_uia_value={:?}",
        after.value
    );
    assert_eq!(after.value, replacement);

    // Edge: a backgrounded target window fails closed and points at
    // act_focus_window instead of stealing the foreground (epic #771).
    let notepad = launch_notepad()?;
    synapse_a11y::focus_window_with_intent(
        notepad.hwnd(),
        synapse_a11y::ForegroundActivationIntent::OperatorRequested {
            caller: "m2_set_field_text_fsv_background_edge",
        },
    )?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let foreground_now = synapse_a11y::current_foreground_context()?;
    println!(
        "readback=foreground edge=stolen_by_notepad notepad_hwnd=0x{:x} live_foreground_hwnd=0x{:x} live_foreground_process={}",
        notepad.hwnd(),
        foreground_now.hwnd,
        foreground_now.process_name
    );
    anyhow::ensure!(
        foreground_now.hwnd != hwnd,
        "edge-case setup failed: Chrome is still the foreground window"
    );
    let error = client
        .tools_call_error(
            "act_set_field_text",
            json!({"element_id": title_node.element_id.to_string(), "text": "must not land"}),
        )
        .await?;
    println!("readback=set_field_text edge=target_backgrounded after_error={error}");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("act_focus_window"),
        "backgrounded-target refusal must direct callers to act_focus_window; got {message}"
    );
    let untouched = synapse_a11y::element_value(&title_node.element_id)?;
    println!(
        "readback=set_field_text edge=target_backgrounded separate_uia_value={:?}",
        untouched.value
    );
    assert_eq!(untouched.value, replacement);
    notepad.close()?;

    let status = client.shutdown().await?;
    assert!(status.success());
    let _ = chrome.child.kill();
    Ok(())
}

#[tokio::test]
#[ignore = "requires an interactive Windows desktop with Notepad and UIA"]
async fn set_field_text_native_tier_notepad_fsv_live() -> anyhow::Result<()> {
    let handle = launch_notepad()?;
    let hwnd = handle.hwnd();
    println!(
        "readback=notepad edge=launch after_hwnd=0x{hwnd:x} pid={}",
        handle.pid()
    );
    let mut client = StdioMcpClient::launch_and_init().await?;

    let subtree = synapse_a11y::snapshot_window_from_hwnd(hwnd, 4)?;
    let editor = subtree
        .nodes
        .iter()
        .filter(|node| node.enabled && node.bbox.w > 4 && node.bbox.h > 4)
        .max_by_key(|node| {
            (
                node.patterns.contains(&UiaPattern::Value),
                node.patterns.contains(&UiaPattern::Text),
                node.depth,
            )
        })
        .cloned()
        .context("Notepad snapshot had no editor-like element")?;
    println!(
        "readback=notepad edge=editor element_id={} role={} patterns={:?}",
        editor.element_id, editor.role, editor.patterns
    );

    let replacement = "FSV-882 native tier replacement text";
    let before = synapse_a11y::element_value(&editor.element_id)?;
    println!(
        "readback=set_field_text edge=native_happy before_len={} requested={replacement:?}",
        before.value.chars().count()
    );
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": editor.element_id.to_string(), "text": replacement}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!("readback=set_field_text edge=native_happy after_response={body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["required_foreground"], false);
    // Separate Source-of-Truth read, not the tool's own response:
    let after = synapse_a11y::element_value(&editor.element_id)?;
    println!(
        "readback=set_field_text edge=native_happy separate_uia_value={:?}",
        after.value
    );
    assert_eq!(after.value, replacement);

    // Edge: empty text clears the editor through the same primitive.
    let response = client
        .tools_call(
            "act_set_field_text",
            json!({"element_id": editor.element_id.to_string(), "text": ""}),
        )
        .await?;
    let body: Value = structured(&response)?;
    println!("readback=set_field_text edge=native_clear after_response={body}");
    assert_eq!(body["ok"], true);
    let cleared = synapse_a11y::element_value(&editor.element_id)?;
    println!(
        "readback=set_field_text edge=native_clear separate_uia_value={:?}",
        cleared.value
    );
    assert!(cleared.value.is_empty());

    let status = client.shutdown().await?;
    assert!(status.success());
    handle.close()?;
    Ok(())
}
