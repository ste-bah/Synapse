use std::{
    env,
    error::Error,
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

use criterion::Criterion;
use serde_json::json;
use synapse_action::{ActionBackend, EmitState, HardwareBackend};
use synapse_core::{Action, Backend, Key, KeyCode};
use synapse_hid_host::{HidGateway, connect_auto};

const BENCH_NAME: &str = "action_hardware_press";
const HARDWARE_ITERATIONS: usize = 1_000;
const TARGET_P99_NS: u128 = 5_000_000;
const HOLD_MS: u32 = 1;
const KEY_A_HID_USAGE: u8 = 0x04;
const PORT_ENV: &str = "SYNAPSE_HID_BENCH_PORT";

fn main() -> Result<(), Box<dyn Error>> {
    let Some(gateway) = connect_or_ci_skip()? else {
        return Ok(());
    };

    {
        let mut criterion = Criterion::default()
            .warm_up_time(Duration::from_millis(100))
            .measurement_time(Duration::from_secs(1))
            .sample_size(10)
            .configure_from_args();
        bench_action_hardware_press(&mut criterion, gateway);
        criterion.final_summary();
    }

    let Some(gateway) = connect_or_ci_skip()? else {
        return Ok(());
    };
    let mut harness = HardwarePressHarness::new(gateway);
    let report = measure_hardware_press(&mut harness)?;
    let baseline_path = write_baseline(&report)?;
    report.print(&baseline_path);

    assert!(
        report.pass,
        "{BENCH_NAME} p99 {} ns exceeded {TARGET_P99_NS} ns or state cleanup failed",
        report.p99_ns
    );

    Ok(())
}

fn bench_action_hardware_press(criterion: &mut Criterion, gateway: HidGateway) {
    let mut harness = HardwarePressHarness::new(gateway);

    criterion.bench_function(BENCH_NAME, |bencher| {
        bencher.iter_custom(|iterations| {
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                let elapsed = harness
                    .press_once()
                    .unwrap_or_else(|error| panic!("{BENCH_NAME} iteration failed: {error}"));
                black_box(elapsed);
                total = total.saturating_add(elapsed);
            }
            total
        });
    });

    harness
        .release_all()
        .unwrap_or_else(|error| panic!("{BENCH_NAME} release_all failed: {error}"));
}

fn measure_hardware_press(
    harness: &mut HardwarePressHarness,
) -> Result<BenchReport, Box<dyn Error>> {
    let before = harness.snapshot_summary();
    let mut elapsed_ns = Vec::with_capacity(HARDWARE_ITERATIONS);
    for _ in 0..HARDWARE_ITERATIONS {
        let elapsed = harness.press_once()?;
        elapsed_ns.push(elapsed.as_nanos());
    }
    elapsed_ns.sort_unstable();
    harness.release_all()?;
    let after = harness.snapshot_summary();

    let p99_ns = percentile(&elapsed_ns, 99);
    let pass = p99_ns <= TARGET_P99_NS && after == SnapshotSummary::empty();

    Ok(BenchReport {
        before,
        after,
        iterations: HARDWARE_ITERATIONS,
        p50_ns: percentile(&elapsed_ns, 50),
        p99_ns,
        max_ns: elapsed_ns.last().copied().unwrap_or(0),
        pass,
    })
}

struct HardwarePressHarness {
    backend: HardwareBackend,
    state: EmitState,
    action: Action,
}

impl HardwarePressHarness {
    fn new(gateway: HidGateway) -> Self {
        Self {
            backend: HardwareBackend::new(gateway),
            state: EmitState::new(),
            action: Action::KeyPress {
                key: key_a(),
                hold_ms: HOLD_MS,
                backend: Backend::Hardware,
            },
        }
    }

    fn press_once(&mut self) -> Result<Duration, Box<dyn Error>> {
        let started = Instant::now();
        self.backend.execute(&self.action, &mut self.state)?;
        Ok(started.elapsed())
    }

    fn release_all(&mut self) -> Result<(), Box<dyn Error>> {
        self.backend
            .execute(&Action::ReleaseAll, &mut self.state)
            .map_err(Into::into)
    }

    fn snapshot_summary(&self) -> SnapshotSummary {
        let snapshot = self.state.snapshot();
        SnapshotSummary {
            held_keys: snapshot.held_keys.len(),
            held_buttons: snapshot.held_buttons.len(),
            pad_count: snapshot.pad_state.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotSummary {
    held_keys: usize,
    held_buttons: usize,
    pad_count: usize,
}

impl SnapshotSummary {
    const fn empty() -> Self {
        Self {
            held_keys: 0,
            held_buttons: 0,
            pad_count: 0,
        }
    }
}

struct BenchReport {
    before: SnapshotSummary,
    after: SnapshotSummary,
    iterations: usize,
    p50_ns: u128,
    p99_ns: u128,
    max_ns: u128,
    pass: bool,
}

impl BenchReport {
    fn print(&self, baseline_path: &std::path::Path) {
        println!(
            "readback={BENCH_NAME} before={:?} after={:?} iterations={} p50_ns={} p99_ns={} max_ns={} target_p99_ns={} baseline_path={} result_value={}",
            self.before,
            self.after,
            self.iterations,
            self.p50_ns,
            self.p99_ns,
            self.max_ns,
            TARGET_P99_NS,
            baseline_path.display(),
            if self.pass { "pass" } else { "fail" }
        );
    }
}

fn connect_or_ci_skip() -> Result<Option<HidGateway>, Box<dyn Error>> {
    match connect_gateway() {
        Ok(gateway) => Ok(Some(gateway)),
        Err(error) if ci_enabled() => {
            println!("readback={BENCH_NAME} skipped_on_ci=true reason={error}");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn connect_gateway() -> Result<HidGateway, Box<dyn Error>> {
    if let Some(port) = env::var_os(PORT_ENV) {
        let port = port.to_string_lossy().into_owned();
        Ok(HidGateway::connect(port)?)
    } else {
        Ok(connect_auto()?)
    }
}

fn write_baseline(report: &BenchReport) -> Result<PathBuf, Box<dyn Error>> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    let dir = PathBuf::from(local_app_data)
        .join("synapse")
        .join("benchmarks")
        .join("baselines");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{BENCH_NAME}.json"));
    let body = serde_json::to_string_pretty(&json!({
        "bench": BENCH_NAME,
        "iterations": report.iterations,
        "target_p99_ns": TARGET_P99_NS,
        "p50_ns": report.p50_ns,
        "p99_ns": report.p99_ns,
        "max_ns": report.max_ns,
        "before": {
            "held_keys": report.before.held_keys,
            "held_buttons": report.before.held_buttons,
            "pad_count": report.before.pad_count,
        },
        "after": {
            "held_keys": report.after.held_keys,
            "held_buttons": report.after.held_buttons,
            "pad_count": report.after.pad_count,
        },
        "pass": report.pass,
    }))?;
    std::fs::write(&path, format!("{body}\n"))?;
    Ok(path)
}

fn percentile(values: &[u128], percentile: usize) -> u128 {
    if values.is_empty() {
        return 0;
    }
    let index = (values.len().saturating_sub(1) * percentile) / 100;
    values[index]
}

const fn key_a() -> Key {
    Key {
        code: KeyCode::HidCode {
            value: KEY_A_HID_USAGE,
        },
        use_scancode: false,
    }
}

fn ci_enabled() -> bool {
    env::var_os("CI").is_some()
}
