use synapse_core::{HumanizeParams, PathPoint};

use crate::TimedPathPoint;

const DEFAULT_CORRECTION_MS: f64 = 16.0;
const EPSILON: f64 = 1.0e-9;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum HumanizeError {
    #[error("humanized path requires at least 2 timed samples, got {samples}")]
    NotEnoughSamples { samples: usize },
    #[error("humanize parameter {field} must be finite and non-negative, got {value}")]
    InvalidNonNegative { field: &'static str, value: f32 },
    #[error("humanize probability {field} must be finite and within [0,1], got {value}")]
    InvalidProbability { field: &'static str, value: f32 },
    #[error("humanize factor {field} must be finite and at least 1.0, got {value}")]
    InvalidOvershootFactor { field: &'static str, value: f32 },
    #[error("humanize overshoot factor range must be ordered, got min={min} max={max}")]
    InvalidOvershootRange { min: f32, max: f32 },
    #[error("humanize micro-pause range must be ordered, got min={min_ms} max={max_ms}")]
    InvalidMicroPauseRange { min_ms: u32, max_ms: u32 },
    #[error("timed path sample {index} has non-finite fields")]
    NonFiniteSample { index: usize },
    #[error("timed path sample {index} timestamp is not monotonic")]
    NonMonotonicTimestamp { index: usize },
}

pub type HumanizeResult<T> = Result<T, HumanizeError>;

/// Applies deterministic human-like tremor, overshoot, and micro-pauses to a
/// timed path.
///
/// # Errors
///
/// Returns [`HumanizeError`] when the sample stream is too short, contains
/// non-finite or non-monotonic samples, or when the supplied parameters are
/// outside their accepted ranges.
pub fn humanize_timed_path(
    samples: &[TimedPathPoint],
    params: Option<HumanizeParams>,
) -> HumanizeResult<Vec<TimedPathPoint>> {
    let Some(params) = params else {
        return Ok(samples.to_vec());
    };
    validate_samples(samples)?;
    validate_params(params)?;

    if humanization_disabled(params) {
        return Ok(samples.to_vec());
    }

    let mut rng = DeterministicRng::new(effective_seed(params, samples));
    let mut result = Vec::with_capacity(samples.len());
    let mut pause_offset_ms = 0.0;

    for (index, sample) in samples.iter().copied().enumerate() {
        let is_endpoint = index == 0 || index + 1 == samples.len();
        let mut point = sample.point;
        if !is_endpoint {
            let velocity = instantaneous_velocity(samples, index);
            let sigma = tremor_sigma_px(params, velocity);
            point = jitter_path_point(point, sigma, &mut rng);
        }

        result.push(TimedPathPoint {
            elapsed_ms: sample.elapsed_ms + pause_offset_ms,
            arclen: sample.arclen,
            point,
        });

        if !is_endpoint && should_happen(params.micro_pause_prob, &mut rng) {
            let pause_ms = micro_pause_ms(params.micro_pause_ms_range, &mut rng);
            pause_offset_ms += pause_ms;
            result.push(TimedPathPoint {
                elapsed_ms: sample.elapsed_ms + pause_offset_ms,
                arclen: sample.arclen,
                point,
            });
        }
    }

    apply_overshoot(params, samples, &mut result, &mut rng);
    Ok(result)
}

fn validate_samples(samples: &[TimedPathPoint]) -> HumanizeResult<()> {
    if samples.len() < 2 {
        return Err(HumanizeError::NotEnoughSamples {
            samples: samples.len(),
        });
    }

    for (index, sample) in samples.iter().enumerate() {
        if !sample.elapsed_ms.is_finite()
            || !sample.arclen.is_finite()
            || !sample.point.x.is_finite()
            || !sample.point.y.is_finite()
        {
            return Err(HumanizeError::NonFiniteSample { index });
        }
        if index > 0 && sample.elapsed_ms + EPSILON < samples[index - 1].elapsed_ms {
            return Err(HumanizeError::NonMonotonicTimestamp { index });
        }
    }
    Ok(())
}

fn validate_params(params: HumanizeParams) -> HumanizeResult<()> {
    validate_non_negative("tremor_base_stddev_px", params.tremor_base_stddev_px)?;
    validate_non_negative("tremor_velocity_scale", params.tremor_velocity_scale)?;
    validate_probability("overshoot_prob", params.overshoot_prob)?;
    validate_probability("micro_pause_prob", params.micro_pause_prob)?;
    validate_overshoot_factor("overshoot_factor_range.0", params.overshoot_factor_range.0)?;
    validate_overshoot_factor("overshoot_factor_range.1", params.overshoot_factor_range.1)?;
    if params.overshoot_factor_range.0 > params.overshoot_factor_range.1 {
        return Err(HumanizeError::InvalidOvershootRange {
            min: params.overshoot_factor_range.0,
            max: params.overshoot_factor_range.1,
        });
    }
    if params.micro_pause_ms_range.0 > params.micro_pause_ms_range.1 {
        return Err(HumanizeError::InvalidMicroPauseRange {
            min_ms: params.micro_pause_ms_range.0,
            max_ms: params.micro_pause_ms_range.1,
        });
    }
    Ok(())
}

fn validate_non_negative(field: &'static str, value: f32) -> HumanizeResult<()> {
    if value.is_finite() && value >= 0.0 {
        return Ok(());
    }
    Err(HumanizeError::InvalidNonNegative { field, value })
}

fn validate_probability(field: &'static str, value: f32) -> HumanizeResult<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        return Ok(());
    }
    Err(HumanizeError::InvalidProbability { field, value })
}

fn validate_overshoot_factor(field: &'static str, value: f32) -> HumanizeResult<()> {
    if value.is_finite() && value >= 1.0 {
        return Ok(());
    }
    Err(HumanizeError::InvalidOvershootFactor { field, value })
}

fn humanization_disabled(params: HumanizeParams) -> bool {
    params.tremor_base_stddev_px.abs() <= f32::EPSILON
        && params.overshoot_prob.abs() <= f32::EPSILON
        && params.micro_pause_prob.abs() <= f32::EPSILON
}

fn instantaneous_velocity(samples: &[TimedPathPoint], index: usize) -> f64 {
    let prev = samples[index.saturating_sub(1)];
    let next = samples[(index + 1).min(samples.len() - 1)];
    let dt = (next.elapsed_ms - prev.elapsed_ms).abs();
    if dt <= EPSILON {
        0.0
    } else {
        (next.arclen - prev.arclen).abs() / dt
    }
}

fn tremor_sigma_px(params: HumanizeParams, velocity_px_per_ms: f64) -> f64 {
    let base = f64::from(params.tremor_base_stddev_px);
    let scale = f64::from(params.tremor_velocity_scale);
    base * (1.0 + scale / (1.0 + velocity_px_per_ms.max(0.0)))
}

fn jitter_path_point(point: PathPoint, sigma: f64, rng: &mut DeterministicRng) -> PathPoint {
    if sigma <= 0.0 {
        return point;
    }
    PathPoint {
        x: point.x + gaussian(rng, sigma),
        y: point.y + gaussian(rng, sigma),
    }
}

fn should_happen(probability: f32, rng: &mut DeterministicRng) -> bool {
    probability > 0.0 && rng.next_unit() < f64::from(probability)
}

fn micro_pause_ms(range: (u32, u32), rng: &mut DeterministicRng) -> f64 {
    let width = range.1 - range.0;
    rng.next_unit()
        .mul_add(f64::from(width), f64::from(range.0))
}

fn apply_overshoot(
    params: HumanizeParams,
    original: &[TimedPathPoint],
    result: &mut Vec<TimedPathPoint>,
    rng: &mut DeterministicRng,
) {
    if !should_happen(params.overshoot_prob, rng) || result.len() < 2 {
        return;
    }

    let final_sample = result[result.len() - 1];
    let previous = final_distinct_point(result).unwrap_or(original[0].point);
    let direction = PathPoint {
        x: final_sample.point.x - previous.x,
        y: final_sample.point.y - previous.y,
    };
    let norm = (direction.x.mul_add(direction.x, direction.y * direction.y)).sqrt();
    if norm <= EPSILON {
        return;
    }

    let low = f64::from(params.overshoot_factor_range.0);
    let high = f64::from(params.overshoot_factor_range.1);
    let factor = rng.next_unit().mul_add(high - low, low);
    let distance = original.last().map_or(0.0, |sample| sample.arclen);
    let overshoot_distance = distance * (factor - 1.0);
    if overshoot_distance <= EPSILON {
        return;
    }

    let overshoot = PathPoint {
        x: (direction.x / norm).mul_add(overshoot_distance, final_sample.point.x),
        y: (direction.y / norm).mul_add(overshoot_distance, final_sample.point.y),
    };
    let correction_ms = DEFAULT_CORRECTION_MS
        + micro_pause_ms(params.micro_pause_ms_range, rng).min(DEFAULT_CORRECTION_MS);

    let last_index = result.len() - 1;
    result[last_index] = TimedPathPoint {
        point: overshoot,
        ..final_sample
    };
    result.push(TimedPathPoint {
        elapsed_ms: final_sample.elapsed_ms + correction_ms,
        arclen: final_sample.arclen,
        point: final_sample.point,
    });
}

fn final_distinct_point(samples: &[TimedPathPoint]) -> Option<PathPoint> {
    let final_point = samples.last()?.point;
    samples
        .iter()
        .rev()
        .skip(1)
        .find(|sample| sample.point.distance_to(final_point) > EPSILON)
        .map(|sample| sample.point)
}

fn gaussian(rng: &mut DeterministicRng, stddev: f64) -> f64 {
    if stddev <= 0.0 {
        return 0.0;
    }
    let u1 = rng.next_open_unit();
    let u2 = rng.next_open_unit();
    let z0 = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
    z0 * stddev
}

fn effective_seed(params: HumanizeParams, samples: &[TimedPathPoint]) -> u64 {
    if let Some(seed) = params.seed {
        return seed;
    }

    let mut seed = 0x243f_6a88_85a3_08d3;
    mix_u64(&mut seed, u64::from(params.tremor_base_stddev_px.to_bits()));
    mix_u64(&mut seed, u64::from(params.tremor_velocity_scale.to_bits()));
    mix_u64(&mut seed, u64::from(params.overshoot_prob.to_bits()));
    mix_u64(
        &mut seed,
        u64::from(params.overshoot_factor_range.0.to_bits()),
    );
    mix_u64(
        &mut seed,
        u64::from(params.overshoot_factor_range.1.to_bits()),
    );
    mix_u64(&mut seed, u64::from(params.micro_pause_prob.to_bits()));
    mix_u64(&mut seed, u64::from(params.micro_pause_ms_range.0));
    mix_u64(&mut seed, u64::from(params.micro_pause_ms_range.1));
    for sample in samples {
        mix_u64(&mut seed, sample.elapsed_ms.to_bits());
        mix_u64(&mut seed, sample.arclen.to_bits());
        mix_u64(&mut seed, sample.point.x.to_bits());
        mix_u64(&mut seed, sample.point.y.to_bits());
    }
    seed
}

const fn mix_u64(seed: &mut u64, value: u64) {
    *seed ^= value
        .wrapping_add(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(*seed << 6)
        .wrapping_add(*seed >> 2);
}

#[derive(Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn next_unit(&mut self) -> f64 {
        f64::from(self.next_u32()) / (f64::from(u32::MAX) + 1.0)
    }

    fn next_open_unit(&mut self) -> f64 {
        (f64::from(self.next_u32()) + 1.0) / (f64::from(u32::MAX) + 2.0)
    }

    fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() >> 32).unwrap_or(0)
    }
}
