use pico_hid::led::{
    ERROR_CRC_THRESHOLD_PER_SEC, IDLE_AFTER_MS, IDLE_BLINK_PERIOD_MS, LedInputs, LedMode,
    WATCHDOG_BLINK_PERIOD_MS, WATCHDOG_WINDOW_MS, led_output,
};

#[test]
fn idle_uses_half_hertz_slow_blink() {
    let on = led_output(inputs(IDLE_BLINK_PERIOD_MS, None, None, 0));
    assert_eq!(on.mode, LedMode::IdleSlowBlink);
    assert!(on.on);

    let off = led_output(inputs(
        IDLE_BLINK_PERIOD_MS + IDLE_BLINK_PERIOD_MS / 2,
        None,
        None,
        0,
    ));
    assert_eq!(off.mode, LedMode::IdleSlowBlink);
    assert!(!off.on);
}

#[test]
fn active_is_steady_until_five_second_boundary() {
    let active = led_output(inputs(123, Some(IDLE_AFTER_MS), None, 0));
    assert_eq!(active.mode, LedMode::ActiveSteady);
    assert!(active.on);

    let idle = led_output(inputs(123, Some(IDLE_AFTER_MS + 1), None, 0));
    assert_eq!(idle.mode, LedMode::IdleSlowBlink);
}

#[test]
fn watchdog_fast_blink_takes_priority_over_active() {
    let on = led_output(inputs(0, Some(10), Some(WATCHDOG_WINDOW_MS), 0));
    assert_eq!(on.mode, LedMode::WatchdogFastBlink);
    assert!(on.on);

    let off = led_output(inputs(WATCHDOG_BLINK_PERIOD_MS / 2, Some(10), Some(10), 0));
    assert_eq!(off.mode, LedMode::WatchdogFastBlink);
    assert!(!off.on);

    let active = led_output(inputs(0, Some(10), Some(WATCHDOG_WINDOW_MS + 1), 0));
    assert_eq!(active.mode, LedMode::ActiveSteady);
}

#[test]
fn error_sos_takes_highest_priority() {
    let not_error = led_output(inputs(0, Some(10), Some(10), ERROR_CRC_THRESHOLD_PER_SEC));
    assert_eq!(not_error.mode, LedMode::WatchdogFastBlink);

    let error_on = led_output(inputs(
        0,
        Some(10),
        Some(10),
        ERROR_CRC_THRESHOLD_PER_SEC + 1,
    ));
    assert_eq!(error_on.mode, LedMode::ErrorSos);
    assert!(error_on.on);

    let error_off = led_output(inputs(
        150,
        Some(10),
        Some(10),
        ERROR_CRC_THRESHOLD_PER_SEC + 1,
    ));
    assert_eq!(error_off.mode, LedMode::ErrorSos);
    assert!(!error_off.on);
}

fn inputs(
    now_ms: u32,
    ms_since_last_command: Option<u32>,
    ms_since_watchdog_fire: Option<u32>,
    crc_errors_last_second: u32,
) -> LedInputs {
    LedInputs {
        now_ms,
        ms_since_last_command,
        ms_since_watchdog_fire,
        crc_errors_last_second,
    }
}
