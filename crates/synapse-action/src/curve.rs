use synapse_core::{AimCurve, AimNaturalParams, Point};

const MIN_SAMPLES: u32 = 8;

/// Samples a cursor path for the requested aim curve.
///
/// The returned path always includes the current start point as the first
/// anchor and the requested end point as the final anchor. `Instant` therefore
/// has one jump segment: `[start, end]`.
#[must_use]
pub fn sample_curve(
    curve: &AimCurve,
    start: Point,
    end: Point,
    duration_ms: u32,
    seed: Option<u64>,
) -> Vec<Point> {
    match curve {
        AimCurve::Instant => vec![start, end],
        AimCurve::Linear => sample_standard(start, end, duration_ms, linear_axis),
        AimCurve::EaseInOut => sample_standard(start, end, duration_ms, smoothstep_axis),
        AimCurve::Bezier { p1, p2 } => sample_standard(start, end, duration_ms, |t| {
            (
                cubic_bezier_axis(t, f64::from(p1.0), f64::from(p2.0)),
                cubic_bezier_axis(t, f64::from(p1.1), f64::from(p2.1)),
            )
        }),
        AimCurve::Natural { params } => sample_natural(*params, start, end, duration_ms, seed),
    }
}

fn sample_count(duration_ms: u32) -> u32 {
    MIN_SAMPLES.max(duration_ms / 4)
}

fn sample_capacity(count: u32) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

fn sample_standard(
    start: Point,
    end: Point,
    duration_ms: u32,
    axis: impl Fn(f64) -> (f64, f64),
) -> Vec<Point> {
    let count = sample_count(duration_ms);
    let mut samples = Vec::with_capacity(sample_capacity(count));
    let last_index = count.saturating_sub(1);

    for index in 0..count {
        let t = normalized_index(index, last_index);
        let (x_axis, y_axis) = axis(t);
        samples.push(point_at(start, end, x_axis, y_axis));
    }

    force_endpoints(&mut samples, start, end);
    samples
}

fn sample_natural(
    params: AimNaturalParams,
    start: Point,
    end: Point,
    duration_ms: u32,
    seed: Option<u64>,
) -> Vec<Point> {
    let count = sample_count(duration_ms);
    let mut rng = DeterministicRng::new(effective_seed(
        seed,
        params.seed,
        start,
        end,
        duration_ms,
        params,
    ));
    let mut samples = Vec::with_capacity(sample_capacity(count));

    let p1 = (
        0.25 + gaussian(
            &mut rng,
            f64::from(sanitized_non_negative(params.control_point_jitter)),
        ),
        0.10 + gaussian(
            &mut rng,
            f64::from(sanitized_non_negative(params.control_point_jitter)),
        ),
    );
    let p2 = (
        0.75 + gaussian(
            &mut rng,
            f64::from(sanitized_non_negative(params.control_point_jitter)),
        ),
        0.90 + gaussian(
            &mut rng,
            f64::from(sanitized_non_negative(params.control_point_jitter)),
        ),
    );

    let overshoot = should_overshoot(params.overshoot_prob, &mut rng);
    let target = if overshoot {
        overshoot_target(start, end, params.overshoot_factor_range, &mut rng)
    } else {
        end
    };

    let micro_steps = u32::from(params.micro_correct_steps).min(count.saturating_sub(2));
    let main_count = count.saturating_sub(micro_steps).max(2);
    let main_last = main_count.saturating_sub(1);
    let timing_scale = if duration_ms == 0 {
        0.0
    } else {
        f64::from(sanitized_non_negative(params.timing_stddev_ms)) / f64::from(duration_ms)
    };
    let tremor_stddev = sanitized_non_negative(params.tremor_stddev_px);

    for index in 0..main_count {
        let mut t = normalized_index(index, main_last);
        if index != 0 && index != main_last {
            t = (t + gaussian(&mut rng, timing_scale)).clamp(0.0, 1.0);
        }

        let x_axis = cubic_bezier_axis(t, p1.0, p2.0);
        let y_axis = cubic_bezier_axis(t, p1.1, p2.1);
        let mut point = point_at(start, target, x_axis, y_axis);

        if index != 0 && index != main_last && tremor_stddev > 0.0 {
            point = jitter_point(point, tremor_stddev, &mut rng);
        }

        samples.push(point);
    }

    if micro_steps > 0 {
        let settle_start = samples.last().copied().unwrap_or(start);
        for step in 1..=micro_steps {
            let t = f64::from(step) / f64::from(micro_steps);
            samples.push(point_at(settle_start, end, t, t));
        }
    }

    force_endpoints(&mut samples, start, end);
    samples
}

