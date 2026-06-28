//! Verification Inbox — first slice (#1345).
//!
//! Lets an agent read verification/OTP codes out of the user's already-logged-in
//! web surface (e.g. Gmail) through the existing Chrome bridge — no Twilio / paid
//! SIM / paid SaaS. This slice reuses `browser_content` (debugger-free normal
//! Chrome bridge page read) on the session's bound tab, runs a keyword-gated OTP
//! extractor over the page text, journals every read to a CF_KV audit
//! (`verification/audit/v1/...`, masked codes only), and returns the extracted
//! codes with surrounding context so the agent can pick the right one by service.
//!
//! Providers (Gmail/Outlook/Messages/Voice) and the binding/poll tools are
//! follow-on slices; this is the Gmail-via-bridge read + audit backbone.

use std::sync::atomic::{AtomicU64, Ordering};

use rmcp::{RoleServer, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_storage::{Db, cf};

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};
use crate::m1::{BrowserContentParams, mcp_error};

const AUDIT_PREFIX: &str = "verification/audit/v1/";
const AUDIT_SCHEMA: u32 = 1;
const MAX_CONTENT_BYTES: usize = 2_000_000;
const MAX_CODES: usize = 25;
static AUDIT_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationInboxParams {
    /// Bridge tab to read (`chrome-tab:<id>`). Defaults to this session's bound
    /// CDP target. The tab should already be on the logged-in mail/SMS surface.
    #[serde(default)]
    pub cdp_target_id: Option<String>,
    /// Browser HWND owning the target. Defaults to the session target's window.
    #[serde(default)]
    pub window_hwnd: Option<i64>,
    /// Logical source label for the audit trail (e.g. `gmail`, `outlook`).
    #[serde(default)]
    pub source: Option<String>,
    /// Max codes to return (default 25).
    #[serde(default)]
    pub max_codes: Option<usize>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationCode {
    /// The extracted code (full value — for the agent to use).
    pub code: String,
    /// `google_g`, `numeric`, or `alphanumeric`.
    pub kind: String,
    /// ~50 chars of page text around the code, for service/sender attribution.
    pub context: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationInboxResponse {
    pub ok: bool,
    pub source: String,
    pub url: String,
    pub title: String,
    pub text_len: usize,
    pub codes: Vec<VerificationCode>,
    /// CF_KV key of the audit row written for this read.
    pub audit_key: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationAuditParams {
    /// Max rows to return, newest first (default 50).
    #[serde(default)]
    pub max: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationAuditRow {
    pub schema_version: u32,
    pub source: String,
    pub url: String,
    pub title: String,
    /// Masked codes only (e.g. `12***6`) — the audit never stores the raw code.
    pub masked_codes: Vec<String>,
    pub code_count: usize,
    pub read_at_unix_ms: u64,
    pub by_session: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationAuditResponse {
    pub ok: bool,
    pub count: usize,
    pub rows: Vec<VerificationAuditRow>,
}

#[tool_router(router = verification_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Read verification/OTP codes from this MCP session's bound Chrome tab (Verification Inbox, #1345). Reuses the debugger-free normal Chrome bridge page read (browser_content) on the bound tab — point it at the user's already-logged-in Gmail/Outlook/Messages surface — then runs a keyword-gated extractor (Google G-#### codes, and 4–8 digit / 5–8 alphanumeric codes that appear near words like code/verification/OTP/passcode/confirm/2FA) and returns each code with ~50 chars of surrounding context for service attribution. No Twilio/paid SIM/paid SaaS. Every read is journaled to a CF_KV audit (verification/audit/v1, MASKED codes only) and the audit key is returned. Use verification_audit to inspect the trail."
    )]
    pub async fn verification_inbox(
        &self,
        params: Parameters<VerificationInboxParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<VerificationInboxResponse>, ErrorData> {
        let params = params.0;
        let source = params.source.clone().unwrap_or_else(|| "unspecified".to_owned());
        let max_codes = params.max_codes.unwrap_or(MAX_CODES).min(200);
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "verification_inbox",
            source = %source,
            "tool.invocation kind=verification_inbox"
        );

        let session_id = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());

        let content = self
            .browser_content(
                Parameters(BrowserContentParams {
                    cdp_target_id: params.cdp_target_id.clone(),
                    window_hwnd: params.window_hwnd,
                    max_bytes: Some(MAX_CONTENT_BYTES),
                }),
                request_context.clone(),
            )
            .await?;
        let content = content.0;
        let text = html_to_text(&content.html);
        let mut codes = extract_verification_codes(&text);
        codes.truncate(max_codes);

        let read_at_unix_ms = unix_time_ms_now();
        let masked_codes: Vec<String> = codes.iter().map(|c| mask_code(&c.code)).collect();
        let audit_row = VerificationAuditRow {
            schema_version: AUDIT_SCHEMA,
            source: source.clone(),
            url: content.url.clone(),
            title: content.title.clone(),
            masked_codes,
            code_count: codes.len(),
            read_at_unix_ms,
            by_session: session_id.clone(),
        };
        let audit_key = self.verification_write_audit(&audit_row, read_at_unix_ms)?;

        Ok(Json(VerificationInboxResponse {
            ok: true,
            source,
            url: content.url,
            title: content.title,
            text_len: text.len(),
            codes,
            audit_key,
        }))
    }

    #[tool(
        description = "Read the Verification Inbox audit trail (#1345): the CF_KV journal (verification/audit/v1) of every verification_inbox read — source, tab url/title, MASKED codes, count, read timestamp, and consuming session — newest first. This is the inspectable source of truth for which codes were surfaced to agents; raw codes are never stored."
    )]
    pub async fn verification_audit(
        &self,
        params: Parameters<VerificationAuditParams>,
        _request_context: RequestContext<RoleServer>,
    ) -> Result<Json<VerificationAuditResponse>, ErrorData> {
        let max = params.0.max.unwrap_or(50).min(1000);
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "verification_audit",
            "tool.invocation kind=verification_audit"
        );
        let db = self.verification_db()?;
        let rows = db
            .scan_cf_prefix(cf::CF_KV, AUDIT_PREFIX.as_bytes())
            .map_err(|error| {
                mcp_error(
                    error.code(),
                    format!("verification_audit failed to scan audit rows: {error}"),
                )
            })?;
        let mut decoded: Vec<VerificationAuditRow> = Vec::new();
        for (_key, raw) in rows {
            if let Ok(row) = serde_json::from_slice::<VerificationAuditRow>(&raw) {
                decoded.push(row);
            }
        }
        // Keys are zero-padded by timestamp+seq, so scan order is ascending; newest first.
        decoded.reverse();
        decoded.truncate(max);
        Ok(Json(VerificationAuditResponse {
            ok: true,
            count: decoded.len(),
            rows: decoded,
        }))
    }

    fn verification_db(&self) -> Result<std::sync::Arc<Db>, ErrorData> {
        let state = self.m3_state_handle();
        let mut guard = state.lock().map_err(|_error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                "M3 service state lock poisoned while opening verification storage",
            )
        })?;
        guard
            .ensure_storage()
            .map_err(|error| mcp_error(error.code(), error.to_string()))
    }

    fn verification_write_audit(
        &self,
        row: &VerificationAuditRow,
        read_at_unix_ms: u64,
    ) -> Result<String, ErrorData> {
        let db = self.verification_db()?;
        let seq = AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
        let key = format!("{AUDIT_PREFIX}{read_at_unix_ms:020}-{seq:08}");
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                format!("verification audit row encode failed: {error}"),
            )
        })?;
        db.put_batch(cf::CF_KV, [(key.clone().into_bytes(), encoded)])
            .map_err(|error| {
                mcp_error(
                    error.code(),
                    format!("verification audit row persist failed: {error}"),
                )
            })?;
        db.flush().map_err(|error| {
            mcp_error(
                error.code(),
                format!("verification audit row flush failed: {error}"),
            )
        })?;
        Ok(key)
    }
}

