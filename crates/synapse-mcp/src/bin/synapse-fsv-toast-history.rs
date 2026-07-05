#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_impl::run()
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("synapse-fsv-toast-history requires Windows notification APIs")
}

#[cfg(windows)]
mod windows_impl {
    use std::{collections::VecDeque, fmt::Write as _};

    use anyhow::{Context as _, bail};
    use regex::Regex;
    use serde::Serialize;
    use sha2::{Digest as _, Sha256};
    use windows::{
        UI::Notifications::ToastNotificationManager,
        Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
        core::HSTRING,
    };

    const AUMID: &str = "Synapse.Daemon";
    const GROUP: &str = "synapse";

    #[derive(Debug, Serialize)]
    struct ToastRow {
        tag: String,
        group: String,
        xml_len: usize,
        xml_sha256: String,
        texts: Vec<String>,
        actions: Vec<ToastActionReadback>,
    }

    #[derive(Debug, Serialize)]
    struct ToastActionReadback {
        content: String,
        arguments: String,
        activation_type: String,
    }

    #[derive(Debug, Serialize)]
    struct ListResponse {
        aumid: &'static str,
        tag_filter: Option<String>,
        group_filter: Option<String>,
        count: usize,
        rows: Vec<ToastRow>,
    }

    #[derive(Debug, Serialize)]
    struct ApprovalExtractResponse {
        aumid: &'static str,
        tag: String,
        group: String,
        xml_len: usize,
        xml_sha256: String,
        texts: Vec<String>,
        accept_uri: Option<String>,
        decline_uri: Option<String>,
        snooze_uri: Option<String>,
        actions: Vec<ToastActionReadback>,
    }

    #[derive(Debug, Serialize)]
    struct RemoveResponse {
        aumid: &'static str,
        tag: String,
        group: String,
        remaining_count: usize,
    }

    #[derive(Debug, Default)]
    struct Options {
        tag: Option<String>,
        group: Option<String>,
    }

    pub fn run() -> anyhow::Result<()> {
        ensure_com();
        let mut args = std::env::args().skip(1).collect::<VecDeque<_>>();
        let command = args.pop_front().unwrap_or_else(|| "list".to_owned());
        let options = parse_options(args)?;
        match command.as_str() {
            "list" => {
                let rows = read_rows(options.tag.as_deref(), options.group.as_deref())?;
                write_json(&ListResponse {
                    aumid: AUMID,
                    tag_filter: options.tag,
                    group_filter: options.group,
                    count: rows.len(),
                    rows,
                })
            }
            "extract-approval" => {
                let tag = options
                    .tag
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .context("extract-approval requires --tag <toast-tag>")?;
                let mut rows = read_rows(Some(tag), options.group.as_deref())?;
                if rows.is_empty() {
                    bail!("toast tag {tag:?} was not found in Action Center history");
                }
                if rows.len() > 1 {
                    bail!("toast tag {tag:?} matched {} rows", rows.len());
                }
                let row = rows.remove(0);
                let accept_uri = action_uri(&row.actions, "accept");
                let decline_uri = action_uri(&row.actions, "decline");
                let snooze_uri = action_uri(&row.actions, "snooze");
                write_json(&ApprovalExtractResponse {
                    aumid: AUMID,
                    tag: row.tag,
                    group: row.group,
                    xml_len: row.xml_len,
                    xml_sha256: row.xml_sha256,
                    texts: row.texts,
                    accept_uri,
                    decline_uri,
                    snooze_uri,
                    actions: row.actions,
                })
            }
            "remove" => {
                let tag = options
                    .tag
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .context("remove requires --tag <toast-tag>")?;
                let group = options.group.as_deref().unwrap_or(GROUP);
                remove_grouped_tag(tag, group)?;
                let remaining = read_rows(Some(tag), Some(group))?.len();
                write_json(&RemoveResponse {
                    aumid: AUMID,
                    tag: tag.to_owned(),
                    group: group.to_owned(),
                    remaining_count: remaining,
                })
            }
            "--help" | "-h" | "help" => {
                println!(
                    "Usage: synapse-fsv-toast-history <list|extract-approval|remove> [--tag TAG] [--group GROUP]"
                );
                Ok(())
            }
            other => bail!("unknown command {other:?}; expected list, extract-approval, or remove"),
        }
    }

