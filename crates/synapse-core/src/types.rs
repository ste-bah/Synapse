use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Software,
    Vigem,
    Hardware,
    Auto,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionMode {
    A11yOnly,
    PixelOnly,
    Hybrid,
    Auto,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[must_use]
    pub fn distance_to(self, other: Self) -> f64 {
        let dx = f64::from(self.x) - f64::from(other.x);
        let dy = f64::from(self.y) - f64::from(other.y);
        dx.hypot(dy)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// Returns true when a point is inside this rectangle.
    ///
    /// The right and bottom edges are exclusive. Non-positive width or height
    /// rectangles are empty.
    #[must_use]
    pub const fn contains(self, point: Point) -> bool {
        if self.w <= 0 || self.h <= 0 {
            return false;
        }

        let right = self.x.saturating_add(self.w);
        let bottom = self.y.saturating_add(self.h);
        point.x >= self.x && point.x < right && point.y >= self.y && point.y < bottom
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

pub type SessionId = String;
pub type ElementId = String;
pub type EntityId = String;
pub type ReflexId = String;
pub type SubscriptionId = String;
pub type ProfileId = String;

#[must_use]
pub fn new_session_id() -> SessionId {
    uuid::Uuid::now_v7().to_string()
}

#[must_use]
pub fn new_reflex_id() -> ReflexId {
    uuid::Uuid::now_v7().to_string()
}

#[must_use]
pub fn new_subscription_id() -> SubscriptionId {
    uuid::Uuid::now_v7().to_string()
}

#[must_use]
pub fn element_id(hwnd: i64, runtime_id_hex: &str) -> ElementId {
    format!("{hwnd}:{runtime_id_hex}")
}

#[must_use]
pub fn entity_id(track: u64) -> EntityId {
    format!("track:{track}")
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Health {
    pub ok: bool,
    pub version: String,
    pub build: String,
    pub uptime_s: u64,
    pub subsystems: BTreeMap<String, SubsystemHealth>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SubsystemHealth {
    pub status: String,
    pub detail: Option<String>,
}
