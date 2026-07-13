//! Manual de-risking probe for #686 (NOT FSV-gated): drive the real CDP action
//! functions against a live Chromium and verify the DOM actually changed.
//!
//! Usage (Windows only): `cargo run -p synapse-a11y --example cdp_action_probe -- <http://127.0.0.1:9222>`
//!
//! Known input → known output: a page (title `PROBEPAGE`) with a button whose
//! onclick sets `body[data-clicked]=yes`, and a text input. We CDP-click the
//! button and CDP-type into the input, then read the attribute and the input
//! value back to prove the actions landed.

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_impl::run().await
}

#[cfg(not(windows))]
fn main() {
    eprintln!("cdp_action_probe is Windows-only");
}

#[cfg(windows)]
mod windows_impl {
    use chromiumoxide::Browser;
    use chromiumoxide::cdp::browser_protocol::accessibility::{
        AxValue, EnableParams, GetFullAxTreeParams,
    };
    use futures_util::StreamExt as _;
    use synapse_a11y::CdpMouseButton;

    const PAGE_TITLE: &str = "PROBEPAGE";
    const KNOWN_PAGE: &str = "data:text/html,<html><head><title>PROBEPAGE</title></head><body>\
<button onclick=\"document.body.setAttribute('data-clicked','yes')\">Go</button>\
<input id='t' aria-label='field'></body></html>";

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let Some(endpoint) = std::env::args().nth(1) else {
            return Err("usage: cdp_action_probe <http://host:port>".into());
        };
        let (browser, mut handler) = Browser::connect(&endpoint).await?;
        let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

        let page = browser.new_page(KNOWN_PAGE).await?;
        page.wait_for_navigation().await?;
        page.execute(EnableParams::default()).await?;
        let tree = page.execute(GetFullAxTreeParams::default()).await?;

        let mut button_backend = None;
        let mut input_backend = None;
        for node in &tree.result.nodes {
            let role = ax_str(node.role.as_ref());
            match role.as_str() {
                "button" => {
                    button_backend = node.backend_dom_node_id.as_ref().map(|id| *id.inner());
                }
                "textbox" => {
                    input_backend = node.backend_dom_node_id.as_ref().map(|id| *id.inner());
                }
                _ => {}
            }
        }
        println!(
            "readback=action_probe backends=button:{button_backend:?} input:{input_backend:?}"
        );

        // The button onclick sets body[data-clicked]; the page title stays stable
        // ("PROBEPAGE") so page-by-title selection works through the whole run.
        let _ = page
            .evaluate("document.body.setAttribute('data-clicked','no')")
            .await;

        // --- CLICK ---
        let button = button_backend.ok_or("button not found in AX tree")?;
        let before = eval_string(&page, "document.body.getAttribute('data-clicked')||''").await?;
        let landed = synapse_a11y::cdp_click_node(
            &endpoint,
            PAGE_TITLE,
            None,
            button,
            CdpMouseButton::Left,
            1,
            0,
        )
        .await?;
        let after = eval_string(&page, "document.body.getAttribute('data-clicked')||''").await?;
        println!(
            "readback=cdp_click before=data-clicked:{before:?} landed:({:.1},{:.1}) after=data-clicked:{after:?}",
            landed.x, landed.y
        );
        assert_eq!(after, "yes", "CDP click did not fire the button onclick");

        // --- TYPE: insert text into the input, read value back ---
        let input = input_backend.ok_or("input not found in AX tree")?;
        synapse_a11y::cdp_type_node(&endpoint, PAGE_TITLE, None, input, "hello probe").await?;
        let value = eval_string(&page, "document.getElementById('t').value").await?;
        println!("readback=cdp_type after=input_value:{value:?}");
        assert_eq!(
            value, "hello probe",
            "CDP type did not enter the expected text"
        );

        handler_task.abort();
        println!("readback=action_probe done ALL_ASSERTS_PASSED");
        Ok(())
    }

    async fn eval_string(
        page: &chromiumoxide::Page,
        expr: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let result = page.evaluate(expr).await?;
        Ok(result.into_value::<String>().unwrap_or_default())
    }

    fn ax_str(value: Option<&AxValue>) -> String {
        value
            .and_then(|value| value.value.as_ref())
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }
}