fn normalized_index(index: u32, last_index: u32) -> f64 {
    if last_index == 0 {
        1.0
    } else {
        f64::from(index) / f64::from(last_index)
    }
}

const fn linear_axis(t: f64) -> (f64, f64) {
    (t, t)
}

fn smoothstep_axis(t: f64) -> (f64, f64) {
    let t2 = t * t;
    let eased = (-2.0 * t).mul_add(t2, 3.0 * t2);
    (eased, eased)
}

fn cubic_bezier_axis(t: f64, p1: f64, p2: f64) -> f64 {
    let inverse = 1.0 - t;
    let t2 = t * t;
    let t3 = t2 * t;
    let a = 3.0 * inverse * inverse * t;
    let b = 3.0 * inverse * t2;
    a.mul_add(p1, b.mul_add(p2, t3))
}

fn point_at(start: Point, end: Point, x_axis: f64, y_axis: f64) -> Point {
    let dx = f64::from(end.x) - f64::from(start.x);
    let dy = f64::from(end.y) - f64::from(start.y);
    Point {
        x: clamp_i32(dx.mul_add(x_axis, f64::from(start.x))),
        y: clamp_i32(dy.mul_add(y_axis, f64::from(start.y))),
    }
}

fn jitter_point(point: Point, stddev: f32, rng: &mut DeterministicRng) -> Point {
    Point {
        x: clamp_i32(f64::from(point.x) + gaussian(rng, f64::from(stddev))),
        y: clamp_i32(f64::from(point.y) + gaussian(rng, f64::from(stddev))),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "pixel coordinates are rounded and clamped for #159/#160 FSV"
)]
fn clamp_i32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[allow(
    clippy::missing_const_for_fn,
    reason = "slice endpoint mutation is clearer as ordinary runtime code for #159"
)]
fn force_endpoints(samples: &mut [Point], start: Point, end: Point) {
    if let Some(first) = samples.first_mut() {
        *first = start;
    }
    if let Some(last) = samples.last_mut() {
        *last = end;
    }
}

#[allow(
    clippy::missing_const_for_fn,
    reason = "float sanitization is ordinary runtime input handling for #160"
)]
fn sanitized_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn should_overshoot(probability: f32, rng: &mut DeterministicRng) -> bool {
    probability.is_finite()
        && probability > 0.0
        && rng.next_unit() < f64::from(probability.min(1.0))
}

fn overshoot_target(
    start: Point,
    end: Point,
    factor_range: (f32, f32),
    rng: &mut DeterministicRng,
) -> Point {
    let low = if factor_range.0.is_finite() {
        factor_range.0
    } else {
        1.0
    };
    let high = if factor_range.1.is_finite() {
        factor_range.1
    } else {
        low
    };
    let min = low.min(high);
    let max = low.max(high);
    let factor = rng
        .next_unit()
        .mul_add(f64::from(max - min), f64::from(min));
    let dx = f64::from(end.x) - f64::from(start.x);
    let dy = f64::from(end.y) - f64::from(start.y);

    Point {
        x: clamp_i32(dx.mul_add(factor, f64::from(start.x))),
        y: clamp_i32(dy.mul_add(factor, f64::from(start.y))),
    }
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

fn effective_seed(
    override_seed: Option<u64>,
    params_seed: Option<u64>,
    start: Point,
    end: Point,
    duration_ms: u32,
    params: AimNaturalParams,
) -> u64 {
    if let Some(seed) = override_seed.or(params_seed) {
        return seed;
    }

    let mut seed = 0x9e37_79b9_7f4a_7c15;
    mix_i32(&mut seed, start.x);
    mix_i32(&mut seed, start.y);
    mix_i32(&mut seed, end.x);
    mix_i32(&mut seed, end.y);
    mix_u64(&mut seed, u64::from(duration_ms));
    mix_u64(&mut seed, u64::from(params.control_point_jitter.to_bits()));
    mix_u64(&mut seed, u64::from(params.tremor_stddev_px.to_bits()));
    mix_u64(&mut seed, u64::from(params.overshoot_prob.to_bits()));
    mix_u64(
        &mut seed,
        u64::from(params.overshoot_factor_range.0.to_bits()),
    );
    mix_u64(
        &mut seed,
        u64::from(params.overshoot_factor_range.1.to_bits()),
    );
    mix_u64(&mut seed, u64::from(params.micro_correct_steps));
    mix_u64(&mut seed, u64::from(params.timing_stddev_ms.to_bits()));
    seed
}

fn mix_i32(seed: &mut u64, value: i32) {
    mix_u64(seed, u64::from(value.cast_unsigned()));
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
