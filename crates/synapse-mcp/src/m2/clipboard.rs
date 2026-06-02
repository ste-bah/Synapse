use std::time::Instant;

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{ActionError, ClipboardFormat};
use synapse_core::error_codes;

use crate::m1::mcp_error;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClipboardParams {
    pub verb: ActClipboardVerb,
    pub text: Option<String>,
    #[serde(default = "default_clipboard_format")]
    #[schemars(default = "default_clipboard_format")]
    pub format: ActClipboardFormat,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActClipboardVerb {
    Read,
    Write,
    Clear,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActClipboardFormat {
    Text,
    Unicode,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActClipboardResponse {
    pub ok: bool,
    pub verb: ActClipboardVerb,
    pub format: ActClipboardFormat,
    pub written: bool,
    pub cleared: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_len: Option<usize>,
    pub elapsed_ms: u32,
}

pub async fn act_clipboard(params: ActClipboardParams) -> Result<ActClipboardResponse, ErrorData> {
    validate_params(&params)?;
    let started = Instant::now();
    let format = params.format.to_clipboard_format();
    let response = match params.verb {
        ActClipboardVerb::Read => {
            let text = synapse_action::read_clipboard_text(format)
                .map_err(|error| action_error_to_mcp(&error))?;
            ActClipboardResponse {
                ok: true,
                verb: params.verb,
                format: params.format,
                written: false,
                cleared: false,
                text_len: Some(text.chars().count()),
                text: Some(text),
                elapsed_ms: elapsed_ms(started),
            }
        }
        ActClipboardVerb::Write => {
            let text = params
                .text
                .as_deref()
                .ok_or_else(missing_write_text_error)?;
            synapse_action::write_clipboard_text(format, text)
                .map_err(|error| action_error_to_mcp(&error))?;
            ActClipboardResponse {
                ok: true,
                verb: params.verb,
                format: params.format,
                written: true,
                cleared: false,
                text: None,
                text_len: Some(text.chars().count()),
                elapsed_ms: elapsed_ms(started),
            }
        }
        ActClipboardVerb::Clear => {
            synapse_action::clear_clipboard().map_err(|error| action_error_to_mcp(&error))?;
            ActClipboardResponse {
                ok: true,
                verb: params.verb,
                format: params.format,
                written: false,
                cleared: true,
                text: None,
                text_len: None,
                elapsed_ms: elapsed_ms(started),
            }
        }
    };
    tracing::info!(
        code = "M2_ACT_CLIPBOARD_READBACK",
        kind = "act_clipboard",
        verb = response.verb.as_str(),
        format = response.format.as_str(),
        written = response.written,
        cleared = response.cleared,
        text_len = response.text_len,
        "readback=clipboard_backend tool=act_clipboard after_operation_readback"
    );
    Ok(response)
}

impl ActClipboardFormat {
    const fn to_clipboard_format(self) -> ClipboardFormat {
        match self {
            Self::Text => ClipboardFormat::Text,
            Self::Unicode => ClipboardFormat::Unicode,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Unicode => "unicode",
        }
    }
}

impl ActClipboardVerb {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Clear => "clear",
        }
    }
}

fn validate_params(params: &ActClipboardParams) -> Result<(), ErrorData> {
    match params.verb {
        ActClipboardVerb::Write => {
            if params.text.is_none() {
                return Err(missing_write_text_error());
            }
        }
        ActClipboardVerb::Read | ActClipboardVerb::Clear => {
            if params.text.is_some() {
                return Err(mcp_error(
                    error_codes::TOOL_PARAMS_INVALID,
                    "act_clipboard text is only valid with verb=write",
                ));
            }
        }
    }
    Ok(())
}

fn missing_write_text_error() -> ErrorData {
    mcp_error(
        error_codes::TOOL_PARAMS_INVALID,
        "act_clipboard verb=write requires text",
    )
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn default_clipboard_format() -> ActClipboardFormat {
    ActClipboardFormat::Unicode
}

fn elapsed_ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_format_non_ascii_reaches_backend_validation() {
        let params = ActClipboardParams {
            verb: ActClipboardVerb::Write,
            text: Some("unicode-clipboard-edge-雪".to_owned()),
            format: ActClipboardFormat::Text,
        };

        assert!(validate_params(&params).is_ok());
    }
}
