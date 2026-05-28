use std::{
    error::Error,
    fmt,
    io::{self, BufRead, Write},
};

use anyhow::Context;
use synapse_core::error_codes;

pub const HARDWARE_HID_ACK_PHRASE: &str = "I AUTHORIZE HARDWARE INPUT";
pub const HARDWARE_CONSENT_REFUSED_REASON: &str = "hardware_consent_refused";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HardwareConsentInput {
    Interactive,
    #[cfg(test)]
    Provided(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardwareConsentRefused;

impl HardwareConsentRefused {
    #[must_use]
    pub const fn code() -> &'static str {
        error_codes::SAFETY_PROFILE_ACTION_DENIED
    }

    #[must_use]
    pub const fn reason() -> &'static str {
        HARDWARE_CONSENT_REFUSED_REASON
    }
}

impl fmt::Display for HardwareConsentRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} reason={}",
            Self::code(),
            HARDWARE_CONSENT_REFUSED_REASON
        )
    }
}

impl Error for HardwareConsentRefused {}

pub fn require_hardware_hid_consent(port: &str, input: HardwareConsentInput) -> anyhow::Result<()> {
    match input {
        HardwareConsentInput::Interactive => read_interactive_consent(port),
        #[cfg(test)]
        HardwareConsentInput::Provided(value) => verify_hardware_hid_consent(value),
    }
}

#[must_use]
pub fn hardware_hid_prompt(port: &str) -> String {
    format!(
        "Synapse is about to enable hardware HID input via {port}.\n\
This will physically inject keyboard/mouse/gamepad events into the OS\n\
that other apps (including anti-cheat) cannot distinguish from user input.\n\
\n\
Type '{HARDWARE_HID_ACK_PHRASE}' and press Enter to continue.\n"
    )
}

pub fn verify_hardware_hid_consent(input: &str) -> anyhow::Result<()> {
    if input == HARDWARE_HID_ACK_PHRASE {
        return Ok(());
    }
    Err(HardwareConsentRefused.into())
}

fn read_interactive_consent(port: &str) -> anyhow::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr
        .write_all(hardware_hid_prompt(port).as_bytes())
        .context("write hardware HID consent prompt")?;
    stderr
        .flush()
        .context("flush hardware HID consent prompt")?;

    let mut line = String::new();
    let mut stdin = io::stdin().lock();
    stdin
        .read_line(&mut line)
        .context("read hardware HID consent response")?;
    drop(stdin);
    let line = strip_line_ending(&line);
    verify_hardware_hid_consent(line)
}

fn strip_line_ending(line: &str) -> &str {
    let without_lf = line.strip_suffix('\n').unwrap_or(line);
    without_lf.strip_suffix('\r').unwrap_or(without_lf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_matches_issue_phrase_and_port() {
        let prompt = hardware_hid_prompt("COM426");
        assert!(prompt.contains("via COM426."), "{prompt}");
        assert!(
            prompt.contains("physically inject keyboard/mouse/gamepad"),
            "{prompt}"
        );
        assert!(prompt.contains("I AUTHORIZE HARDWARE INPUT"), "{prompt}");
    }

    #[test]
    fn consent_requires_exact_phrase() {
        assert!(verify_hardware_hid_consent(HARDWARE_HID_ACK_PHRASE).is_ok());
        for value in [
            "",
            "I AUTHORIZE HARDWARE INPUT ",
            "i authorize hardware input",
            "I understand Synapse hardware HID can generate real keyboard, mouse, and gamepad input on this computer.",
        ] {
            let error = verify_hardware_hid_consent(value).unwrap_err();
            assert!(
                error.downcast_ref::<HardwareConsentRefused>().is_some(),
                "{error:#}"
            );
        }
    }
}
