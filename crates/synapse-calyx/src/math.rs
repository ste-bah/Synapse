use std::fmt;

#[cfg(feature = "calyx-cuda")]
use calyx_forge::CudaBackend;
use calyx_forge::{Backend, CUDA_COMPILED, CpuBackend, DeviceInfo, ForgeError};
use serde::Serialize;

use crate::{SynapseCalyxError, SynapseCalyxMathBackend, SynapseCalyxTuningConfig, invalid_config};

const MATH_BACKEND_REMEDIATION: &str = "inspect the SYNAPSE_CALYX_MATH_* structured events, health payload, CUDA driver state, and Calyx Forge error; use math_backend=\"cpu\" only when intentionally forcing CPU";
const PROBE_TOLERANCE: f32 = 0.0001;
const PROBE_DIM: usize = 3;
const PROBE_QUERY: [f32; PROBE_DIM] = [1.0, 0.0, 0.0];
const PROBE_CANDIDATES: [f32; 9] = [
    1.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, //
    2.0, 0.0, 0.0,
];
const EXPECTED_DOT: [f32; 3] = [1.0, 0.0, 2.0];
const EXPECTED_COSINE: [f32; 3] = [1.0, 0.0, 1.0];
const EXPECTED_L2_SQUARED: [f32; 3] = [0.0, 2.0, 1.0];
const PROBE_TOPK_SCORES: [f32; 4] = [0.25, 1.5, -0.5, 1.5];
const EXPECTED_TOPK: [(usize, f32); 3] = [(1, 1.5), (3, 1.5), (0, 0.25)];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SynapseCalyxMathBackendStatus {
    pub requested_backend: SynapseCalyxMathBackend,
    pub selected_backend: String,
    pub cuda_compiled: bool,
    pub device_name: String,
    pub device_vram_mib: Option<u64>,
    pub device_avx512: bool,
    pub cpu_avx512_available: bool,
    pub cpu_simd_path: String,
    pub vram_budget_bytes: u64,
    pub fallback_code: Option<String>,
    pub fallback_source_code: Option<String>,
    pub fallback_error: Option<String>,
    pub probe: SynapseCalyxMathProbeReport,
}