fn unix_time_ms_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// First two + masked middle + last char, so the audit proves a code was surfaced
/// without storing it. Short codes collapse to all-stars.
fn mask_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    if chars.len() <= 3 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..2].iter().collect();
    let last = chars[chars.len() - 1];
    format!("{head}{}{last}", "*".repeat(chars.len() - 3))
}

/// Strip HTML tags + decode the common entities so the OTP extractor sees page
/// text rather than markup. Deliberately simple (no full HTML parse): the goal is
/// to surface code-bearing text, not to reconstruct the DOM.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script = false;
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    let src: Vec<char> = html.chars().collect();
    // Walk byte-synced with the lowercased copy for tag detection; emit original chars.
    let mut ci = 0;
    while i < bytes.len() {
        if !in_tag && bytes[i] == b'<' {
            // crude <script>/<style> skip
            if lower[i..].starts_with("<script") {
                in_script = true;
            } else if lower[i..].starts_with("</script") {
                in_script = false;
            }
            in_tag = true;
        } else if in_tag && bytes[i] == b'>' {
            in_tag = false;
            if ci < src.len() {
                out.push(' ');
            }
        } else if !in_tag && !in_script {
            if let Some(&ch) = src.get(ci) {
                out.push(ch);
            }
        }
        i += 1;
        ci += 1;
    }
    let decoded = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // Collapse runs of whitespace.
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

