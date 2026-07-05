//! #1110-#1120 support probe: drive the real `cdp_locate` selector engine
//! against a live Chrome on a synthetic page with known content. Each resolved
//! `backendNodeId` is independently read back to its element `id`, then the
//! resolved id drives a real `cdp_click_node` and reads the onclick counter.
//! This is regression/support evidence only; manual FSV evidence belongs in
//! GitHub issue comments.
//!
//! Usage (a Chromium must be listening with --remote-debugging-port):
//!   cargo run -p synapse-a11y --example `cdp_selector_engine_probe` -- <http://127.0.0.1:9222>
#![allow(clippy::expect_used, clippy::too_many_lines, clippy::print_stdout)]

#[cfg(windows)]
use chromiumoxide::cdp::browser_protocol::dom::{
    BackendNodeId, GetDocumentParams, ResolveNodeParams,
};
#[cfg(windows)]
use chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams;
#[cfg(windows)]
use chromiumoxide::{Browser, Page};
#[cfg(windows)]
use futures_util::StreamExt as _;
#[cfg(windows)]
use synapse_a11y::{
    CdpLayoutRelation, CdpLocateEngine, CdpLocateRequest, CdpLocateResult, CdpMouseButton,
    cdp_click_node, cdp_locate,
};

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn req(engine: CdpLocateEngine, query: &str) -> CdpLocateRequest {
    CdpLocateRequest {
        engine,
        query: query.to_owned(),
        limit: 50,
        ..Default::default()
    }
}

#[cfg(windows)]
async fn resolved_id(page: &Page, backend: i64) -> String {
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
        .function_declaration(
            "function(){ return this.id || this.tagName.toLowerCase(); }".to_owned(),
        )
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

#[cfg(windows)]
struct ProbeCase {
    label: &'static str,
    request: CdpLocateRequest,
    expected_ids: Vec<&'static str>,
    expected_count: usize,
}

#[cfg(windows)]
const fn case(
    label: &'static str,
    request: CdpLocateRequest,
    expected_ids: Vec<&'static str>,
    expected_count: usize,
) -> ProbeCase {
    ProbeCase {
        label,
        request,
        expected_ids,
        expected_count,
    }
}

#[cfg(windows)]
struct ProbeContext<'a> {
    endpoint: &'a str,
    target_id: &'a str,
    page: &'a Page,
    pass: u32,
}

#[cfg(windows)]
impl ProbeContext<'_> {
    async fn check(
        &mut self,
        label: &str,
        request: CdpLocateRequest,
        expected_ids: &[&str],
        expected_count: usize,
    ) -> CdpLocateResult {
        let result = cdp_locate(self.endpoint, self.target_id, request)
            .await
            .unwrap_or_else(|e| panic!("[{label}] cdp_locate FAILED: {e}"));
        let mut got = Vec::new();
        for backend in &result.backend_node_ids {
            got.push(resolved_id(self.page, *backend).await);
        }
        let expected: Vec<String> = expected_ids.iter().map(|id| (*id).to_owned()).collect();
        println!(
            "readback=probe check='{label}' match_count={} expected_count={} got_ids={got:?} expected_ids={expected:?}",
            result.match_count, expected_count
        );
        assert_eq!(result.match_count, expected_count, "[{label}] match_count");
        assert_eq!(got, expected, "[{label}] resolved ids (order-sensitive)");
        self.pass += 1;
        result
    }
}

#[cfg(windows)]
async fn run_cases(context: &mut ProbeContext<'_>, cases: Vec<ProbeCase>) {
    for case in cases {
        context
            .check(
                case.label,
                case.request,
                &case.expected_ids,
                case.expected_count,
            )
            .await;
    }
}

