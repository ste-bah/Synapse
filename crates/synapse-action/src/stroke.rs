use synapse_core::{
    HumanizeParams, PathPoint, PathSpec, Point, StrokeMotionModel, StrokeTiming, VelocityProfile,
};

use crate::{
    ArcLengthPath, HumanizeError, PathError, TimedPathPoint, VelocityError, humanize_timed_path,
    position_at_time,
};

pub const STROKE_TICK_MS: f64 = 1.0;
const WIND_MOUSE_TARGET_TOLERANCE_PX: f64 = 1.0;
const WIND_MOUSE_MAX_POINTS: usize = 60_001;

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
    #[error("wind_mouse motion model requires a line path, got {path_kind}")]
    WindMouseRequiresLine { path_kind: &'static str },
    #[error("wind_mouse parameter {field} must be finite and greater than zero, got {value}")]
    InvalidWindMouseParameter { field: &'static str, value: f64 },
    #[error("wind_mouse generated a non-finite point at index {index}: x={x} y={y}")]
    WindMouseNonFinitePoint { index: usize, x: f64, y: f64 },
    #[error(
        "wind_mouse did not converge within {max_points} points; remaining distance {remaining_distance_px:.3}px"
    )]
    WindMouseDidNotConverge {
        max_points: usize,
        remaining_distance_px: f64,
    },
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
    motion_model: StrokeMotionModel,
    humanize: Option<HumanizeParams>,
) -> StrokeResult<StrokePlan> {
    let arclen = ArcLengthPath::new(path)?;
    let path_length_px = arclen.length();
    let duration_ms = duration_for_timing(timing, path_length_px)?;
    let samples = match motion_model {
        StrokeMotionModel::Path => {
            sample_tick_timed_path(&arclen, profile, sample_count(duration_ms)?, duration_ms)?
        }
        StrokeMotionModel::WindMouse {
            gravity,
            wind,
            max_step,
            damped_distance,
            seed,
        } => sample_wind_mouse_timed_path(
            path,
            duration_ms,
            WindMouseParams {
                gravity,
                wind,
                max_step,
                damped_distance,
                seed,
            },
        )?,
    };
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

#[derive(Copy, Clone, Debug)]
struct WindMouseParams {
    gravity: f64,
    wind: f64,
    max_step: f64,
    damped_distance: f64,
    seed: Option<u64>,
}

fn sample_wind_mouse_timed_path(
    path: &PathSpec,
    duration_ms: f64,
    params: WindMouseParams,
) -> StrokeResult<Vec<TimedPathPoint>> {
    let (from, to) = line_endpoints(path)?;
    validate_wind_mouse_params(params)?;
    let points = wind_mouse_points(from, to, params)?;
    timed_points_from_wind_mouse(points, duration_ms)
}

fn line_endpoints(path: &PathSpec) -> StrokeResult<(PathPoint, PathPoint)> {
    match path {
        PathSpec::Line { from, to } => Ok((*from, *to)),
        other => Err(StrokeError::WindMouseRequiresLine {
            path_kind: path_kind(other),
        }),
    }
}

fn validate_wind_mouse_params(params: WindMouseParams) -> StrokeResult<()> {
    validate_positive_wind_mouse("gravity", params.gravity)?;
    validate_positive_wind_mouse("wind", params.wind)?;
    validate_positive_wind_mouse("max_step", params.max_step)?;
    validate_positive_wind_mouse("damped_distance", params.damped_distance)
}

fn validate_positive_wind_mouse(field: &'static str, value: f64) -> StrokeResult<()> {
    if !value.is_finite() || value <= 0.0 {
        return Err(StrokeError::InvalidWindMouseParameter { field, value });
    }
    Ok(())
}

fn wind_mouse_points(
    from: PathPoint,
    to: PathPoint,
    params: WindMouseParams,
) -> StrokeResult<Vec<PathPoint>> {
    let mut rng = DeterministicRng::new(wind_mouse_seed(from, to, params));
    let mut points = Vec::with_capacity(256);
    points.push(from);

    let mut current = from;
    let mut velocity = PathPoint::new(0.0, 0.0);
    let mut wind = PathPoint::new(0.0, 0.0);
    let sqrt3 = 3.0_f64.sqrt();
    let sqrt5 = 5.0_f64.sqrt();

    for index in 1..WIND_MOUSE_MAX_POINTS.saturating_sub(1) {
        let delta = sub(to, current);
        let distance = delta.x.hypot(delta.y);
        if distance <= WIND_MOUSE_TARGET_TOLERANCE_PX {
            break;
        }

        if distance >= params.damped_distance {
            let wind_limit = params.wind.min(distance);
            wind = PathPoint::new(
                wind.x / sqrt3 + rng.symmetric(wind_limit) / sqrt5,
                wind.y / sqrt3 + rng.symmetric(wind_limit) / sqrt5,
            );
        } else {
            let damping = (distance / params.damped_distance).clamp(0.0, 1.0);
            wind = scale(wind, damping / sqrt3);
            velocity = scale(velocity, 0.75);
        }

        velocity = add(
            add(velocity, wind),
            scale(delta, params.gravity / distance.max(f64::EPSILON)),
        );
        let speed = velocity.x.hypot(velocity.y);
        if speed > params.max_step {
            let clipped = params.max_step * rng.range(0.5, 1.0);
            velocity = scale(velocity, clipped / speed);
        }

        let next = add(current, velocity);
        if !next.is_finite() {
            return Err(StrokeError::WindMouseNonFinitePoint {
                index,
                x: next.x,
                y: next.y,
            });
        }
        current = next;
        points.push(current);
    }

    let remaining = current.distance_to(to);
    if remaining > params.max_step.max(WIND_MOUSE_TARGET_TOLERANCE_PX) {
        return Err(StrokeError::WindMouseDidNotConverge {
            max_points: WIND_MOUSE_MAX_POINTS,
            remaining_distance_px: remaining,
        });
    }
    if points
        .last()
        .is_none_or(|point| point.distance_to(to) > f64::EPSILON)
    {
        points.push(to);
    }
    Ok(points)
}

fn timed_points_from_wind_mouse(
    points: Vec<PathPoint>,
    duration_ms: f64,
) -> StrokeResult<Vec<TimedPathPoint>> {
    if points.len() < 2 {
        return Err(PathError::InvalidSampleCount {
            samples: points.len(),
        }
        .into());
    }
    if points.len() > WIND_MOUSE_MAX_POINTS {
        return Err(StrokeError::SampleCountOverflow { duration_ms });
    }

    let mut cumulative = Vec::with_capacity(points.len());
    let mut arclen = 0.0;
    cumulative.push(arclen);
    for pair in points.windows(2) {
        arclen += pair[0].distance_to(pair[1]);
        cumulative.push(arclen);
    }

    let last = points.len() - 1;
    Ok(points
        .into_iter()
        .enumerate()
        .map(|(index, point)| TimedPathPoint {
            elapsed_ms: duration_ms * index as f64 / last as f64,
            arclen: cumulative[index],
            point,
        })
        .collect())
}

fn wind_mouse_seed(from: PathPoint, to: PathPoint, params: WindMouseParams) -> u64 {
    if let Some(seed) = params.seed {
        return seed;
    }
    let mut seed = 0x6a09_e667_f3bc_c909;
    mix_u64(&mut seed, from.x.to_bits());
    mix_u64(&mut seed, from.y.to_bits());
    mix_u64(&mut seed, to.x.to_bits());
    mix_u64(&mut seed, to.y.to_bits());
    mix_u64(&mut seed, params.gravity.to_bits());
    mix_u64(&mut seed, params.wind.to_bits());
    mix_u64(&mut seed, params.max_step.to_bits());
    mix_u64(&mut seed, params.damped_distance.to_bits());
    seed
}

const fn mix_u64(seed: &mut u64, value: u64) {
    *seed ^= value
        .wrapping_add(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(*seed << 6)
        .wrapping_add(*seed >> 2);
}

struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_unit(&mut self) -> f64 {
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        self.state = value;
        ((value >> 11) as f64) / ((1_u64 << 53) as f64)
    }

    fn symmetric(&mut self, magnitude: f64) -> f64 {
        self.range(-magnitude, magnitude)
    }

    fn range(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next_unit()
    }
}

fn path_kind(path: &PathSpec) -> &'static str {
    match path {
        PathSpec::Line { .. } => "line",
        PathSpec::Arc { .. } => "arc",
        PathSpec::Circle { .. } => "circle",
        PathSpec::CubicBezier { .. } => "cubic_bezier",
        PathSpec::Polyline { .. } => "polyline",
        PathSpec::CatmullRom { .. } => "catmull_rom",
    }
}

const fn add(left: PathPoint, right: PathPoint) -> PathPoint {
    PathPoint::new(left.x + right.x, left.y + right.y)
}

const fn sub(left: PathPoint, right: PathPoint) -> PathPoint {
    PathPoint::new(left.x - right.x, left.y - right.y)
}

const fn scale(point: PathPoint, factor: f64) -> PathPoint {
    PathPoint::new(point.x * factor, point.y * factor)
}
