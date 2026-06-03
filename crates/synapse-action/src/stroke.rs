use synapse_core::{HumanizeParams, PathPoint, PathSpec, Point, StrokeTiming, VelocityProfile};

use crate::{
    ArcLengthPath, HumanizeError, PathError, TimedPathPoint, VelocityError, humanize_timed_path,
    position_at_time,
};

pub const STROKE_TICK_MS: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct StrokePlan {
    pub samples: Vec<TimedPathPoint>,
    pub duration_ms: f64,
    pub path_length_px: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum StrokeError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Velocity(#[from] VelocityError),
    #[error(transparent)]
    Humanize(#[from] HumanizeError),
    #[error("stroke duration_ms must be finite and greater than zero, got {duration_ms}")]
    InvalidDuration { duration_ms: f64 },
    #[error("stroke speed px_per_sec must be finite and greater than zero, got {px_per_sec}")]
    InvalidSpeed { px_per_sec: f64 },
    #[error("stroke sample count overflow for duration_ms={duration_ms}")]
    SampleCountOverflow { duration_ms: f64 },
    #[error("stroke point {index} is outside i32 screen coordinate range: x={x} y={y}")]
    ScreenPointOutOfRange { index: usize, x: f64, y: f64 },
}

pub type StrokeResult<T> = Result<T, StrokeError>;

/// Plans a mouse stroke as a one-millisecond-cadence timed point stream.
///
/// # Errors
///
/// Returns [`StrokeError`] when the path is invalid, the timing cannot produce
/// a positive finite duration, the velocity profile rejects a sample, the
/// humanization layer rejects parameters, or planned points cannot fit in the
/// backend screen coordinate type.
pub fn plan_timed_stroke(
    path: &PathSpec,
    profile: VelocityProfile,
    timing: &StrokeTiming,
    humanize: Option<HumanizeParams>,
) -> StrokeResult<StrokePlan> {
    let arclen = ArcLengthPath::new(path)?;
    let path_length_px = arclen.length();
    let duration_ms = duration_for_timing(timing, path_length_px)?;
    let samples =
        sample_tick_timed_path(&arclen, profile, sample_count(duration_ms)?, duration_ms)?;
    let samples = humanize_timed_path(&samples, humanize)?;
    Ok(StrokePlan {
        samples,
        duration_ms,
        path_length_px,
    })
}

/// Converts a floating path point into a concrete physical screen point.
///
/// # Errors
///
/// Returns [`StrokeError::ScreenPointOutOfRange`] when either coordinate is
/// non-finite or outside the `i32` screen coordinate range used by the software
/// input backend.
pub fn screen_point_from_path_point(point: PathPoint, index: usize) -> StrokeResult<Point> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < f64::from(i32::MIN)
        || point.x > f64::from(i32::MAX)
        || point.y < f64::from(i32::MIN)
        || point.y > f64::from(i32::MAX)
    {
        return Err(StrokeError::ScreenPointOutOfRange {
            index,
            x: point.x,
            y: point.y,
        });
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "validated finite i32-range stroke coordinates are rounded to physical pixels"
    )]
    Ok(Point {
        x: point.x.round() as i32,
        y: point.y.round() as i32,
    })
}

fn duration_for_timing(timing: &StrokeTiming, path_length_px: f64) -> StrokeResult<f64> {
    let duration_ms = match timing {
        StrokeTiming::DurationMs { duration_ms } => f64::from(*duration_ms),
        StrokeTiming::SpeedPxPerSec { px_per_sec } => {
            if !px_per_sec.is_finite() || *px_per_sec <= 0.0 {
                return Err(StrokeError::InvalidSpeed {
                    px_per_sec: *px_per_sec,
                });
            }
            path_length_px / px_per_sec * 1000.0
        }
    };

    if !duration_ms.is_finite() || duration_ms <= 0.0 {
        return Err(StrokeError::InvalidDuration { duration_ms });
    }
    Ok(duration_ms)
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "stroke duration is validated finite/positive before deriving a bounded sample count"
)]
fn sample_count(duration_ms: f64) -> StrokeResult<usize> {
    let count = (duration_ms / STROKE_TICK_MS).ceil() + 1.0;
    if !count.is_finite() || count > usize::MAX as f64 {
        return Err(StrokeError::SampleCountOverflow { duration_ms });
    }
    Ok((count as usize).max(2))
}

#[allow(
    clippy::cast_precision_loss,
    reason = "stroke sample indices are bounded by the planned sample count and converted to normalized fractions"
)]
fn sample_tick_timed_path(
    path: &ArcLengthPath<'_>,
    profile: VelocityProfile,
    samples: usize,
    duration_ms: f64,
) -> StrokeResult<Vec<TimedPathPoint>> {
    if samples < 2 {
        return Err(PathError::InvalidSampleCount { samples }.into());
    }

    let last = samples - 1;
    let mut timed = Vec::with_capacity(samples);
    for index in 0..samples {
        let elapsed_fraction = index as f64 / last as f64;
        let position_fraction = position_at_time(profile, elapsed_fraction)?;
        let arclen = path.length() * position_fraction;
        timed.push(TimedPathPoint {
            elapsed_ms: duration_ms * elapsed_fraction,
            arclen,
            point: path.point_at_arclen(arclen)?,
        });
    }
    Ok(timed)
}
