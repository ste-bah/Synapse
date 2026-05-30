use std::{
    env,
    error::Error,
    hint::black_box,
    time::{Duration, Instant},
};

use criterion::Criterion;
use synapse_hid_host::{
    HOST_COMMAND_MOUSE_MOVE_REL, HidGateway, HidTelemetrySnapshot, HostCommandRequest, connect_auto,
};

const BENCH_NAME: &str = "hid_high_volume";
const MOVE_COUNT: usize = 10_000;
const EXPECTED_COMMANDS: u32 = 10_000;
const TARGET_WALL_MS: u128 = 15_000;
const EXPECTED_CURSOR_DX: i64 = 10_000;
const PORT_ENV: &str = "SYNAPSE_HID_BENCH_PORT";
const MOVE_PAYLOAD: [u8; 4] = [1, 0, 0, 0];

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
        bench_high_volume(&mut criterion, gateway);
        criterion.final_summary();
    }

    let Some(mut gateway) = connect_or_ci_skip()? else {
        return Ok(());
    };
    let report = measure_high_volume(&mut gateway)?;
    report.print();
    assert!(
        report.pass,
        "{BENCH_NAME} failed: wall_ms={} command_delta={} dropped_delta={} crc_delta={} cursor_dx={}",
        report.wall_ms,
        report.command_delta,
        report.dropped_delta,
        report.crc_delta,
        display_cursor_delta(report.cursor_dx)
    );
    Ok(())
}

fn bench_high_volume(criterion: &mut Criterion, mut gateway: HidGateway) {
    criterion.bench_function(BENCH_NAME, |bencher| {
        bencher.iter_custom(|iterations| {
            let mut total = Duration::ZERO;
            for _ in 0..iterations {
                let started = Instant::now();
                send_high_volume(&mut gateway)
                    .unwrap_or_else(|error| panic!("{BENCH_NAME} iteration failed: {error}"));
                let elapsed = started.elapsed();
                black_box(elapsed);
                total = total.saturating_add(elapsed);
            }
            total
        });
    });
}

fn measure_high_volume(gateway: &mut HidGateway) -> Result<BenchReport, Box<dyn Error>> {
    let before_cursor = cursor_position()?;
    let before = gateway.get_telemetry()?;
    let started = Instant::now();
    send_high_volume(gateway)?;
    let elapsed = started.elapsed();
    let after = gateway.get_telemetry()?;
    let after_cursor = cursor_position()?;

    let command_delta = telemetry_command_delta_excluding_after_read(before, after);
    let dropped_delta = after.frames_dropped.saturating_sub(before.frames_dropped);
    let crc_delta = after.crc_errors.saturating_sub(before.crc_errors);
    let cursor_dx = cursor_delta_x(before_cursor, after_cursor);
    let wall_ms = elapsed.as_millis();
    let pass = wall_ms <= TARGET_WALL_MS
        && command_delta == EXPECTED_COMMANDS
        && dropped_delta == 0
        && crc_delta == 0
        && cursor_dx == Some(EXPECTED_CURSOR_DX);

    Ok(BenchReport {
        before,
        after,
        before_cursor,
        after_cursor,
        wall_ms,
        command_delta,
        dropped_delta,
        crc_delta,
        cursor_dx,
        pass,
    })
}

fn send_high_volume(gateway: &mut HidGateway) -> Result<(), Box<dyn Error>> {
    let commands =
        vec![HostCommandRequest::new(HOST_COMMAND_MOUSE_MOVE_REL, &MOVE_PAYLOAD); MOVE_COUNT];
    gateway.send_commands(&commands)?;
    Ok(())
}

struct BenchReport {
    before: HidTelemetrySnapshot,
    after: HidTelemetrySnapshot,
    before_cursor: Option<CursorPosition>,
    after_cursor: Option<CursorPosition>,
    wall_ms: u128,
    command_delta: u32,
    dropped_delta: u32,
    crc_delta: u32,
    cursor_dx: Option<i64>,
    pass: bool,
}

impl BenchReport {
    fn print(&self) {
        println!(
            "readback={BENCH_NAME} before={:?} after={:?} before_cursor={:?} after_cursor={:?} wall_ms={} target_wall_ms={} command_delta={} expected_commands={} dropped_delta={} crc_delta={} cursor_dx={} expected_cursor_dx={} result_value={}",
            self.before,
            self.after,
            self.before_cursor,
            self.after_cursor,
            self.wall_ms,
            TARGET_WALL_MS,
            self.command_delta,
            EXPECTED_COMMANDS,
            self.dropped_delta,
            self.crc_delta,
            display_cursor_delta(self.cursor_dx),
            EXPECTED_CURSOR_DX,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorPosition {
    x: i32,
    y: i32,
}

fn cursor_delta_x(before: Option<CursorPosition>, after: Option<CursorPosition>) -> Option<i64> {
    let before = before?;
    let after = after?;
    Some(i64::from(after.x) - i64::from(before.x))
}

#[cfg(windows)]
fn cursor_position() -> Result<Option<CursorPosition>, Box<dyn Error>> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetPhysicalCursorPos};

    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetPhysicalCursorPos(&raw mut point) }?;
    Ok(Some(CursorPosition {
        x: point.x,
        y: point.y,
    }))
}

#[cfg(not(windows))]
fn cursor_position() -> Result<Option<CursorPosition>, Box<dyn Error>> {
    Ok(None)
}

fn display_cursor_delta(delta: Option<i64>) -> String {
    delta.map_or_else(|| "n/a".to_owned(), |value| value.to_string())
}

fn ci_enabled() -> bool {
    env::var_os("CI").is_some()
}
