pub const LED_TICK_MS: u64 = 100;
pub const IDLE_AFTER_MS: u32 = 5000;
pub const WATCHDOG_WINDOW_MS: u32 = 2000;
pub const ERROR_CRC_THRESHOLD_PER_SEC: u32 = 10;
pub const IDLE_BLINK_PERIOD_MS: u32 = 2000;
pub const WATCHDOG_BLINK_PERIOD_MS: u32 = 200;
pub const SOS_UNIT_MS: u16 = 150;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedMode {
    IdleSlowBlink,
    ActiveSteady,
    WatchdogFastBlink,
    ErrorSos,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedInputs {
    pub now_ms: u32,
    pub ms_since_last_command: Option<u32>,
    pub ms_since_watchdog_fire: Option<u32>,
    pub crc_errors_last_second: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedOutput {
    pub mode: LedMode,
    pub on: bool,
}

#[derive(Clone, Copy)]
struct Segment {
    on: bool,
    duration_ms: u16,
}

const SOS_PATTERN: [Segment; 18] = [
    dot(true),
    dot(false),
    dot(true),
    dot(false),
    dot(true),
    letter_gap(),
    dash(true),
    dot(false),
    dash(true),
    dot(false),
    dash(true),
    letter_gap(),
    dot(true),
    dot(false),
    dot(true),
    dot(false),
    dot(true),
    word_gap(),
];

const fn dot(on: bool) -> Segment {
    Segment {
        on,
        duration_ms: SOS_UNIT_MS,
    }
}

const fn dash(on: bool) -> Segment {
    Segment {
        on,
        duration_ms: SOS_UNIT_MS * 3,
    }
}

const fn letter_gap() -> Segment {
    Segment {
        on: false,
        duration_ms: SOS_UNIT_MS * 3,
    }
}

const fn word_gap() -> Segment {
    Segment {
        on: false,
        duration_ms: SOS_UNIT_MS * 7,
    }
}

pub fn led_output(inputs: LedInputs) -> LedOutput {
    let mode = led_mode(inputs);
    let on = match mode {
        LedMode::IdleSlowBlink => blink_on(inputs.now_ms, IDLE_BLINK_PERIOD_MS),
        LedMode::ActiveSteady => true,
        LedMode::WatchdogFastBlink => blink_on(inputs.now_ms, WATCHDOG_BLINK_PERIOD_MS),
        LedMode::ErrorSos => sos_on(inputs.now_ms),
    };

    LedOutput { mode, on }
}

pub fn led_mode(inputs: LedInputs) -> LedMode {
    if inputs.crc_errors_last_second > ERROR_CRC_THRESHOLD_PER_SEC {
        return LedMode::ErrorSos;
    }

    if matches!(
        inputs.ms_since_watchdog_fire,
        Some(elapsed) if elapsed <= WATCHDOG_WINDOW_MS
    ) {
        return LedMode::WatchdogFastBlink;
    }

    if matches!(
        inputs.ms_since_last_command,
        Some(elapsed) if elapsed <= IDLE_AFTER_MS
    ) {
        return LedMode::ActiveSteady;
    }

    LedMode::IdleSlowBlink
}

fn blink_on(now_ms: u32, period_ms: u32) -> bool {
    now_ms % period_ms < period_ms / 2
}

fn sos_on(now_ms: u32) -> bool {
    let mut phase = now_ms % sos_period_ms();
    let mut index = 0;

    while index < SOS_PATTERN.len() {
        let segment = SOS_PATTERN[index];
        let duration = segment.duration_ms as u32;
        if phase < duration {
            return segment.on;
        }
        phase -= duration;
        index += 1;
    }

    false
}

fn sos_period_ms() -> u32 {
    let mut total = 0;
    let mut index = 0;

    while index < SOS_PATTERN.len() {
        total += SOS_PATTERN[index].duration_ms as u32;
        index += 1;
    }

    total
}
