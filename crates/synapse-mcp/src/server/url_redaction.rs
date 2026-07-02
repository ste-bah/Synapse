const URL_REDACTION_MARKER: &str = "redacted";

pub(crate) fn redact_url_for_public_readback(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }

    if let Some(rest) = strip_scheme_prefix(url, "view-source") {
        return format!("view-source:{}", redact_url_for_public_readback(rest));
    }

    for scheme in ["data", "javascript", "mailto"] {
        if strip_scheme_prefix(url, scheme).is_some() {
            return format!("{scheme}:{URL_REDACTION_MARKER}");
        }
    }

    if strip_scheme_prefix(url, "file").is_some() {
        return format!("file:{URL_REDACTION_MARKER}");
    }

    match reqwest::Url::parse(url) {
        Ok(mut parsed) => {
            if parsed.query().is_some() {
                parsed.set_query(Some(URL_REDACTION_MARKER));
            }
            if parsed.fragment().is_some() {
                parsed.set_fragment(Some(URL_REDACTION_MARKER));
            }
            parsed.to_string()
        }
        Err(_error) => redact_query_and_fragment_without_parse(url),
    }
}

pub(crate) fn redact_title_for_public_url_readback(url: &str, title: String) -> String {
    if url_has_content_bearing_scheme(url) {
        return URL_REDACTION_MARKER.to_owned();
    }
    title
}

pub(crate) fn url_has_content_bearing_scheme(url: &str) -> bool {
    if let Some(rest) = strip_scheme_prefix(url, "view-source") {
        return url_has_content_bearing_scheme(rest);
    }

    ["data", "javascript", "mailto", "file"]
        .iter()
        .any(|scheme| strip_scheme_prefix(url, scheme).is_some())
}

fn strip_scheme_prefix<'a>(url: &'a str, scheme: &str) -> Option<&'a str> {
    let (candidate, rest) = url.split_once(':')?;
    candidate.eq_ignore_ascii_case(scheme).then_some(rest)
}

fn redact_query_and_fragment_without_parse(url: &str) -> String {
    let query_index = url.find('?');
    let fragment_index = url.find('#');
    match (query_index, fragment_index) {
        (None, None) => url.to_owned(),
        (Some(query), None) => format!("{}?{URL_REDACTION_MARKER}", &url[..query]),
        (None, Some(fragment)) => format!("{}#{URL_REDACTION_MARKER}", &url[..fragment]),
        (Some(query), Some(fragment)) if query < fragment => {
            format!(
                "{}?{URL_REDACTION_MARKER}#{URL_REDACTION_MARKER}",
                &url[..query]
            )
        }
        (Some(_query), Some(fragment)) => {
            format!("{}#{URL_REDACTION_MARKER}", &url[..fragment])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{redact_title_for_public_url_readback, redact_url_for_public_readback};

    #[test]
    fn redacts_query_and_fragment_values() {
        let redacted = redact_url_for_public_readback(
            "https://example.test/path?body=SYNAPSE_SECRET&token=SYNAPSE_TOKEN#frag=SYNAPSE_HASH",
        );

        assert_eq!(redacted, "https://example.test/path?redacted#redacted");
        assert!(!redacted.contains("SYNAPSE_SECRET"));
        assert!(!redacted.contains("SYNAPSE_TOKEN"));
        assert!(!redacted.contains("SYNAPSE_HASH"));
    }

    #[test]
    fn preserves_url_without_sensitive_components() {
        assert_eq!(
            redact_url_for_public_readback("https://example.test/path"),
            "https://example.test/path"
        );
    }

    #[test]
    fn redacts_opaque_content_bearing_schemes() {
        assert_eq!(
            redact_url_for_public_readback("data:text/html,<main>SYNAPSE_SECRET</main>"),
            "data:redacted"
        );
        assert_eq!(
            redact_url_for_public_readback("mailto:user@example.test?subject=SYNAPSE_SECRET"),
            "mailto:redacted"
        );
        assert_eq!(
            redact_url_for_public_readback("file:///C:/Users/hotra/private.txt"),
            "file:redacted"
        );
    }

    #[test]
    fn redacts_titles_for_opaque_content_bearing_schemes() {
        let redacted = redact_title_for_public_url_readback(
            "data:text/html,<title>SYNAPSE_SECRET</title>",
            "SYNAPSE_SECRET".to_owned(),
        );

        assert_eq!(redacted, "redacted");
        assert!(!redacted.contains("SYNAPSE_SECRET"));
        assert_eq!(
            redact_title_for_public_url_readback(
                "view-source:file:///C:/Users/hotra/private.txt",
                "private.txt".to_owned()
            ),
            "redacted"
        );
        assert_eq!(
            redact_title_for_public_url_readback(
                "https://example.test/path?token=SYNAPSE_SECRET",
                "Example".to_owned()
            ),
            "Example"
        );
    }

    #[test]
    fn redacts_unparseable_query_and_fragment() {
        let redacted = redact_url_for_public_readback("/relative/path?token=SYNAPSE_SECRET#frag");

        assert_eq!(redacted, "/relative/path?redacted#redacted");
        assert!(!redacted.contains("SYNAPSE_SECRET"));
    }
}