    fn ensure_com() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    fn parse_options(mut args: VecDeque<String>) -> anyhow::Result<Options> {
        let mut options = Options::default();
        while let Some(arg) = args.pop_front() {
            match arg.as_str() {
                "--tag" => {
                    options.tag = Some(args.pop_front().context("--tag requires a value")?);
                }
                "--group" => {
                    options.group = Some(args.pop_front().context("--group requires a value")?);
                }
                other => bail!("unknown option {other:?}"),
            }
        }
        Ok(options)
    }

    fn read_rows(
        tag_filter: Option<&str>,
        group_filter: Option<&str>,
    ) -> anyhow::Result<Vec<ToastRow>> {
        let history =
            ToastNotificationManager::History().context("ToastNotificationManager::History")?;
        let toasts = history
            .GetHistoryWithId(&HSTRING::from(AUMID))
            .context("GetHistoryWithId(Synapse.Daemon)")?;
        let size = toasts.Size().context("toast history Size")?;
        let mut rows = Vec::new();
        for index in 0..size {
            let toast = toasts.GetAt(index).context("toast history GetAt")?;
            let tag = toast
                .Tag()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            let group = toast
                .Group()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            if let Some(filter) = tag_filter
                && tag != filter
            {
                continue;
            }
            if let Some(filter) = group_filter.or(Some(GROUP))
                && group != filter
            {
                continue;
            }
            let xml = toast
                .Content()
                .and_then(|document| document.GetXml())
                .map(|xml| xml.to_string_lossy())
                .unwrap_or_default();
            rows.push(ToastRow {
                tag,
                group,
                xml_len: xml.len(),
                xml_sha256: sha256_hex(xml.as_bytes()),
                texts: extract_texts(&xml)?,
                actions: extract_actions(&xml)?,
            });
        }
        Ok(rows)
    }

    fn remove_grouped_tag(tag: &str, group: &str) -> anyhow::Result<()> {
        let history =
            ToastNotificationManager::History().context("ToastNotificationManager::History")?;
        history
            .RemoveGroupedTagWithId(
                &HSTRING::from(tag),
                &HSTRING::from(group),
                &HSTRING::from(AUMID),
            )
            .with_context(|| {
                format!("RemoveGroupedTagWithId(tag={tag}, group={group}, app_id={AUMID})")
            })
    }

    fn extract_texts(xml: &str) -> anyhow::Result<Vec<String>> {
        let text_re = Regex::new(r"(?s)<text(?:\s[^>]*)?>(.*?)</text>")?;
        Ok(text_re
            .captures_iter(xml)
            .filter_map(|capture| capture.get(1).map(|value| decode_xml(value.as_str())))
            .collect())
    }

    fn extract_actions(xml: &str) -> anyhow::Result<Vec<ToastActionReadback>> {
        let action_re = Regex::new(r"<action\b([^>]*)/?>")?;
        let attr_re = Regex::new(r#"([A-Za-z_:][\w:.-]*)="([^"]*)""#)?;
        let mut actions = Vec::new();
        for action in action_re.captures_iter(xml) {
            let attrs = action
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let mut content = String::new();
            let mut arguments = String::new();
            let mut activation_type = String::new();
            for attr in attr_re.captures_iter(attrs) {
                let name = attr.get(1).map(|value| value.as_str()).unwrap_or_default();
                let value = decode_xml(attr.get(2).map(|value| value.as_str()).unwrap_or_default());
                match name {
                    "content" => content = value,
                    "arguments" => arguments = value,
                    "activationType" => activation_type = value,
                    _ => {}
                }
            }
            actions.push(ToastActionReadback {
                content,
                arguments,
                activation_type,
            });
        }
        Ok(actions)
    }

    fn action_uri(actions: &[ToastActionReadback], decision: &str) -> Option<String> {
        let needle = format!("decision={decision}");
        actions
            .iter()
            .find(|action| action.arguments.contains(&needle))
            .map(|action| action.arguments.clone())
    }

    fn decode_xml(value: &str) -> String {
        value
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut hex = String::with_capacity("sha256:".len() + digest.len() * 2);
        hex.push_str("sha256:");
        for byte in digest {
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    fn write_json(value: &impl Serialize) -> anyhow::Result<()> {
        serde_json::to_writer_pretty(std::io::stdout(), value)?;
        println!();
        Ok(())
    }
}
