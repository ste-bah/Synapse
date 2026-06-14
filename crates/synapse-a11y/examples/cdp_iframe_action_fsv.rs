//! Full State Verification for #982: drive the FIXED CDP action path
//! (`get_page_with_discovery`) against a LIVE Chrome whose form lives in a
//! nested same-origin iframe, proving a write to an iframe-owned node now
//! succeeds where it previously failed with
//! "selected target … is no longer present for backendNodeId N".
//!
//! Usage (Windows, against a CDP-debug Chrome): run the
//! `cdp_iframe_action_fsv` example with four args — the http CDP endpoint, the
//! page `targetId`, the `backendNodeId`, and the replacement text.
//!
//! Source of Truth: the node's own `value`, read back via a SEPARATE
//! `cdp_node_value` call after the write — not the write's return value.

#[cfg(not(windows))]
fn main() {
    eprintln!("cdp_iframe_action_fsv is Windows-only (chromiumoxide CDP path)");
}

#[cfg(windows)]
fn main() {
    use synapse_a11y::{cdp_node_value, cdp_set_node_text};

    let mut args = std::env::args().skip(1);
    let endpoint = args.next().unwrap_or_else(|| {
        panic!("arg1=endpoint required, e.g. http://127.0.0.1:57278");
    });
    let target_id = args
        .next()
        .unwrap_or_else(|| panic!("arg2=page target id required"));
    let backend_node_id: i64 = args
        .next()
        .unwrap_or_else(|| panic!("arg3=backendNodeId required"))
        .parse()
        .unwrap_or_else(|err| panic!("backendNodeId must be an integer: {err}"));
    let text = args.next().unwrap_or_else(|| "SYNAPSE982-FIXED".to_owned());
    let title_hint = "SYNAPSE982 iframe repro - Google Chrome";

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|err| panic!("build tokio runtime: {err}"));

    runtime.block_on(async move {
        println!(
            "readback=cdp_iframe_fsv stage=params endpoint={endpoint} target={target_id} backendNodeId={backend_node_id} text={text:?}"
        );

        // BEFORE: read the node's current value (Source of Truth) through the
        // same fixed discovery path.
        let before = cdp_node_value(&endpoint, title_hint, Some(&target_id), backend_node_id)
            .await
            .unwrap_or_else(|err| panic!("FAILED before-read (fix regressed): {err}"));
        println!("readback=cdp_iframe_fsv stage=before node_value={before:?}");

        // EXECUTE: replace the iframe-owned node's text.
        let write = cdp_set_node_text(&endpoint, title_hint, Some(&target_id), backend_node_id, &text)
            .await
            .unwrap_or_else(|err| panic!("FAILED write (fix regressed): {err}"));
        println!(
            "readback=cdp_iframe_fsv stage=write selection_mode={} cleared_with_delete={}",
            write.selection_mode, write.cleared_with_delete
        );

        // AFTER: re-read the Source of Truth and prove the write landed.
        let after = cdp_node_value(&endpoint, title_hint, Some(&target_id), backend_node_id)
            .await
            .unwrap_or_else(|err| panic!("FAILED after-read (fix regressed): {err}"));
        println!("readback=cdp_iframe_fsv stage=after node_value={after:?}");

        assert_eq!(
            after, text,
            "iframe node value must equal the written text after the fix; before={before:?}"
        );
        println!("readback=cdp_iframe_fsv stage=verdict PASS before={before:?} after={after:?}");
    });
}
