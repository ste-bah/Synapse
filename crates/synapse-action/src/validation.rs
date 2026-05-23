use synapse_core::Action;

use crate::{ActionError, ActionResult};

pub const MAX_DRAG_DISTANCE_PX: f64 = 4096.0;

/// Validates cross-backend action invariants before enqueue or emission.
///
/// # Errors
///
/// Returns `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT` when a `MouseDrag` distance is
/// greater than [`MAX_DRAG_DISTANCE_PX`].
pub fn validate_action(action: &Action) -> ActionResult<()> {
    if let Action::MouseDrag { from, to, .. } = action {
        let distance = from.distance_to(*to);
        if distance > MAX_DRAG_DISTANCE_PX {
            return Err(ActionError::DragDistanceExceedsLimit {
                detail: format!(
                    "drag distance {distance:.3} px exceeds max {MAX_DRAG_DISTANCE_PX:.0} px"
                ),
            });
        }
    }

    Ok(())
}
