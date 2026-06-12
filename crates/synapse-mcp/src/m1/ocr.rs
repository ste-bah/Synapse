use rmcp::ErrorData;
use synapse_core::{ForegroundContext, OcrBackend, OcrResult, OcrWord, Rect, error_codes};
use synapse_perception::{TextRegion, read_text as platform_read_text, read_text_with_provider};

use crate::m1::{M1State, ReadTextParams, current_input, mcp_error, window_input_from_hwnd};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadTextCaptureSource {
    Screen,
    Window { hwnd: i64, window_region: Rect },
}

#[derive(Clone, Debug)]
pub struct ResolvedReadTextRequest {
    pub region: Rect,
    pub capture_source: ReadTextCaptureSource,
    pub requested_backend: OcrBackend,
    pub effective_backend: OcrBackend,
    pub lang_hint: Option<String>,
    pub synthetic: bool,
}

impl ResolvedReadTextRequest {
    #[must_use]
    pub fn lang(&self) -> String {
        self.lang_hint
            .as_deref()
            .map(str::trim)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("und")
            .to_owned()
    }
}

pub fn resolve_read_text_request(
    state: &M1State,
    params: &ReadTextParams,
    target_hwnd: Option<i64>,
) -> Result<ResolvedReadTextRequest, ErrorData> {
    let (region, capture_source) = text_region(state, params, target_hwnd)?;
    validate_ocr_region(region)?;
    Ok(ResolvedReadTextRequest {
        region,
        capture_source,
        requested_backend: params.backend,
        effective_backend: effective_ocr_backend(params.backend)?,
        lang_hint: params.lang_hint.clone(),
        synthetic: state.synthetic.is_some(),
    })
}

pub fn read_text_request_uncached(
    request: &ResolvedReadTextRequest,
) -> Result<OcrResult, ErrorData> {
    if request.synthetic {
        let provider = SyntheticOcrProvider {
            region: request.region,
        };
        let words = read_text_with_provider(&provider, request.region)
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;
        return Ok(ocr_result_from_text_regions(words, request));
    }
    match request.effective_backend {
        OcrBackend::Winrt => {
            let words = platform_read_text(request.region)
                .map_err(|err| mcp_error(err.code(), err.to_string()))?;
            Ok(ocr_result_from_text_regions(words, request))
        }
        OcrBackend::Crnn => Err(crnn_unavailable_error()),
        OcrBackend::Auto => Err(mcp_error(
            error_codes::OCR_BACKEND_UNAVAILABLE,
            "internal OCR backend resolution left backend=auto after request validation",
        )),
    }
}

#[cfg(windows)]
pub fn read_text_request_from_bgra(
    request: &ResolvedReadTextRequest,
    captured: &synapse_capture::CapturedBgraBitmap,
) -> Result<OcrResult, ErrorData> {
    if request.synthetic {
        return read_text_request_uncached(request);
    }
    match request.effective_backend {
        OcrBackend::Winrt => {
            let words = synapse_perception::read_text_from_bgra_bitmap(
                request.region,
                captured.width,
                captured.height,
                &captured.bytes,
            )
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;
            Ok(ocr_result_from_text_regions(words, request))
        }
        OcrBackend::Crnn => Err(crnn_unavailable_error()),
        OcrBackend::Auto => Err(mcp_error(
            error_codes::OCR_BACKEND_UNAVAILABLE,
            "internal OCR backend resolution left backend=auto after request validation",
        )),
    }
}

