//! Owned-PTY terminal capture for spawned agents (#902).
//!
//! Synapse launches each spawned agent attached to a ConPTY pseudoconsole it
//! owns, so it can (1) record the raw VT byte stream as an asciicast v3 file for
//! replay (#920) and live streaming (#914), and (2) maintain a VT shadow screen
//! of the current terminal contents.
//!
//! This module is split into focused, independently-tested pieces:
//!   - [`asciicast`] — the asciicast v3 NDJSON recorder (pure, no Windows deps).
//!   - [`shadow_screen`] — a VT-parsed shadow screen of current terminal state.
//!   - `conpty` (Windows only) — the ConPTY process-attach + capture loop.

// The recorder and shadow screen are fully exercised by their unit tests and
// are wired into the live ConPTY capture loop + the #914 streaming endpoint as
// those land; until then their public surface is legitimately not yet called
// from non-test code.
#![allow(dead_code)]

pub(crate) mod asciicast;
pub(crate) mod capture;
pub(crate) mod shadow_screen;