const CODE_KEYWORDS: &[&str] = &[
    "code", "verification", "verify", "otp", "one-time", "one time", "passcode", "pass code",
    "confirm", "2fa", "two-factor", "two factor", "security code", "login code",
    "authentication", "auth code", "access code", "pin",
];

/// Keyword-gated OTP/verification-code extraction from page text. Google `G-####`
/// codes are always included; bare numeric (4–8 digits) and alphanumeric (5–8,
/// mixed letter+digit) tokens are included only when a code keyword appears within
/// ~60 chars, which suppresses the timestamps/counts/ids that pollute raw page
/// text. Deduplicated; each carries ~50 chars of context for attribution.
pub fn extract_verification_codes(text: &str) -> Vec<VerificationCode> {
    let lower = text.to_ascii_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let n = chars.len();
    let mut out: Vec<VerificationCode> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // A code is associated with a keyword only when the keyword is in the SAME
    // CLAUSE — i.e. no sentence terminator (.!?;: newline) sits between them. This
    // is what distinguishes "your verification code is 481923" (associated) from
    // "...verification code. Order 559001 shipped" (the 559001 belongs to a new
    // sentence). Scans up to 50 chars each side, stopping at the first terminator.
    let is_terminator = |c: char| matches!(c, '.' | '!' | '?' | ';' | '\n' | '\r');
    let keyword_near = |start: usize, end: usize| -> bool {
        // backward clause
        let mut back = String::new();
        let mut k = start;
        let lo = start.saturating_sub(50);
        while k > lo {
            k -= 1;
            if is_terminator(lower_chars[k]) {
                break;
            }
            back.push(lower_chars[k]);
        }
        let back: String = back.chars().rev().collect();
        if CODE_KEYWORDS.iter().any(|kw| back.contains(kw)) {
            return true;
        }
        // forward clause (e.g. "481923 is your login code")
        let mut fwd = String::new();
        let hi = (end + 50).min(n);
        let mut f = end;
        while f < hi {
            if is_terminator(lower_chars[f]) {
                break;
            }
            fwd.push(lower_chars[f]);
            f += 1;
        }
        CODE_KEYWORDS.iter().any(|kw| fwd.contains(kw))
    };
    let context_of = |start: usize, end: usize| -> String {
        let from = start.saturating_sub(25);
        let to = (end + 25).min(n);
        chars[from..to].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let is_tok = |c: char| c.is_ascii_alphanumeric() || c == '-';

    let mut i = 0;
    while i < n {
        if !chars[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        // token boundary: preceding char must not be alnum/- (so we get whole tokens)
        if i > 0 && is_tok(chars[i - 1]) && chars[i - 1] != '-' {
            // still inside a token start scan; advance to token end
        }
        let start = i;
        let mut j = i;
        while j < n && is_tok(chars[j]) {
            j += 1;
        }
        let token: String = chars[start..j].iter().collect();
        let kind = classify_code(&token);
        if let Some(kind) = kind {
            let gated = kind == "google_g" || keyword_near(start, j);
            if gated && seen.insert(token.clone()) {
                out.push(VerificationCode {
                    code: token.clone(),
                    kind: kind.to_owned(),
                    context: context_of(start, j),
                });
            }
        }
        i = j.max(start + 1);
    }
    out
}

fn classify_code(token: &str) -> Option<&'static str> {
    // Google G-#### style.
    if let Some(rest) = token.strip_prefix("G-").or_else(|| token.strip_prefix("g-")) {
        if (4..=8).contains(&rest.len()) && rest.chars().all(|c| c.is_ascii_digit()) {
            return Some("google_g");
        }
    }
    if token.contains('-') {
        return None;
    }
    let len = token.chars().count();
    let all_digit = token.chars().all(|c| c.is_ascii_digit());
    if all_digit && (4..=8).contains(&len) {
        // A bare 4-digit 19xx/20xx is almost always a year, not an OTP — drop it.
        if len == 4
            && let Ok(year) = token.parse::<u32>()
            && (1900..=2099).contains(&year)
        {
            return None;
        }
        return Some("numeric");
    }
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
    let all_upper_alnum = token
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase());
    if has_digit && has_alpha && all_upper_alnum && (5..=8).contains(&len) {
        return Some("alphanumeric");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_google_g_code_unconditionally() {
        let codes = extract_verification_codes("Your Google verification code is G-739212 for sign-in");
        assert!(codes.iter().any(|c| c.code == "G-739212" && c.kind == "google_g"));
    }

    #[test]
    fn numeric_code_only_with_keyword() {
        let with = extract_verification_codes("Your verification code is 481923. Do not share it.");
        assert!(with.iter().any(|c| c.code == "481923" && c.kind == "numeric"));
        // Bare numbers with no code keyword nearby are NOT treated as codes.
        let without = extract_verification_codes("Order 481923 shipped on 2026 at 14:32 to 90210");
        assert!(without.iter().all(|c| c.code != "481923"));
    }

    #[test]
    fn clause_boundary_excludes_adjacent_sentence_numbers() {
        // The keyword "code." ends one sentence; the order number / year / zip in
        // the NEXT sentence must NOT be picked up as codes.
        let text = "Your verification code is 481923. Order 559001 shipped 2026 to 90210.";
        let got: Vec<String> = extract_verification_codes(text)
            .into_iter()
            .map(|c| c.code)
            .collect();
        assert!(got.contains(&"481923".to_owned()), "got {got:?}");
        assert!(!got.contains(&"559001".to_owned()), "got {got:?}");
        assert!(!got.contains(&"2026".to_owned()), "got {got:?}");
        assert!(!got.contains(&"90210".to_owned()), "got {got:?}");
    }

    #[test]
    fn alphanumeric_code_with_keyword() {
        let codes = extract_verification_codes("Enter passcode A1B2C3 to confirm your account");
        assert!(codes.iter().any(|c| c.code == "A1B2C3" && c.kind == "alphanumeric"));
    }

    #[test]
    fn masking_hides_the_code() {
        assert_eq!(mask_code("481923"), "48***3");
        assert_eq!(mask_code("G-739212"), "G-*****2");
        assert_eq!(mask_code("99"), "**");
    }

    #[test]
    fn html_to_text_strips_tags_and_keeps_code() {
        let html = "<div><span>Your code is</span> <b>551823</b><script>var x=999111;</script></div>";
        let text = html_to_text(html);
        assert!(text.contains("Your code is"));
        assert!(text.contains("551823"));
        assert!(!text.contains("999111")); // script content dropped
        let codes = extract_verification_codes(&text);
        assert!(codes.iter().any(|c| c.code == "551823"));
    }
}
