pub mod anchor;
pub mod extractor;

pub use anchor::{
    HudAnchor, HudAnchorRegion, ResolvedHudRegion, resolve_anchor_region, resolve_hud_region,
    resolve_hud_region_rect,
};
pub use extractor::{
    ExtractionSource, FieldExtraction, FieldExtractionRequest, extract_field, parse_hud_text,
};
