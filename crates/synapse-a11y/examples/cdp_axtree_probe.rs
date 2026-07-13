//! Manual de-risking probe (NOT an automated test, NOT FSV-gated): attach to a
//! running Chromium CDP endpoint, navigate to a page with KNOWN content, pull
//! `Accessibility.getFullAXTree`, and print the mapped nodes + one box model.
//!
//! Usage (Windows only — CDP attach is a `cfg(windows)` capability):
//!   `cargo run -p synapse-a11y --example cdp_axtree_probe -- <http://127.0.0.1:9222>`
//!
//! Synthetic known input → known expected output: the navigated page contains a
//! heading "Hello Probe", a button "Apply", a link "YC Link", and an email
//! textbox, so the printed AX tree can be checked by eye against those.

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_impl::run().await
}

#[cfg(not(windows))]
fn main() {
    eprintln!("cdp_axtree_probe is a Windows-only example (CDP attach is cfg(windows))");
}

#[cfg(windows)]
mod windows_impl {
    use chromiumoxide::Browser;
    use chromiumoxide::cdp::browser_protocol::accessibility::{
        AxValue, EnableParams, GetFullAxTreeParams,
    };
    use chromiumoxide::cdp::browser_protocol::dom::GetBoxModelParams;
    use futures_util::StreamExt as _;

    const KNOWN_PAGE: &str = "data:text/html,<html><body><h1>Hello Probe</h1>\
<button>Apply</button><a href='#x'>YC Link</a>\
<label>Email<input type='email' aria-label='email'></label></body></html>";

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let Some(endpoint) = std::env::args().nth(1) else {
            return Err("usage: cdp_axtree_probe <http://host:port | ws url>".into());
        };
        println!("readback=cdp_attach before=endpoint:{endpoint}");

        let (browser, mut handler) = Browser::connect(endpoint).await?;
        let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });
        println!("readback=cdp_attach after=connected");

        let page = browser.new_page(KNOWN_PAGE).await?;
        page.wait_for_navigation().await?;
        println!("readback=cdp_navigate after=url:{:?}", page.url().await?);

        page.execute(EnableParams::default()).await?;
        let tree = page.execute(GetFullAxTreeParams::default()).await?;
        let nodes = &tree.result.nodes;
        println!("readback=ax_tree after=total_nodes:{}", nodes.len());

        let mut interactive = 0_usize;
        let mut apply_backend = None;
        for node in nodes {
            if node.ignored {
                continue;
            }
            let role = ax_value_str(node.role.as_ref());
            let name = ax_value_str(node.name.as_ref());
            if matches!(
                role.as_str(),
                "button" | "link" | "heading" | "textbox" | "StaticText"
            ) {
                interactive += 1;
                println!(
                    "readback=ax_node role:{role} name:{name:?} backend_dom_node_id:{:?}",
                    node.backend_dom_node_id
                );
                if role == "button" && name == "Apply" {
                    apply_backend = node.backend_dom_node_id;
                }
            }
        }
        println!("readback=ax_tree after=interactive_nodes:{interactive}");

        // Prove screen-space bounds are resolvable for an AX node via box model.
        if let Some(backend) = apply_backend {
            let params = GetBoxModelParams::builder()
                .backend_node_id(backend)
                .build();
            match page.execute(params).await {
                Ok(box_model) => {
                    let model = &box_model.result.model;
                    println!(
                        "readback=box_model role:button name:Apply content:{:?} width:{} height:{}",
                        model.content, model.width, model.height
                    );
                }
                Err(error) => println!("readback=box_model ERROR:{error}"),
            }
        } else {
            println!("readback=box_model ERROR=apply_button_not_found_in_ax_tree");
        }

        handler_task.abort();
        println!("readback=cdp_probe done");
        Ok(())
    }

    fn ax_value_str(value: Option<&AxValue>) -> String {
        value
            .and_then(|value| value.value.as_ref())
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }
}