#[cfg(windows)]
fn main() {
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

    rt.block_on(Box::pin(async move {
        let (browser, mut handler) = Browser::connect(&endpoint).await.expect("connect to Chrome");
        let _drive = tokio::spawn(async move { while handler.next().await.is_some() {} });

        let page = browser.new_page("about:blank").await.expect("new page");
        let target_id = page.target_id().inner().clone();
        page.evaluate(format!(
            "document.title='Synapse Selector Probe'; document.body.innerHTML = `{PAGE_HTML}`;"
        ))
        .await
        .expect("inject synthetic page");
        // Prime the full document so resolveNode sees every backend node.
        let _ = page
            .execute(GetDocumentParams::builder().depth(-1).pierce(true).build())
            .await;
        println!("readback=probe stage=setup target={target_id} url=about:blank title='Synapse Selector Probe'");

        let checks = {
            // Runs each request, reads back the resolved ids, and asserts the set + count.
            let mut probe = ProbeContext {
                endpoint: &endpoint,
                target_id: &target_id,
                page: &page,
                pass: 0,
            };
            run_cases(
                &mut probe,
                vec![
                // ---- #1111 CSS ----
                case(
                    "css.unique",
                    req(CdpLocateEngine::Css, "#btn-apply"),
                    vec!["btn-apply"],
                    1,
                ),
                case(
                    "css.multiple",
                    req(CdpLocateEngine::Css, "a.lnk"),
                    vec!["lnk-1", "lnk-2"],
                    2,
                ),
                case(
                    "css.nomatch",
                    req(CdpLocateEngine::Css, "#does-not-exist"),
                    vec![],
                    0,
                ),
                // ---- #1112 XPath ----
                case(
                    "xpath.attr",
                    req(CdpLocateEngine::Xpath, "//button[@id='btn-submit']"),
                    vec!["btn-submit"],
                    1,
                ),
                case(
                    "xpath.text",
                    req(CdpLocateEngine::Xpath, "//a[contains(.,'Read more')]"),
                    vec!["lnk-1", "lnk-2"],
                    2,
                ),
                case(
                    "xpath.nomatch",
                    req(CdpLocateEngine::Xpath, "//table"),
                    vec![],
                    0,
                ),
                // ---- #1113 Text (substring / exact / regex) ----
                // substring "apply" (case-insensitive) -> deepest matches: btn-apply + apply-span.
                case(
                    "text.substring",
                    req(CdpLocateEngine::Text, "apply"),
                    vec!["btn-apply", "apply-span"],
                    2,
                ),
                case(
                    "text.exact",
                    CdpLocateRequest {
                        exact: true,
                        ..req(CdpLocateEngine::Text, "Submit Order")
                    },
                    vec!["btn-submit"],
                    1,
                ),
                case(
                    "text.regex",
                    CdpLocateRequest {
                        regex: true,
                        ..req(CdpLocateEngine::Text, "^Read more$")
                    },
                    vec!["lnk-1", "lnk-2"],
                    2,
                ),
                // ---- #1114 Role + name + state ----
                case(
                    "role.name",
                    CdpLocateRequest {
                        name: Some("Submit Order".to_owned()),
                        name_exact: true,
                        ..req(CdpLocateEngine::Role, "button")
                    },
                    vec!["btn-submit"],
                    1,
                ),
                case(
                    "role.checked",
                    CdpLocateRequest {
                        checked: Some(true),
                        ..req(CdpLocateEngine::Role, "checkbox")
                    },
                    vec!["cb-on"],
                    1,
                ),
                case(
                    "role.heading.level",
                    CdpLocateRequest {
                        level: Some(2),
                        ..req(CdpLocateEngine::Role, "heading")
                    },
                    vec!["heading2"],
                    1,
                ),
                // ---- #1115 Label / placeholder / altText / title ----
                case(
                    "label.for",
                    req(CdpLocateEngine::Label, "Full Name"),
                    vec!["inp-name"],
                    1,
                ),
                case(
                    "label.aria",
                    req(CdpLocateEngine::Label, "Search query"),
                    vec!["inp-aria"],
                    1,
                ),
                case(
                    "placeholder",
                    req(CdpLocateEngine::Placeholder, "Email address"),
                    vec!["inp-email"],
                    1,
                ),
                case(
                    "alttext",
                    req(CdpLocateEngine::AltText, "Company Logo"),
                    vec!["img-logo"],
                    1,
                ),
                case(
                    "title",
                    req(CdpLocateEngine::Title, "Tooltip Here"),
                    vec!["sp-title"],
                    1,
                ),
                // ---- #1116 TestId (default + configurable attribute) ----
                case(
                    "testid.default",
                    req(CdpLocateEngine::TestId, "submit-btn"),
                    vec!["btn-submit"],
                    1,
                ),
                case(
                    "testid.custom-attr",
                    CdpLocateRequest {
                        testid_attribute: Some("data-test".to_owned()),
                        ..req(CdpLocateEngine::TestId, "alt-attr")
                    },
                    vec!["ti-custom"],
                    1,
                ),
                // ---- #1117 Layout / relational ----
                case(
                    "layout.right-of",
                    CdpLocateRequest {
                        relation: Some(CdpLayoutRelation::RightOf),
                        anchor: Some("#anchor-box".to_owned()),
                        ..req(CdpLocateEngine::Layout, "button.lay")
                    },
                    vec!["right-btn"],
                    1,
                ),
                case(
                    "layout.left-of",
                    CdpLocateRequest {
                        relation: Some(CdpLayoutRelation::LeftOf),
                        anchor: Some("#anchor-box".to_owned()),
                        ..req(CdpLocateEngine::Layout, "button.lay")
                    },
                    vec!["left-btn"],
                    1,
                ),
                ],
            )
            .await;

        // ---- #1118 chaining / filter / nth / strict ----
        // root scoping: only the button inside #card.
        let card = cdp_locate(&endpoint, &target_id, req(CdpLocateEngine::Css, "#card"))
            .await
            .expect("locate card");
        let card_backend = card.backend_node_ids[0];
        run_cases(
            &mut probe,
            vec![
                case(
                    "chain.root",
                    CdpLocateRequest {
                        root_backend_node_id: Some(card_backend),
                        ..req(CdpLocateEngine::Css, "button")
                    },
                    vec!["incard"],
                    1,
                ),
                case(
                    "nth.first",
                    CdpLocateRequest {
                        nth: Some(0),
                        ..req(CdpLocateEngine::Css, "a.lnk")
                    },
                    vec!["lnk-1"],
                    2,
                ),
                case(
                    "nth.last",
                    CdpLocateRequest {
                        nth: Some(-1),
                        ..req(CdpLocateEngine::Css, "a.lnk")
                    },
                    vec!["lnk-2"],
                    2,
                ),
                case(
                    "filter.hasText",
                    CdpLocateRequest {
                        has_text: Some("Read more".to_owned()),
                        ..req(CdpLocateEngine::Css, "a")
                    },
                    vec!["lnk-1", "lnk-2"],
                    2,
                ),
                case(
                    "filter.hasText.none",
                    CdpLocateRequest {
                        has_text: Some("zzz-nope".to_owned()),
                        ..req(CdpLocateEngine::Css, "a")
                    },
                    vec![],
                    0,
                ),
            ],
        )
        .await;

        // strict: >1 match must error; ==1 must pass.
        let strict_err = cdp_locate(
            &endpoint,
            &target_id,
            CdpLocateRequest { strict: true, ..req(CdpLocateEngine::Css, "a.lnk") },
        )
        .await;
        println!("readback=probe check='strict.multiple' result={strict_err:?}");
        assert!(strict_err.is_err(), "strict mode must error on >1 match");
        probe.pass += 1;
        probe
            .check(
                "strict.unique",
                CdpLocateRequest {
                    strict: true,
                    ..req(CdpLocateEngine::Css, "#btn-apply")
                },
                &["btn-apply"],
                1,
            )
            .await;

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
        println!("readback=probe action stage=before __synapse_clicks={clicks_before:?}");
        let point = cdp_click_node(
            &endpoint,
            "Synapse Selector Probe",
            Some(&target_id),
            submit_backend,
            CdpMouseButton::Left,
            1,
            0,
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
            "readback=probe action stage=after click_at=({:.1},{:.1}) __synapse_clicks={clicks_after:?}",
            point.x, point.y
        );
        assert_eq!(clicks_before, "0", "counter should start at 0");
        assert_eq!(clicks_after, "1", "resolved testid id must drive a real click → counter 0->1");
            probe.pass += 1;
            probe.pass
        };

        let _ = page.close().await;
        println!("readback=probe VERDICT PASS checks={checks}");
    }));
}
