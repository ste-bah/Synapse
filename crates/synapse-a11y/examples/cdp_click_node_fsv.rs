//! #982 FSV for the CLICK path: call the FIXED `cdp_click_node` (which routes
//! through `get_page_with_discovery`) on an iframe-owned button and prove it
//! actuates the in-process iframe handler by reading a window counter the
//! button's onclick increments. Source of Truth: `window[<var>]` in the
//! button's OWN frame, read separately after the click.
//!
//! args: endpoint, page targetId, button backendNodeId, window-counter var

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use chromiumoxide::Browser;
    use chromiumoxide::cdp::browser_protocol::dom::{
        BackendNodeId, GetDocumentParams, ResolveNodeParams,
    };
    use chromiumoxide::cdp::browser_protocol::target::TargetId;
    use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
    use futures_util::StreamExt as _;
    use synapse_a11y::{CdpMouseButton, cdp_click_node};

    let mut a = std::env::args().skip(1);
    let endpoint = a.next().expect("endpoint");
    let target_id = a.next().expect("targetId");
    let bn: i64 = a.next().expect("bn").parse().expect("int");
    let var = a.next().unwrap_or_else(|| "__cclicks".to_owned());
    let title_hint = "SYNAPSE982 iframe repro - Google Chrome";

    let read_counter = {
        let endpoint = endpoint.clone();
        let target_id = target_id.clone();
        let var = var.clone();
        move || {
            let endpoint = endpoint.clone();
            let target_id = target_id.clone();
            let var = var.clone();
            async move {
                let (browser, mut handler) = Browser::connect(&endpoint).await.expect("connect");
                let _h = tokio::spawn(async move { while handler.next().await.is_some() {} });
                let mut page = None;
                for _ in 0..30 {
                    if let Ok(p) = browser.get_page(TargetId::new(target_id.clone())).await {
                        page = Some(p);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                let page = page.expect("page");
                let _ = page
                    .execute(GetDocumentParams::builder().depth(-1).pierce(true).build())
                    .await;
                let r = page
                    .execute(
                        ResolveNodeParams::builder()
                            .backend_node_id(BackendNodeId::new(bn))
                            .build(),
                    )
                    .await
                    .expect("resolve");
                let oid = r.object.object_id.clone().expect("oid");
                let f = format!(
                    "function(){{return String((this.ownerDocument.defaultView||{{}})[{var:?}])}}"
                );
                let call = CallFunctionOnParams::builder()
                    .function_declaration(f)
                    .object_id(oid)
                    .return_by_value(true)
                    .build()
                    .expect("call");
                let v = page.execute(call).await.expect("eval");
                v.result
                    .result
                    .value
                    .clone()
                    .and_then(|x| x.as_str().map(ToOwned::to_owned))
                    .unwrap_or_default()
            }
        }
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async move {
        let before = read_counter().await;
        println!("readback=cdp_click_node_fsv stage=before {var}={before:?}");

        let point = cdp_click_node(&endpoint, title_hint, Some(&target_id), bn, CdpMouseButton::Left, 1)
            .await
            .unwrap_or_else(|err| panic!("cdp_click_node FAILED (fix regressed): {err}"));
        println!(
            "readback=cdp_click_node_fsv stage=click dispatched_at=({:.1},{:.1})",
            point.x, point.y
        );

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let after = read_counter().await;
        println!("readback=cdp_click_node_fsv stage=after {var}={after:?}");
        assert_eq!(
            after, "1",
            "iframe button onclick must have incremented {var} from undefined to 1 (before={before:?})"
        );
        println!("readback=cdp_click_node_fsv stage=verdict PASS before={before:?} after={after:?}");
    });
}
