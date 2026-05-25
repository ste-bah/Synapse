use synapse_core::DirectionEstimate;

use crate::AudioWindow;

const MIN_DIRECTION_RMS: f32 = 0.000_1;
const MAX_ITD_SECONDS: f32 = 0.001;
const CENTER_CONFIDENCE: f32 = 0.35;
const AMBIENT_CONFIDENCE: f32 = 0.12;

#[must_use]
pub fn estimate_direction(window: &AudioWindow) -> DirectionEstimate {
    let channels = usize::from(window.format.channels);
    if channels < 2 || window.frames == 0 || window.samples.len() < channels {
        return undefined();
    }

    let (left, right) = stereo_channels(window, channels);
    if left.is_empty() {
        return undefined();
    }

    let left_rms = rms(&left);
    let right_rms = rms(&right);
    let combined_rms = (left_rms.mul_add(left_rms, right_rms * right_rms) * 0.5).sqrt();
    if combined_rms <= MIN_DIRECTION_RMS {
        return undefined();
    }

    let energy_deg = energy_azimuth(left_rms, right_rms);
    let max_lag = max_lag_samples(window.format.sample_rate_hz);
    let (lag_samples, lag_score) = best_lag(&left, &right, max_lag);
    let lag_deg = lag_azimuth(lag_samples, max_lag);
    let azimuth_deg = blend_azimuth(energy_deg, lag_deg, lag_score);

    DirectionEstimate {
        azimuth_deg,
        confidence: confidence(energy_deg, lag_deg, lag_score),
    }
}

fn stereo_channels(window: &AudioWindow, channels: usize) -> (Vec<f32>, Vec<f32>) {
    let mut left = Vec::with_capacity(window.frames);
    let mut right = Vec::with_capacity(window.frames);
    for frame in window.samples.chunks_exact(channels).take(window.frames) {
        left.push(frame[0].clamp(-1.0, 1.0));
        right.push(frame[1].clamp(-1.0, 1.0));
    }
    (left, right)
}

#[allow(clippy::cast_precision_loss)]
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples.iter().map(|sample| sample * sample).sum::<f32>();
    (sum / samples.len() as f32).sqrt()
}

fn energy_azimuth(left_rms: f32, right_rms: f32) -> f32 {
    if left_rms <= MIN_DIRECTION_RMS && right_rms <= MIN_DIRECTION_RMS {
        return 0.0;
    }
    let pan = (4.0 / std::f32::consts::PI).mul_add(right_rms.atan2(left_rms), -1.0);
    (pan * 90.0).clamp(-90.0, 90.0)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn max_lag_samples(sample_rate_hz: u32) -> i32 {
    ((sample_rate_hz.max(1) as f32 * MAX_ITD_SECONDS).round() as i32).max(1)
}

fn best_lag(left: &[f32], right: &[f32], max_lag: i32) -> (i32, f32) {
    let mut best_lag = 0;
    let mut best_score = f32::NEG_INFINITY;
    for lag in -max_lag..=max_lag {
        let score = normalized_correlation(left, right, lag);
        if score > best_score {
            best_lag = lag;
            best_score = score;
        }
    }
    (best_lag, best_score.max(0.0))
}

fn normalized_correlation(left: &[f32], right: &[f32], lag: i32) -> f32 {
    let (left_start, right_start, len) = lag_ranges(left.len().min(right.len()), lag);
    if len == 0 {
        return 0.0;
    }

    let mut dot = 0.0;
    let mut left_energy = 0.0;
    let mut right_energy = 0.0;
    for idx in 0..len {
        let l = left[left_start + idx];
        let r = right[right_start + idx];
        dot += l * r;
        left_energy += l * l;
        right_energy += r * r;
    }
    let denom = (left_energy * right_energy).sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

const fn lag_ranges(len: usize, lag: i32) -> (usize, usize, usize) {
    let lag_abs = lag.unsigned_abs() as usize;
    if lag_abs >= len {
        return (0, 0, 0);
    }
    if lag >= 0 {
        (0, lag_abs, len - lag_abs)
    } else {
        (lag_abs, 0, len - lag_abs)
    }
}

#[allow(clippy::cast_precision_loss)]
fn lag_azimuth(lag_samples: i32, max_lag: i32) -> f32 {
    let normalized = lag_samples as f32 / max_lag.max(1) as f32;
    (-normalized * 90.0).clamp(-90.0, 90.0)
}

fn blend_azimuth(energy_deg: f32, lag_deg: f32, lag_score: f32) -> f32 {
    if lag_score < 0.2 || lag_deg.abs() < 5.0 {
        return energy_deg;
    }
    energy_deg.mul_add(0.75, lag_deg * 0.25).clamp(-90.0, 90.0)
}

fn confidence(energy_deg: f32, lag_deg: f32, lag_score: f32) -> f32 {
    let energy_strength = energy_deg.abs() / 90.0;
    let lag_strength = (lag_deg.abs() / 90.0) * lag_score.clamp(0.0, 1.0);
    let strength = energy_strength.max(lag_strength);
    if strength < 0.05 && lag_score < 0.2 {
        return AMBIENT_CONFIDENCE;
    }
    if strength < 0.05 {
        return CENTER_CONFIDENCE * lag_score.clamp(0.0, 1.0);
    }
    0.10_f32
        .mul_add(lag_score.clamp(0.0, 1.0), 0.65_f32.mul_add(strength, 0.25))
        .clamp(0.0, 1.0)
}

const fn undefined() -> DirectionEstimate {
    DirectionEstimate {
        azimuth_deg: 0.0,
        confidence: 0.0,
    }
}
