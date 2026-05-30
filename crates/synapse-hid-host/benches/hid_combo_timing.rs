use std::{
    env,
    error::Error,
    hint::black_box,
    time::{Duration, Instant},
};

use criterion::Criterion;
use synapse_hid_host::{
    HOST_COMMAND_MOUSE_MOVE_REL, HidGateway, HidTelemetrySnapshot, connect_auto,
};

const BENCH_NAME: &str = "hid_combo_timing";
const RUN_COUNT: usize = 1_000;
const STEP_COUNT: usize = 3;
const EXPECTED_COMMANDS: u32 = 3_000;
const STEP_INTERVAL_MS: u64 = 100;
const TARGET_DEVIATION_NS: u128 = 500_000;
const PORT_ENV: &str = "SYNAPSE_HID_BENCH_PORT";
const STEP_PAYLOADS: [[u8; 4]; STEP_COUNT] = [[1, 0, 0, 0], [255, 255, 0, 0], [1, 0, 0, 0]];

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
        bench_single_combo(&mut criterion, gateway);
        criterion.final_summary();
    }

    let Some(mut gateway) = connect_or_ci_skip()? else {
        return Ok(());
    };
    let report = measure_combo_timing(&mut gateway)?;
    report.print();
    assert!(
        report.pass,
        "{BENCH_NAME} max deviation {} ns exceeded {TARGET_DEVIATION_NS} ns or telemetry counters failed",
        report.max_deviation_ns
    );
    Ok(())
}

fn bench_single_combo(criterion: &mut Criterion, mut gateway: HidGateway) {
    criterion.bench_function(BENCH_NAME, |bencher| {
        bencher.iter_custom(|iterations| {
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                let started = Instant::now();
                send_scheduled_combo(&mut gateway)
                    .unwrap_or_else(|error| panic!("{BENCH_NAME} iteration failed: {error}"));
                let elapsed = started.elapsed();
                black_box(elapsed);
                total = total.saturating_add(elapsed);
            }
            total
        });
    });
}

fn measure_combo_timing(gateway: &mut HidGateway) -> Result<BenchReport, Box<dyn Error>> {
    let before = gateway.get_telemetry()?;
    let mut max_deviation_ns = 0_u128;
    let mut latest_intervals = [0_u128; STEP_COUNT - 1];

    for _ in 0..RUN_COUNT {
        let send_times = send_scheduled_combo(gateway)?;
        let intervals = [
            send_times[1].duration_since(send_times[0]).as_nanos(),
            send_times[2].duration_since(send_times[1]).as_nanos(),
        ];
        latest_intervals = intervals;
        for interval in intervals {
            max_deviation_ns = max_deviation_ns.max(deviation_ns(
                interval,
                u128::from(STEP_INTERVAL_MS) * 1_000_000,
            ));
        }
    }

    let after = gateway.get_telemetry()?;
    let command_delta = telemetry_command_delta_excluding_after_read(before, after);
    let dropped_delta = after.frames_dropped.saturating_sub(before.frames_dropped);
    let crc_delta = after.crc_errors.saturating_sub(before.crc_errors);
    let pass = max_deviation_ns <= TARGET_DEVIATION_NS
        && command_delta == EXPECTED_COMMANDS
        && dropped_delta == 0
        && crc_delta == 0;

    Ok(BenchReport {
        before,
        after,
        command_delta,
        expected_commands: EXPECTED_COMMANDS,
        dropped_delta,
        crc_delta,
        latest_intervals,
        max_deviation_ns,
        pass,
    })
}

fn send_scheduled_combo(gateway: &mut HidGateway) -> Result<[Instant; STEP_COUNT], Box<dyn Error>> {
    let start = Instant::now();
    let mut send_times = [start; STEP_COUNT];
    for (index, payload) in STEP_PAYLOADS.iter().enumerate() {
        let step_index = u64::try_from(index).map_err(|_| "step index overflow")?;
        let deadline = start + Duration::from_millis(STEP_INTERVAL_MS * step_index);
        wait_until(deadline);
        send_times[index] = Instant::now();
        gateway.send_command(HOST_COMMAND_MOUSE_MOVE_REL, payload)?;
    }
    Ok(send_times)
}

fn wait_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let remaining = deadline.duration_since(now);
        if remaining > Duration::from_millis(2) {
            std::thread::sleep(remaining.saturating_sub(Duration::from_millis(1)));
        } else {
            std::hint::spin_loop();
        }
    }
}

struct BenchReport {
    before: HidTelemetrySnapshot,
    after: HidTelemetrySnapshot,
    command_delta: u32,
    expected_commands: u32,
    dropped_delta: u32,
    crc_delta: u32,
    latest_intervals: [u128; STEP_COUNT - 1],
    max_deviation_ns: u128,
    pass: bool,
}

impl BenchReport {
    fn print(&self) {
        println!(
            "readback={BENCH_NAME} before={:?} after={:?} expected_commands={} command_delta={} dropped_delta={} crc_delta={} latest_intervals_ns={:?} target_deviation_ns={} max_deviation_ns={} result_value={}",
            self.before,
            self.after,
            self.expected_commands,
            self.command_delta,
            self.dropped_delta,
            self.crc_delta,
            self.latest_intervals,
            TARGET_DEVIATION_NS,
            self.max_deviation_ns,
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

const fn telemetry_command_delta_excluding_after_read(
    before: HidTelemetrySnapshot,
    after: HidTelemetrySnapshot,
) -> u32 {
    after
        .commands_executed
        .saturating_sub(before.commands_executed)
        .saturating_sub(1)
}

const fn deviation_ns(actual: u128, expected: u128) -> u128 {
    actual.abs_diff(expected)
}

fn ci_enabled() -> bool {
    env::var_os("CI").is_some()
}