impl SynapseCalyxMathBackendStatus {
    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "requested_backend={} selected_backend={} cuda_compiled={} device_name={} device_vram_mib={:?} device_avx512={} cpu_avx512_available={} cpu_simd_path={} vram_budget_bytes={} fallback_code={} fallback_source_code={} probe_status={} probe_detail={}",
            self.requested_backend.as_str(),
            self.selected_backend,
            self.cuda_compiled,
            self.device_name,
            self.device_vram_mib,
            self.device_avx512,
            self.cpu_avx512_available,
            self.cpu_simd_path,
            self.vram_budget_bytes,
            self.fallback_code.as_deref().unwrap_or("none"),
            self.fallback_source_code.as_deref().unwrap_or("none"),
            self.probe.status,
            self.probe.detail,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SynapseCalyxMathProbeReport {
    pub status: String,
    pub detail: String,
    pub tolerance: f32,
    pub dot: Vec<f32>,
    pub cosine: Vec<f32>,
    pub l2_squared: Vec<f32>,
    pub topk: Vec<SynapseCalyxMathProbeTopKEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SynapseCalyxMathProbeTopKEntry {
    pub index: usize,
    pub score: f32,
}

pub struct SynapseCalyxMathRuntime {
    backend: Box<dyn Backend>,
    status: SynapseCalyxMathBackendStatus,
}

impl fmt::Debug for SynapseCalyxMathRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SynapseCalyxMathRuntime")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl SynapseCalyxMathRuntime {
    #[must_use]
    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    #[must_use]
    pub const fn status(&self) -> &SynapseCalyxMathBackendStatus {
        &self.status
    }
}

/// Builds the single Calyx Forge math backend for this Synapse process.
///
/// # Errors
///
/// Returns a structured error when config is invalid or when both the selected
/// runtime path and the CPU safety path fail the startup probe. In `auto`, CUDA
/// init/probe failures are surfaced in the returned status and CPU is selected
/// so a missing or unusable GPU does not take down the daemon.
pub fn math_backend(
    config: &SynapseCalyxTuningConfig,
) -> Result<SynapseCalyxMathRuntime, SynapseCalyxError> {
    config.validate()?;
    let cpu_reference = CpuBackend::new();
    let cpu_readback = CpuReadback::from_backend(&cpu_reference);
    match config.math_backend {
        SynapseCalyxMathBackend::Cpu => {
            runtime_from_backend(config, cpu_reference, cpu_readback, None)
        }
        SynapseCalyxMathBackend::Auto => {
            match cuda_runtime_candidate(config, cpu_readback.clone()) {
                Ok(runtime) => Ok(runtime),
                Err(error) => {
                    tracing::warn!(
                        code = error.code,
                        source_code = error.source_code.unwrap_or("none"),
                        error = %error,
                        "Calyx CUDA backend unavailable in auto mode; selecting CPU backend"
                    );
                    runtime_from_backend(config, cpu_reference, cpu_readback, Some(error))
                }
            }
        }
        SynapseCalyxMathBackend::Cuda => Err(invalid_config(
            "math_backend = \"cuda\" is not a supported Synapse override; use \"auto\" to prefer CUDA with CPU fallback or \"cpu\" to force CPU",
        )),
    }
}

#[derive(Clone, Debug)]
struct CpuReadback {
    avx512_available: bool,
    simd_path: String,
}

impl CpuReadback {
    fn from_backend(backend: &CpuBackend) -> Self {
        Self {
            avx512_available: backend.avx512_available(),
            simd_path: backend.simd_path().to_owned(),
        }
    }
}

#[cfg(feature = "calyx-cuda")]
fn cuda_runtime_candidate(
    config: &SynapseCalyxTuningConfig,
    cpu_readback: CpuReadback,
) -> Result<SynapseCalyxMathRuntime, SynapseCalyxError> {
    let backend = CudaBackend::new().map_err(|error| {
        forge_error(
            "SYNAPSE_CALYX_MATH_CUDA_UNAVAILABLE",
            "initialize Calyx CUDA backend",
            &error,
        )
    })?;
    runtime_from_backend(config, backend, cpu_readback, None)
}

#[cfg(not(feature = "calyx-cuda"))]
fn cuda_runtime_candidate(
    _config: &SynapseCalyxTuningConfig,
    _cpu_readback: CpuReadback,
) -> Result<SynapseCalyxMathRuntime, SynapseCalyxError> {
    Err(SynapseCalyxError::new(
        "SYNAPSE_CALYX_MATH_CUDA_NOT_COMPILED",
        "synapse-calyx was built without the calyx-cuda feature, so auto mode cannot initialize CUDA",
        MATH_BACKEND_REMEDIATION,
    ))
}

fn runtime_from_backend<B>(
    config: &SynapseCalyxTuningConfig,
    backend: B,
    cpu_readback: CpuReadback,
    fallback: Option<SynapseCalyxError>,
) -> Result<SynapseCalyxMathRuntime, SynapseCalyxError>
where
    B: Backend + 'static,
{
    let device_info = backend.device_info();
    let probe = run_startup_probe(&backend)?;
    let status = status_from_device_info(config, &device_info, cpu_readback, fallback, probe);
    tracing::info!(
        code = "SYNAPSE_CALYX_MATH_BACKEND_SELECTED",
        requested_backend = status.requested_backend.as_str(),
        selected_backend = status.selected_backend.as_str(),
        cuda_compiled = status.cuda_compiled,
        device_name = status.device_name.as_str(),
        device_vram_mib = status.device_vram_mib,
        device_avx512 = status.device_avx512,
        cpu_avx512_available = status.cpu_avx512_available,
        cpu_simd_path = status.cpu_simd_path.as_str(),
        vram_budget_bytes = status.vram_budget_bytes,
        fallback_code = status.fallback_code.as_deref().unwrap_or("none"),
        fallback_source_code = status.fallback_source_code.as_deref().unwrap_or("none"),
        probe_status = status.probe.status.as_str(),
        probe_dot = ?status.probe.dot,
        probe_cosine = ?status.probe.cosine,
        probe_l2_squared = ?status.probe.l2_squared,
        probe_topk = ?status.probe.topk,
        "selected Calyx Forge math backend"
    );
    Ok(SynapseCalyxMathRuntime {
        backend: Box::new(backend),
        status,
    })
}

fn status_from_device_info(
    config: &SynapseCalyxTuningConfig,
    info: &DeviceInfo,
    cpu_readback: CpuReadback,
    fallback: Option<SynapseCalyxError>,
    probe: SynapseCalyxMathProbeReport,
) -> SynapseCalyxMathBackendStatus {
    let (fallback_code, fallback_source_code, fallback_error) =
        fallback.map_or((None, None, None), |error| {
            (
                Some(error.code.to_owned()),
                error.source_code.map(str::to_owned),
                Some(error.to_string()),
            )
        });
    SynapseCalyxMathBackendStatus {
        requested_backend: config.math_backend,
        selected_backend: info.kind.to_string(),
        cuda_compiled: CUDA_COMPILED,
        device_name: info.name.clone(),
        device_vram_mib: info.vram_mib,
        device_avx512: info.avx512,
        cpu_avx512_available: cpu_readback.avx512_available,
        cpu_simd_path: cpu_readback.simd_path,
        vram_budget_bytes: config.vram_budget_bytes,
        fallback_code,
        fallback_source_code,
        fallback_error,
        probe,
    }
}

fn run_startup_probe(
    backend: &dyn Backend,
) -> Result<SynapseCalyxMathProbeReport, SynapseCalyxError> {
    let mut dot = vec![0.0; EXPECTED_DOT.len()];
    backend
        .dot(&PROBE_QUERY, &PROBE_CANDIDATES, PROBE_DIM, &mut dot)
        .map_err(|error| probe_error("dot", &error))?;
    assert_close_vec("dot", &dot, &EXPECTED_DOT)?;

    let mut cosine = vec![0.0; EXPECTED_COSINE.len()];
    backend
        .cosine(&PROBE_QUERY, &PROBE_CANDIDATES, PROBE_DIM, &mut cosine)
        .map_err(|error| probe_error("cosine", &error))?;
    assert_close_vec("cosine", &cosine, &EXPECTED_COSINE)?;

    let mut l2_squared = vec![0.0; EXPECTED_L2_SQUARED.len()];
    backend
        .l2(&PROBE_QUERY, &PROBE_CANDIDATES, PROBE_DIM, &mut l2_squared)
        .map_err(|error| probe_error("l2", &error))?;
    assert_close_vec("l2", &l2_squared, &EXPECTED_L2_SQUARED)?;

    let topk = backend
        .topk(&PROBE_TOPK_SCORES, EXPECTED_TOPK.len())
        .map_err(|error| probe_error("topk", &error))?;
    assert_topk(&topk)?;

    Ok(SynapseCalyxMathProbeReport {
        status: "ok".to_owned(),
        detail: format!(
            "fixed vectors matched expected dot={EXPECTED_DOT:?} cosine={EXPECTED_COSINE:?} l2_squared={EXPECTED_L2_SQUARED:?} topk={EXPECTED_TOPK:?}"
        ),
        tolerance: PROBE_TOLERANCE,
        dot,
        cosine,
        l2_squared,
        topk: topk
            .into_iter()
            .map(|(index, score)| SynapseCalyxMathProbeTopKEntry { index, score })
            .collect(),
    })
}

fn assert_close_vec(
    label: &'static str,
    actual: &[f32],
    expected: &[f32],
) -> Result<(), SynapseCalyxError> {
    if actual.len() != expected.len() {
        return Err(probe_mismatch(format!(
            "{label} length mismatch actual_len={} expected_len={}",
            actual.len(),
            expected.len()
        )));
    }
    for (index, (actual_value, expected_value)) in actual.iter().zip(expected).enumerate() {
        if (*actual_value - *expected_value).abs() > PROBE_TOLERANCE {
            return Err(probe_mismatch(format!(
                "{label}[{index}] actual={actual_value} expected={expected_value} tolerance={PROBE_TOLERANCE}; actual={actual:?} expected={expected:?}"
            )));
        }
    }
    Ok(())
}

fn assert_topk(actual: &[(usize, f32)]) -> Result<(), SynapseCalyxError> {
    if actual.len() != EXPECTED_TOPK.len() {
        return Err(probe_mismatch(format!(
            "topk length mismatch actual_len={} expected_len={}",
            actual.len(),
            EXPECTED_TOPK.len()
        )));
    }
    for (rank, ((actual_index, actual_score), (expected_index, expected_score))) in
        actual.iter().zip(EXPECTED_TOPK).enumerate()
    {
        if *actual_index != expected_index
            || (*actual_score - expected_score).abs() > PROBE_TOLERANCE
        {
            return Err(probe_mismatch(format!(
                "topk[{rank}] actual=({actual_index}, {actual_score}) expected=({expected_index}, {expected_score}) tolerance={PROBE_TOLERANCE}; actual={actual:?} expected={EXPECTED_TOPK:?}"
            )));
        }
    }
    Ok(())
}

fn probe_error(op: &'static str, error: &ForgeError) -> SynapseCalyxError {
    forge_error(
        "SYNAPSE_CALYX_MATH_PROBE_FAILED",
        format!("run Calyx math startup probe op={op}"),
        error,
    )
}

fn probe_mismatch(message: String) -> SynapseCalyxError {
    SynapseCalyxError::new(
        "SYNAPSE_CALYX_MATH_PROBE_MISMATCH",
        message,
        MATH_BACKEND_REMEDIATION,
    )
}

fn forge_error(
    code: &'static str,
    action: impl AsRef<str>,
    error: &ForgeError,
) -> SynapseCalyxError {
    SynapseCalyxError {
        code,
        message: format!("{}: {error}", action.as_ref()),
        remediation: MATH_BACKEND_REMEDIATION,
        source_code: Some(error.code()),
    }
}
