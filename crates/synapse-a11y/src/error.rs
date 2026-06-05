use synapse_core::error_codes;
use thiserror::Error;

pub type A11yResult<T> = Result<T, A11yError>;

#[derive(Debug, Error)]
pub enum A11yError {
    #[error("Windows UI Automation is not available: {detail}")]
    NotAvailable { detail: String },
    #[error("no foreground window is available: {detail}")]
    NoForeground { detail: String },
    #[error("UI Automation element is stale: {detail}")]
    ElementStale { detail: String },
    #[error("UI Automation element has no supported click control pattern: {detail}")]
    ElementPatternUnsupported { detail: String },
    #[error("Chromium DevTools Protocol is unreachable: {detail}")]
    CdpUnreachable { detail: String },
    #[error("Chromium DevTools Protocol attach failed: {detail}")]
    CdpAttachFailed { detail: String },
    #[error("Chromium accessibility tree retrieval failed: {detail}")]
    CdpAxtreeFailed { detail: String },
    #[error("invalid element id: {detail}")]
    InvalidElementId { detail: String },
    #[error("accessibility backend failed: {detail}")]
    Internal { detail: String },
}

impl A11yError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotAvailable { .. } => error_codes::A11Y_NOT_AVAILABLE,
            Self::NoForeground { .. } => error_codes::A11Y_NO_FOREGROUND,
            Self::ElementStale { .. } => error_codes::A11Y_ELEMENT_STALE,
            Self::ElementPatternUnsupported { .. } => {
                error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED
            }
            Self::CdpUnreachable { .. } => error_codes::A11Y_CDP_UNREACHABLE,
            Self::CdpAttachFailed { .. } => error_codes::A11Y_CDP_ATTACH_FAILED,
            Self::CdpAxtreeFailed { .. } => error_codes::A11Y_CDP_AXTREE_FAILED,
            Self::InvalidElementId { .. } | Self::Internal { .. } => error_codes::OBSERVE_INTERNAL,
        }
    }

    #[must_use]
    pub fn not_available(detail: impl Into<String>) -> Self {
        Self::NotAvailable {
            detail: detail.into(),
        }
    }

    #[must_use]
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal {
            detail: detail.into(),
        }
    }
}
