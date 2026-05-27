use serde::{Deserialize, Serialize};
use synapse_core::{HudRegion, Rect, WindowEdge};

use crate::{PerceptionError, PerceptionResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudAnchor {
    None,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HudAnchorRegion {
    pub anchor: HudAnchor,
    pub x_offset: i32,
    pub y_offset: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedHudRegion {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ResolvedHudRegion {
    #[must_use]
    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    #[must_use]
    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    #[must_use]
    pub const fn rect(self) -> Rect {
        Rect {
            x: self.left,
            y: self.top,
            w: self.width(),
            h: self.height(),
        }
    }

    #[must_use]
    pub const fn as_ltrb(self) -> (i32, i32, i32, i32) {
        (self.left, self.top, self.right, self.bottom)
    }
}

impl From<WindowEdge> for HudAnchor {
    fn from(value: WindowEdge) -> Self {
        match value {
            WindowEdge::TopLeft => Self::TopLeft,
            WindowEdge::TopRight => Self::TopRight,
            WindowEdge::BottomLeft => Self::BottomLeft,
            WindowEdge::BottomRight => Self::BottomRight,
            WindowEdge::Center => Self::Center,
        }
    }
}

/// Resolves a profile HUD region against a foreground window client rectangle.
///
/// `Rect` uses x/y/width/height, while `ResolvedHudRegion` exposes the
/// left/top/right/bottom tuple used by Win32 client rectangles and issue
/// acceptance readbacks.
///
/// # Errors
///
/// Returns [`PerceptionError::HudExtractionFailed`] when dimensions are
/// non-positive, fractional regions do not fit the unit window, window
/// dimensions are invalid for anchored/fractional regions, or coordinate math
/// would overflow.
pub fn resolve_hud_region(
    region: &HudRegion,
    window_client: Rect,
) -> PerceptionResult<ResolvedHudRegion> {
    match *region {
        HudRegion::Absolute { x, y, w, h } => resolve_absolute(x, y, w, h),
        HudRegion::FractionOfWindow { x, y, w, h } => {
            resolve_fraction_of_window(window_client, x, y, w, h)
        }
        HudRegion::AnchoredToEdge {
            edge,
            x_offset,
            y_offset,
            w,
            h,
        } => resolve_anchor_region(
            HudAnchorRegion {
                anchor: edge.into(),
                x_offset,
                y_offset,
                w,
                h,
            },
            window_client,
        ),
    }
}

/// Resolves a profile HUD region and returns the existing `Rect` shape.
///
/// # Errors
///
/// Returns [`PerceptionError::HudExtractionFailed`] for the same invalid
/// geometry and overflow cases as [`resolve_hud_region`].
pub fn resolve_hud_region_rect(region: &HudRegion, window_client: Rect) -> PerceptionResult<Rect> {
    Ok(resolve_hud_region(region, window_client)?.rect())
}

/// Resolves a compact anchor spec against a foreground window client rectangle.
///
/// Offsets locate the top-left corner of the HUD crop relative to the selected
/// anchor point. `HudAnchor::None` treats `x_offset`/`y_offset` as absolute
/// screen coordinates and ignores `window_client`.
///
/// # Errors
///
/// Returns [`PerceptionError::HudExtractionFailed`] when region dimensions are
/// non-positive, an anchored region has an invalid window client rectangle, or
/// coordinate math would overflow.
pub fn resolve_anchor_region(
    region: HudAnchorRegion,
    window_client: Rect,
) -> PerceptionResult<ResolvedHudRegion> {
    validate_dimensions(region.w, region.h)?;

    if region.anchor == HudAnchor::None {
        return resolve_absolute(region.x_offset, region.y_offset, region.w, region.h);
    }

    validate_window(window_client)?;
    let (base_x, base_y) = anchor_point(region.anchor, window_client)?;
    let left = checked_add(base_x, region.x_offset, "anchor x plus x_offset")?;
    let top = checked_add(base_y, region.y_offset, "anchor y plus y_offset")?;
    resolved_from_origin(left, top, region.w, region.h)
}

fn resolve_absolute(x: i32, y: i32, w: i32, h: i32) -> PerceptionResult<ResolvedHudRegion> {
    validate_dimensions(w, h)?;
    resolved_from_origin(x, y, w, h)
}

fn resolve_fraction_of_window(
    window_client: Rect,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> PerceptionResult<ResolvedHudRegion> {
    validate_window(window_client)?;
    validate_fraction(x, y, w, h)?;

    let left_offset = rounded_fraction(window_client.w, x, "fraction x")?;
    let top_offset = rounded_fraction(window_client.h, y, "fraction y")?;
    let width = rounded_fraction(window_client.w, w, "fraction w")?;
    let height = rounded_fraction(window_client.h, h, "fraction h")?;
    validate_dimensions(width, height)?;

    let left = checked_add(window_client.x, left_offset, "window x plus fraction x")?;
    let top = checked_add(window_client.y, top_offset, "window y plus fraction y")?;
    resolved_from_origin(left, top, width, height)
}

fn anchor_point(anchor: HudAnchor, window_client: Rect) -> PerceptionResult<(i32, i32)> {
    let right = checked_add(window_client.x, window_client.w, "window x plus width")?;
    let bottom = checked_add(window_client.y, window_client.h, "window y plus height")?;
    match anchor {
        HudAnchor::None | HudAnchor::TopLeft => Ok((window_client.x, window_client.y)),
        HudAnchor::TopRight => Ok((right, window_client.y)),
        HudAnchor::BottomLeft => Ok((window_client.x, bottom)),
        HudAnchor::BottomRight => Ok((right, bottom)),
        HudAnchor::Center => {
            let half_w = window_client.w / 2;
            let half_h = window_client.h / 2;
            Ok((
                checked_add(window_client.x, half_w, "window x plus half width")?,
                checked_add(window_client.y, half_h, "window y plus half height")?,
            ))
        }
    }
}

fn validate_window(window_client: Rect) -> PerceptionResult<()> {
    if window_client.w <= 0 || window_client.h <= 0 {
        return Err(hud_error(format!(
            "window client rect must have positive dimensions: {window_client:?}"
        )));
    }
    Ok(())
}

fn validate_dimensions(w: i32, h: i32) -> PerceptionResult<()> {
    if w <= 0 || h <= 0 {
        return Err(hud_error(format!(
            "HUD region dimensions must be positive: w={w} h={h}"
        )));
    }
    Ok(())
}

fn validate_fraction(x: f32, y: f32, w: f32, h: f32) -> PerceptionResult<()> {
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
        return Err(hud_error(format!(
            "fractional HUD region must be finite: x={x} y={y} w={w} h={h}"
        )));
    }
    if x < 0.0 || y < 0.0 || w <= 0.0 || h <= 0.0 || x + w > 1.0 || y + h > 1.0 {
        return Err(hud_error(format!(
            "fractional HUD region must fit inside the unit window: x={x} y={y} w={w} h={h}"
        )));
    }
    Ok(())
}

fn rounded_fraction(size: i32, fraction: f32, field: &str) -> PerceptionResult<i32> {
    let value = (f64::from(size) * f64::from(fraction)).round();
    if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(hud_error(format!("{field} resolved outside i32 range")));
    }
    #[allow(clippy::cast_possible_truncation)]
    let rounded = value as i32;
    Ok(rounded)
}

fn resolved_from_origin(
    left: i32,
    top: i32,
    w: i32,
    h: i32,
) -> PerceptionResult<ResolvedHudRegion> {
    validate_dimensions(w, h)?;
    Ok(ResolvedHudRegion {
        left,
        top,
        right: checked_add(left, w, "HUD region left plus width")?,
        bottom: checked_add(top, h, "HUD region top plus height")?,
    })
}

fn checked_add(left: i32, right: i32, context: &str) -> PerceptionResult<i32> {
    left.checked_add(right)
        .ok_or_else(|| hud_error(format!("{context} overflowed: {left} + {right}")))
}

fn hud_error(detail: impl Into<String>) -> PerceptionError {
    PerceptionError::HudExtractionFailed {
        detail: detail.into(),
    }
}