/// Runs WinRT OCR over a web element's captured BGRA bitmap and returns an
/// `OcrResult` whose word boxes are relative to the captured element (#703).
///
/// Used by the `read_text` handler when `element_id` is a CDP/web node, which
/// the UIA element-bounds path cannot resolve. The bitmap comes from a CDP
/// element-clipped screenshot, so the OCR region is the whole bitmap.
///
/// # Errors
///
/// `OCR_NO_TEXT` if the bitmap dimensions exceed `i32` or OCR finds no text;
/// any `WinRT` OCR backend error from `read_text_from_bgra_bitmap`.
#[cfg(windows)]
pub fn ocr_result_from_web_bitmap(
    width: u32,
    height: u32,
    bgra: &[u8],
    lang_hint: Option<&str>,
) -> Result<OcrResult, ErrorData> {
    let w = i32::try_from(width).map_err(|_| {
        mcp_error(
            error_codes::OCR_NO_TEXT,
            format!("web element OCR bitmap width {width} exceeds i32"),
        )
    })?;
    let h = i32::try_from(height).map_err(|_| {
        mcp_error(
            error_codes::OCR_NO_TEXT,
            format!("web element OCR bitmap height {height} exceeds i32"),
        )
    })?;
    let region = Rect { x: 0, y: 0, w, h };
    let words = synapse_perception::read_text_from_bgra_bitmap(region, width, height, bgra)
        .map_err(|err| mcp_error(err.code(), err.to_string()))?;
    let request = ResolvedReadTextRequest {
        region,
        capture_source: ReadTextCaptureSource::Screen,
        requested_backend: OcrBackend::Auto,
        effective_backend: OcrBackend::Winrt,
        lang_hint: lang_hint.map(str::to_owned),
        synthetic: false,
    };
    Ok(ocr_result_from_text_regions(words, &request))
}

fn text_region(
    state: &M1State,
    params: &ReadTextParams,
    target_hwnd: Option<i64>,
) -> Result<(Rect, ReadTextCaptureSource), ErrorData> {
    let target_hwnd = params.window_hwnd.or(target_hwnd);
    if let Some(region) = params.region {
        let capture_source = match target_hwnd {
            Some(hwnd) => target_window_region_capture_source(hwnd, region)?,
            None => ReadTextCaptureSource::Screen,
        };
        return Ok((region, capture_source));
    }
    if let Some(element_id) = &params.element_id {
        if state.synthetic.is_none() {
            let region = synapse_a11y::element_bounding_rect(element_id).map_err(|err| {
                mcp_error(
                    error_codes::OCR_NO_TEXT,
                    format!("element_id has no live visible OCR region: {err}"),
                )
            })?;
            return Ok((region, ReadTextCaptureSource::Screen));
        }
        let input = current_input(state, 2)?;
        let region = input
            .elements
            .iter()
            .find(|node| &node.element_id == element_id)
            .map(|node| node.bbox)
            .ok_or_else(|| {
                mcp_error(
                    error_codes::OCR_NO_TEXT,
                    "element_id has no visible OCR region",
                )
            })?;
        return Ok((region, ReadTextCaptureSource::Screen));
    }

    let Some(hwnd) = target_hwnd else {
        let input = current_input(state, 2)?;
        let region = input.focused.map(|focused| focused.bbox).ok_or_else(|| {
            mcp_error(
                error_codes::OCR_NO_TEXT,
                "read_text requires region, element_id, or a focused element with a visible OCR region",
            )
        })?;
        return Ok((region, ReadTextCaptureSource::Screen));
    };

    let input = {
        fail_if_minimized_target_needs_uia_region(hwnd)?;
        window_input_from_hwnd(hwnd, 2, state.perception_mode)?
    };
    let absolute_region = input.focused.map(|focused| focused.bbox).ok_or_else(|| {
        mcp_error(
            error_codes::OCR_NO_TEXT,
            "read_text requires region, element_id, or a focused element with a visible OCR region",
        )
    })?;
    let window_region = absolute_region_to_window_region(&input.foreground, absolute_region)?;
    Ok((
        window_region,
        ReadTextCaptureSource::Window {
            hwnd,
            window_region,
        },
    ))
}

