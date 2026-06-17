//! #1110–#1120 FSV: drive the REAL `cdp_locate` selector engine against a live
//! Chrome on a synthetic page with KNOWN content, and prove every engine
//! resolves the exact node(s). Source of Truth: the live DOM — each resolved
//! `backendNodeId` is independently read back to its element `id` via a separate
//! `resolveNode` + `callFunctionOn` and compared to the element we KNOW the query
//! should select. Then the resolved id drives a REAL `cdp_click_node` and we read
//! the onclick counter the page increments.
//!
//! Usage (a Chromium must be listening with --remote-debugging-port):
//!   cargo run -p synapse-a11y --example cdp_selector_engine_fsv -- http://127.0.0.1:9222
#![allow(clippy::expect_used, clippy::too_many_lines, clippy::print_stdout)]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use chromiumoxide::Browser;
    use chromiumoxide::cdp::browser_protocol::dom::{
        BackendNodeId, GetDocumentParams, ResolveNodeParams,
    };
    use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
    use futures_util::StreamExt as _;
    use synapse_a11y::{
        CdpLayoutRelation, CdpLocateEngine, CdpLocateRequest, CdpMouseButton, cdp_click_node,
        cdp_locate,
    };

    // A page that exercises every engine and edge case. Every assertable node
    // carries a stable `id` so we can read the resolution back independently.
    const PAGE_HTML: &str = r"
      <h2 id='heading2'>Section Title</h2>
      <button id='btn-apply'>Apply</button>
      <button id='btn-apply2'><span id='apply-span'>Apply</span></button>
      <button id='btn-submit' data-testid='submit-btn'
              onclick='window.__synapse_clicks=(window.__synapse_clicks||0)+1'>Submit Order</button>
      <input id='inp-email' placeholder='Email address'>
      <label for='inp-name'>Full Name</label><input id='inp-name'>
      <input id='inp-aria' aria-label='Search query'>
      <img id='img-logo' alt='Company Logo' src='data:,'>
      <span id='sp-title' title='Tooltip Here'>hover me</span>
      <a class='lnk' id='lnk-1' href='#'>Read more</a>
      <a class='lnk' id='lnk-2' href='#'>Read more</a>
      <input type='checkbox' id='cb-on' checked>
      <input type='checkbox' id='cb-off'>
      <div id='ti-custom' data-test='alt-attr'>custom testid</div>
      <div id='anchor-box' style='position:absolute;left:100px;top:300px;width:50px;height:20px;'>Anchor</div>
      <button class='lay' id='left-btn' style='position:absolute;left:20px;top:300px;width:40px;height:20px;'>L</button>
      <button class='lay' id='right-btn' style='position:absolute;left:220px;top:300px;width:40px;height:20px;'>R</button>
      <div id='card'><button id='incard' class='nested'>InsideCard</button></div>
    ";

    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:9222".to_owned());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");

    rt.block_on(async move {
        let (browser, mut handler) = Browser::connect(&endpoint).await.expect("connect to Chrome");
        let _drive = tokio::spawn(async move { while handler.next().await.is_some() {} });

        let page = browser.new_page("about:blank").await.expect("new page");
        let target_id = page.target_id().inner().clone();
        page.evaluate(format!(
            "document.title='Synapse Selector FSV'; document.body.innerHTML = `{PAGE_HTML}`;"
        ))
        .await
        .expect("inject synthetic page");
        // Prime the full document so resolveNode sees every backend node.
        let _ = page
            .execute(GetDocumentParams::builder().depth(-1).pierce(true).build())
            .await;
        println!("readback=fsv stage=setup target={target_id} url=about:blank title='Synapse Selector FSV'");

        // Independent Source-of-Truth read: backendNodeId -> element.id.
        let id_of = |backend: i64| {
            let page = page.clone();
            async move {
                let resolved = page
                    .execute(
                        ResolveNodeParams::builder()
                            .backend_node_id(BackendNodeId::new(backend))
                            .build(),
                    )
                    .await
                    .expect("resolveNode");
                let object_id = resolved.object.object_id.clone().expect("objectId");
                let call = CallFunctionOnParams::builder()
                    .function_declaration("function(){ return this.id || this.tagName.toLowerCase(); }".to_owned())
                    .object_id(object_id)
                    .return_by_value(true)
                    .build()
                    .expect("call build");
                page.execute(call)
                    .await
                    .expect("read id")
                    .result
                    .result
                    .value
                    .and_then(|v| v.as_str().map(ToOwned::to_owned))
                    .unwrap_or_default()
            }
        };

        let req = |engine: CdpLocateEngine, query: &str| CdpLocateRequest {
            engine,
            query: query.to_owned(),
            limit: 50,
            ..Default::default()
        };

        // Runs a request, reads back the resolved ids, and asserts the set + count.
        let mut pass = 0u32;
        macro_rules! check {
            ($label:expr, $request:expr, $expected_ids:expr, $expected_count:expr) => {{
                let request: CdpLocateRequest = $request;
                let result = cdp_locate(&endpoint, &target_id, request)
                    .await
                    .unwrap_or_else(|e| panic!("[{}] cdp_locate FAILED: {e}", $label));
                let mut got = Vec::new();
                for b in &result.backend_node_ids {
                    got.push(id_of(*b).await);
                }
                let expected: Vec<String> = $expected_ids.iter().map(|s: &&str| (*s).to_owned()).collect();
                println!(
                    "readback=fsv check='{}' match_count={} expected_count={} got_ids={:?} expected_ids={:?}",
                    $label, result.match_count, $expected_count, got, expected
                );
                assert_eq!(result.match_count, $expected_count, "[{}] match_count", $label);
                assert_eq!(got, expected, "[{}] resolved ids (order-sensitive)", $label);
                pass += 1;
                result
            }};
        }

        // ---- #1111 CSS ----
        check!("css.unique", req(CdpLocateEngine::Css, "#btn-apply"), ["btn-apply"], 1);
        check!("css.multiple", req(CdpLocateEngine::Css, "a.lnk"), ["lnk-1", "lnk-2"], 2);
        check!("css.nomatch", req(CdpLocateEngine::Css, "#does-not-exist"), [] as [&str; 0], 0);

        // ---- #1112 XPath ----
        check!("xpath.attr", req(CdpLocateEngine::Xpath, "//button[@id='btn-submit']"), ["btn-submit"], 1);
        check!("xpath.text", req(CdpLocateEngine::Xpath, "//a[contains(.,'Read more')]"), ["lnk-1", "lnk-2"], 2);
        check!("xpath.nomatch", req(CdpLocateEngine::Xpath, "//table"), [] as [&str; 0], 0);

        // ---- #1113 Text (substring / exact / regex) ----
        // substring "apply" (case-insensitive) -> deepest matches: btn-apply + apply-span.
        check!("text.substring", req(CdpLocateEngine::Text, "apply"), ["btn-apply", "apply-span"], 2);
        check!(
            "text.exact",
            CdpLocateRequest { exact: true, ..req(CdpLocateEngine::Text, "Submit Order") },
            ["btn-submit"], 1
        );
        check!(
            "text.regex",
            CdpLocateRequest { regex: true, ..req(CdpLocateEngine::Text, "^Read more$") },
            ["lnk-1", "lnk-2"], 2
        );

        // ---- #1114 Role + name + state ----
        check!(
            "role.name",
            CdpLocateRequest { name: Some("Submit Order".to_owned()), name_exact: true, ..req(CdpLocateEngine::Role, "button") },
            ["btn-submit"], 1
        );
        check!(
            "role.checked",
            CdpLocateRequest { checked: Some(true), ..req(CdpLocateEngine::Role, "checkbox") },
            ["cb-on"], 1
        );
        check!(
            "role.heading.level",
            CdpLocateRequest { level: Some(2), ..req(CdpLocateEngine::Role, "heading") },
            ["heading2"], 1
        );

        // ---- #1115 Label / placeholder / altText / title ----
        check!("label.for", req(CdpLocateEngine::Label, "Full Name"), ["inp-name"], 1);
        check!("label.aria", req(CdpLocateEngine::Label, "Search query"), ["inp-aria"], 1);
        check!("placeholder", req(CdpLocateEngine::Placeholder, "Email address"), ["inp-email"], 1);
        check!("alttext", req(CdpLocateEngine::AltText, "Company Logo"), ["img-logo"], 1);
        check!("title", req(CdpLocateEngine::Title, "Tooltip Here"), ["sp-title"], 1);

        // ---- #1116 TestId (default + configurable attribute) ----
        check!("testid.default", req(CdpLocateEngine::TestId, "submit-btn"), ["btn-submit"], 1);
        check!(
            "testid.custom-attr",
            CdpLocateRequest { testid_attribute: Some("data-test".to_owned()), ..req(CdpLocateEngine::TestId, "alt-attr") },
            ["ti-custom"], 1
        );

        // ---- #1117 Layout / relational ----
        check!(
            "layout.right-of",
            CdpLocateRequest { relation: Some(CdpLayoutRelation::RightOf), anchor: Some("#anchor-box".to_owned()), ..req(CdpLocateEngine::Layout, "button.lay") },
            ["right-btn"], 1
        );
        check!(
            "layout.left-of",
            CdpLocateRequest { relation: Some(CdpLayoutRelation::LeftOf), anchor: Some("#anchor-box".to_owned()), ..req(CdpLocateEngine::Layout, "button.lay") },
            ["left-btn"], 1
        );

        // ---- #1118 chaining / filter / nth / strict ----
        // root scoping: only the button inside #card.
        let card = cdp_locate(&endpoint, &target_id, req(CdpLocateEngine::Css, "#card"))
            .await
            .expect("locate card");
        let card_backend = card.backend_node_ids[0];
        check!(
            "chain.root",
            CdpLocateRequest { root_backend_node_id: Some(card_backend), ..req(CdpLocateEngine::Css, "button") },
            ["incard"], 1
        );
        check!(
            "nth.first",
            CdpLocateRequest { nth: Some(0), ..req(CdpLocateEngine::Css, "a.lnk") },
            ["lnk-1"], 2
        );
        check!(
            "nth.last",
            CdpLocateRequest { nth: Some(-1), ..req(CdpLocateEngine::Css, "a.lnk") },
            ["lnk-2"], 2
        );
        check!(
            "filter.hasText",
            CdpLocateRequest { has_text: Some("Read more".to_owned()), ..req(CdpLocateEngine::Css, "a") },
            ["lnk-1", "lnk-2"], 2
        );
        check!(
            "filter.hasText.none",
            CdpLocateRequest { has_text: Some("zzz-nope".to_owned()), ..req(CdpLocateEngine::Css, "a") },
            [] as [&str; 0], 0
        );

        // strict: >1 match must error; ==1 must pass.
        let strict_err = cdp_locate(
            &endpoint,
            &target_id,
            CdpLocateRequest { strict: true, ..req(CdpLocateEngine::Css, "a.lnk") },
        )
        .await;
        println!("readback=fsv check='strict.multiple' result={strict_err:?}");
        assert!(strict_err.is_err(), "strict mode must error on >1 match");
        pass += 1;
        check!(
            "strict.unique",
            CdpLocateRequest { strict: true, ..req(CdpLocateEngine::Css, "#btn-apply") },
            ["btn-apply"], 1
        );

        // ---- #1120 the resolved id drives a REAL action ----
        let submit = cdp_locate(&endpoint, &target_id, req(CdpLocateEngine::TestId, "submit-btn"))
            .await
            .expect("locate submit");
        let submit_backend = submit.backend_node_ids[0];
        let clicks_before = page
            .evaluate("String(window.__synapse_clicks||0)")
            .await
            .expect("read counter")
            .into_value::<String>()
            .unwrap_or_default();
        println!("readback=fsv action stage=before __synapse_clicks={clicks_before:?}");
        let point = cdp_click_node(
            &endpoint,
            "Synapse Selector FSV",
            Some(&target_id),
            submit_backend,
            CdpMouseButton::Left,
            1,
        )
        .await
        .expect("cdp_click_node on resolved id");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let clicks_after = page
            .evaluate("String(window.__synapse_clicks||0)")
            .await
            .expect("read counter")
            .into_value::<String>()
            .unwrap_or_default();
        println!(
            "readback=fsv action stage=after click_at=({:.1},{:.1}) __synapse_clicks={clicks_after:?}",
            point.x, point.y
        );
        assert_eq!(clicks_before, "0", "counter should start at 0");
        assert_eq!(clicks_after, "1", "resolved testid id must drive a real click → counter 0->1");
        pass += 1;

        let _ = page.close().await;
        println!("readback=fsv VERDICT PASS checks={pass}");
    });
}
