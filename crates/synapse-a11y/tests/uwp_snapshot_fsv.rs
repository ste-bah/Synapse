//! Real-data Full-State-Verification for cross-process UWP accessibility
//! traversal. No mocks: this launches the real Windows Calculator (a
//! `Windows.UI.Core.CoreWindow`-hosted UWP app whose XAML content runs in a
//! separate process) and asserts that the production `synapse_a11y::snapshot`
//! reaches the in-process source-of-truth display element across the
//! cross-process boundary.
//!
//! Ignored by default because it requires an interactive Windows desktop
//! session. Run manually on a real host:
//!
//! ```text
//! cargo test -p synapse-a11y --test uwp_snapshot_fsv -- --ignored --nocapture
//! ```
//!
//! Before this fix, `snapshot` collapsed to depth 1 (4 nodes, no content)
//! whenever the cross-process walk exceeded a 25ms latency guard. The
//! regression this guards against: the display element disappearing from the
//! tree.

#![cfg(windows)]

use std::error::Error;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use uiautomation::{
    UIAutomation,
    types::{ElementMode, TreeScope, UIProperty},
    variants::Variant,
};

/// Resolve a top-level window HWND by its UIA Name among desktop-root children.
fn hwnd_for_window(name: &str) -> Result<Option<i64>, Box<dyn Error>> {
    let automation = UIAutomation::new().or_else(|_| UIAutomation::new_direct())?;
    let cache = automation.create_cache_request()?;
    cache.add_property(UIProperty::NativeWindowHandle)?;
    cache.set_tree_filter(automation.create_true_condition()?)?;
    cache.set_tree_scope(TreeScope::Element)?;
    cache.set_element_mode(ElementMode::Full)?;
    let root = automation.get_root_element_build_cache(&cache)?;
    let condition =
        automation.create_property_condition(UIProperty::Name, Variant::from(name), None)?;
    match root.find_first_build_cache(TreeScope::Children, &condition, &cache) {
        Ok(element) => {
            let handle: isize = element.get_native_window_handle()?.into();
            Ok(Some(handle as i64))
        }
        Err(_not_found) => Ok(None),
    }
}

#[test]
#[ignore = "requires an interactive Windows desktop session; run with --ignored"]
fn calculator_uwp_display_is_reachable_via_snapshot() -> Result<(), Box<dyn Error>> {
    // Launch Calculator if it is not already present.
    if hwnd_for_window("Calculator")?.is_none() {
        Command::new("cmd").args(["/C", "start", "", "calc.exe"]).spawn()?;
    }

    // Wait for the window to register in the UIA tree (bounded).
    let deadline = Instant::now() + Duration::from_secs(15);
    let hwnd = loop {
        if let Some(hwnd) = hwnd_for_window("Calculator")? {
            break hwnd;
        }
        if Instant::now() >= deadline {
            return Err("Calculator window did not appear in the UIA tree".into());
        }
        sleep(Duration::from_millis(250));
    };

    let root = synapse_a11y::window_from_hwnd(hwnd)?;
    let tree = synapse_a11y::snapshot(&root, 12)?;

    // Source of truth: the CoreWindow-hosted XAML display element, which lives
    // in a different process than the ApplicationFrameHost window.
    let display = tree.nodes.iter().find(|node| {
        node.automation_id.as_deref() == Some("CalculatorResults")
            || node.name.starts_with("Display is")
    });

    assert!(
        tree.nodes.len() > 10,
        "cross-process UWP content missing: snapshot only produced {} nodes (truncated={}); \
         the display element is hosted in a separate process and must be reached",
        tree.nodes.len(),
        tree.truncated,
    );
    assert!(
        display.is_some(),
        "Calculator display (source of truth) not found in {} snapshot nodes; \
         cross-process CoreWindow traversal regressed",
        tree.nodes.len(),
    );

    Ok(())
}