fn validate_ocr_region(region: Rect) -> Result<(), ErrorData> {
    if region.w <= 0 || region.h <= 0 {
        return Err(mcp_error(
            error_codes::OCR_NO_TEXT,
            format!(
                "read_text OCR region must be non-empty: bbox=({}, {}, {}, {})",
                region.x, region.y, region.w, region.h
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn target_window_region_capture_source(
    hwnd: i64,
    client_region: Rect,
) -> Result<ReadTextCaptureSource, ErrorData> {
    synapse_capture::validate_hwnd(hwnd).map_err(|error| {
        mcp_error(
            error_codes::TARGET_WINDOW_NOT_FOUND,
            format!("read_text window_hwnd {hwnd:#x} is not a live window: {error}"),
        )
    })?;
    let window_region =
        synapse_capture::client_region_to_window_region(hwnd, client_region).map_err(|error| {
            mcp_error(
                error.code(),
                format!(
                    "read_text could not convert client-relative region {client_region:?} for hwnd {hwnd:#x} into target-window bitmap coordinates: {error}"
                ),
            )
        })?;
    Ok(ReadTextCaptureSource::Window {
        hwnd,
        window_region,
    })
}

#[cfg(not(windows))]
fn target_window_region_capture_source(
    _hwnd: i64,
    _window_region: Rect,
) -> Result<ReadTextCaptureSource, ErrorData> {
    Err(mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "read_text target-window OCR requires Windows window capture",
    ))
}

#[cfg(windows)]
fn fail_if_minimized_target_needs_uia_region(hwnd: i64) -> Result<(), ErrorData> {
    synapse_capture::validate_hwnd(hwnd).map_err(|error| {
        mcp_error(
            error_codes::TARGET_WINDOW_NOT_FOUND,
            format!("read_text window_hwnd {hwnd:#x} is not a live window: {error}"),
        )
    })?;
    if synapse_a11y::is_window_minimized(hwnd).map_err(|error| {
        mcp_error(
            error.code(),
            format!("read_text could not determine minimized state for hwnd {hwnd:#x}: {error}"),
        )
    })? {
        return Err(mcp_error(
            error_codes::A11Y_TARGET_WINDOW_MINIMIZED_UIA_UNAVAILABLE,
            format!(
                "read_text target hwnd {hwnd:#x} is minimized and no explicit window-relative OCR region was supplied; UIA focused-region lookup is unavailable without restoring the window"
            ),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn fail_if_minimized_target_needs_uia_region(_hwnd: i64) -> Result<(), ErrorData> {
    Err(mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "read_text target-window OCR requires Windows UI Automation",
    ))
}

fn absolute_region_to_window_region(
    foreground: &ForegroundContext,
    absolute_region: Rect,
) -> Result<Rect, ErrorData> {
    let window_region = Rect {
        x: absolute_region.x.saturating_sub(foreground.window_bounds.x),
        y: absolute_region.y.saturating_sub(foreground.window_bounds.y),
        w: absolute_region.w,
        h: absolute_region.h,
    };
    if absolute_region.x < foreground.window_bounds.x
        || absolute_region.y < foreground.window_bounds.y
        || absolute_region.x.saturating_add(absolute_region.w)
            > foreground
                .window_bounds
                .x
                .saturating_add(foreground.window_bounds.w)
        || absolute_region.y.saturating_add(absolute_region.h)
            > foreground
                .window_bounds
                .y
                .saturating_add(foreground.window_bounds.h)
    {
        return Err(mcp_error(
            error_codes::OCR_NO_TEXT,
            format!(
                "focused OCR region {absolute_region:?} is outside target window bounds {:?}",
                foreground.window_bounds
            ),
        ));
    }
    Ok(window_region)
}

pub fn effective_ocr_backend(backend: OcrBackend) -> Result<OcrBackend, ErrorData> {
    match backend {
        OcrBackend::Winrt | OcrBackend::Auto => Ok(OcrBackend::Winrt),
        OcrBackend::Crnn => Err(crnn_unavailable_error()),
    }
}

fn crnn_unavailable_error() -> ErrorData {
    mcp_error(
        error_codes::OCR_BACKEND_UNAVAILABLE,
        "CRNN OCR backend is declared in schema but no CRNN runtime/model provider is wired on this host; request backend=winrt or backend=auto",
    )
}

fn ocr_result_from_text_regions(
    regions: Vec<TextRegion>,
    request: &ResolvedReadTextRequest,
) -> OcrResult {
    let full_text = regions
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let confidence = aggregate_confidence(&regions);
    OcrResult {
        full_text,
        words: regions
            .into_iter()
            .map(|word| OcrWord {
                confidence: identifier_aware_confidence(&word.text, word.confidence),
                text: word.text,
                bbox: word.bbox,
            })
            .collect(),
        confidence,
        region: request.region,
        lang: request.lang(),
    }
}

fn aggregate_confidence(regions: &[TextRegion]) -> f32 {
    if regions.is_empty() {
        return 0.0;
    }
    let sum = regions
        .iter()
        .map(|word| identifier_aware_confidence(&word.text, word.confidence))
        .sum::<f32>();
    let count = u16::try_from(regions.len()).unwrap_or(u16::MAX);
    sum / f32::from(count)
}

fn identifier_aware_confidence(text: &str, confidence: f32) -> f32 {
    let confidence = normalize_confidence(confidence);
    if is_ambiguous_identifier_token(text) {
        confidence.min(AMBIGUOUS_IDENTIFIER_CONFIDENCE_CAP)
    } else {
        confidence
    }
}

const AMBIGUOUS_IDENTIFIER_CONFIDENCE_CAP: f32 = 0.74;

fn is_ambiguous_identifier_token(text: &str) -> bool {
    let token = text.trim_matches(|ch: char| !identifier_char(ch));
    if token.len() < 2 || !token.chars().any(ambiguous_identifier_char) {
        return false;
    }
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    let has_separator = token.chars().any(identifier_separator);
    if looks_like_ordinary_lowercase_word(token) && !has_digit && !has_separator {
        return false;
    }
    let lower = token.to_ascii_lowercase();
    let short_all_ambiguous =
        token.chars().count() <= 4 && token.chars().all(ambiguous_identifier_char);
    let has_ambiguous_pair = [
        "v1", "vl", "vi", "i1", "l1", "1l", "1i", "o0", "0o", "ol", "lo",
    ]
    .iter()
    .any(|pair| lower.contains(pair));
    has_digit
        || has_separator
        || short_all_ambiguous
        || has_ambiguous_pair
        || lower == "vl"
        || lower == "v1"
}

fn looks_like_ordinary_lowercase_word(token: &str) -> bool {
    token.len() >= 3
        && token
            .chars()
            .all(|ch| ch.is_ascii_lowercase() && ch.is_ascii_alphabetic())
}

const fn identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '_' | '.' | ':' | '/' | '\\' | '|' | '[' | ']' | '(' | ')'
        )
}

const fn identifier_separator(ch: char) -> bool {
    matches!(ch, '-' | '_' | '.' | ':' | '/' | '\\' | '|')
}

const fn ambiguous_identifier_char(ch: char) -> bool {
    matches!(ch, '0' | 'O' | 'o' | '1' | 'I' | 'l' | '|' | 'v' | 'V')
}

const fn normalize_confidence(confidence: f32) -> f32 {
    if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

struct SyntheticOcrProvider {
    region: Rect,
}

impl synapse_perception::OcrProvider for SyntheticOcrProvider {
    fn read_text(&self, _region: Rect) -> synapse_perception::PerceptionResult<Vec<TextRegion>> {
        Ok(vec![TextRegion {
            text: "Synapse".to_owned(),
            bbox: Rect {
                x: self.region.x.saturating_add(4),
                y: self.region.y.saturating_add(4),
                w: 72,
                h: 18,
            },
            confidence: 0.99,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AMBIGUOUS_IDENTIFIER_CONFIDENCE_CAP, identifier_aware_confidence,
        is_ambiguous_identifier_token,
    };

    #[test]
    fn caps_short_tokens_collapsed_to_only_ambiguous_glyphs() {
        for token in ["II", "OO", "ll", "00", "v1", "vl", "AMBIG724v1"] {
            assert!(
                is_ambiguous_identifier_token(token),
                "{token} should be ambiguity-capped"
            );
            assert_eq!(
                identifier_aware_confidence(token, 1.0),
                AMBIGUOUS_IDENTIFIER_CONFIDENCE_CAP
            );
        }
    }

    #[test]
    fn does_not_cap_ordinary_words_containing_some_ambiguous_letters() {
        assert!(!is_ambiguous_identifier_token("look"));
        assert_eq!(identifier_aware_confidence("look", 0.96), 0.96);
    }
}
