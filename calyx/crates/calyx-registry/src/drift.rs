use calyx_core::{Input, LensId};
use serde::{Deserialize, Serialize};

pub const CALYX_LENS_RUNTIME_DRIFT: &str = "CALYX_LENS_RUNTIME_DRIFT";
pub const PROCESS_RUNTIME_GOLDEN_TOLERANCE: f32 = 1.0e-6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeGolden {
    pub lens_id: LensId,
    pub runtime_version: String,
    pub probe: Input,
    pub golden_output: Vec<f32>,
    pub tolerance: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftDecision {
    Reuse {
        lens_id: LensId,
        max_abs_delta: f32,
    },
    Drifted {
        old_lens_id: LensId,
        new_lens_id: LensId,
        max_abs_delta: f32,
        signal: String,
    },
}

impl RuntimeGolden {
    pub fn evaluate(&self, observed: &[f32]) -> DriftDecision {
        let max_abs_delta = max_abs_delta(&self.golden_output, observed);
        if self.tolerance.is_finite() && self.tolerance >= 0.0 && max_abs_delta <= self.tolerance {
            return DriftDecision::Reuse {
                lens_id: self.lens_id,
                max_abs_delta,
            };
        }
        // DRIFT: frozen numeric behavior changed beyond tolerance; this must
        // become a new LensId instead of silently reusing the old instrument id.
        let observed_bytes = observed
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let new_lens_id = LensId::from_parts(
            &format!("{}:{}", self.lens_id, self.runtime_version),
            self.lens_id.as_bytes(),
            self.runtime_version.as_bytes(),
            &observed_bytes,
        );
        DriftDecision::Drifted {
            old_lens_id: self.lens_id,
            new_lens_id,
            max_abs_delta,
            signal: CALYX_LENS_RUNTIME_DRIFT.to_string(),
        }
    }
}

fn max_abs_delta(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.iter().chain(right).any(|value| !value.is_finite()) {
        return f32::INFINITY;
    }
    left.iter()
        .zip(right)
        .map(|(a, b)| (*a - *b).abs())
        .fold(0.0_f32, f32::max)
}
