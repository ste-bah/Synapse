use std::sync::Arc;

use synapse_core::{GamepadController, GamepadReport};

use crate::ActionError;

use super::{
    error::retry_vigem_update,
    reports::{ds4_report_snapshot, x360_report_snapshot, xgamepad_from_snapshot},
    state::neutral_gamepad_report,
};

#[cfg(windows)]
#[derive(Debug)]
pub(super) enum VigemPad {
    X360(VigemX360Pad),
    Ds4(VigemDs4Pad),
}

#[cfg(windows)]
impl VigemPad {
    pub(super) const fn new_x360(
        target: vigem_client::Xbox360Wired<Arc<vigem_client::Client>>,
        report: GamepadReport,
    ) -> Self {
        Self::X360(VigemX360Pad { target, report })
    }

    pub(super) const fn new_ds4(
        target: vigem_client::DualShock4Wired<Arc<vigem_client::Client>>,
        report: GamepadReport,
    ) -> Self {
        Self::Ds4(VigemDs4Pad { target, report })
    }

    pub(super) const fn controller(&self) -> GamepadController {
        match self {
            Self::X360(_pad) => GamepadController::X360,
            Self::Ds4(_pad) => GamepadController::Ds4,
        }
    }

    pub(super) fn update(&mut self, report: &GamepadReport) -> Result<(), ActionError> {
        match self {
            Self::X360(pad) => pad.update(report),
            Self::Ds4(pad) => pad.update(report),
        }
    }

    pub(super) fn neutralize(&mut self) -> Result<(), ActionError> {
        let neutral = neutral_gamepad_report(self.controller());
        self.update(&neutral)
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub(super) struct VigemX360Pad {
    target: vigem_client::Xbox360Wired<Arc<vigem_client::Client>>,
    report: GamepadReport,
}

#[cfg(windows)]
impl VigemX360Pad {
    fn update(&mut self, report: &GamepadReport) -> Result<(), ActionError> {
        let gamepad = xgamepad_from_snapshot(x360_report_snapshot(report));
        retry_vigem_update("update_x360_report", || self.target.update(&gamepad))?;
        self.report = report.clone();
        Ok(())
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub(super) struct VigemDs4Pad {
    target: vigem_client::DualShock4Wired<Arc<vigem_client::Client>>,
    report: GamepadReport,
}

#[cfg(windows)]
impl VigemDs4Pad {
    fn update(&mut self, report: &GamepadReport) -> Result<(), ActionError> {
        let ds4_report = ds4_report_snapshot(report).into_vigem_report();
        retry_vigem_update("update_ds4_report", || self.target.update(&ds4_report))?;
        self.report = report.clone();
        Ok(())
    }
}
