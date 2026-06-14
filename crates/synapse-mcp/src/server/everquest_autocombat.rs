//! Server-side autonomous combat loop for the level-1 `EverQuest` wizard (#550).
//!
//! One MCP call runs many bounded game engagements (acquire -> consider ->
//! melee auto-attack + nuke-when-mana-allows -> confirm kill -> recover ->
//! re-acquire) so the agent does not pay a stdio round-trip per keystroke.
//! Every emitted key still flows through the audited `act_keymap` action path
//! and the #517 foreground/profile/scope/UI gates.
//!
//! Combat model (live-test finding): a level-1 wizard (≈28 mana, ≈30 HP) cannot
//! kill mobs with a single `hotbar4` Blast of Cold nuke — one cast empties the
//! bar and can be resisted. So the loop fights like a player: it starts melee
//! auto-attack on a con-safe target and keeps meleeing through the fight, only
//! casting the nuke when mana% is at/above a threshold, and persists on ONE
//! target until it dies or a stop condition fires. `resisted`/`fizzled`/`miss`
//! are "keep fighting", not "abandon target". Looting is intentionally out of
//! scope for the L1->L2 MVP; XP comes from kills + sit-recover.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rmcp::{ErrorData, RoleServer, schemars::JsonSchema, service::RequestContext};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_core::{Profile, error_codes};
use synapse_everquest::tail_log;
use tokio::time::sleep;

use super::{
    Json, Parameters, SynapseService, act_keymap_with_handle, everquest_log::EVERQUEST_PROFILE_ID,
    release_all_with_handles, tool, tool_router,
};
use crate::{
    m1::mcp_error,
    m2::{ActKeymapParams, PressBackend, ReleaseAllParams},
};

const TOOL: &str = "everquest_autocombat";
const SCHEMA_VERSION: u32 = 1;
const RUN_ROW_PREFIX: &str = "everquest/autocombat/v1/everquest.live";
const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_LOG_EVENTS: usize = 128;
const KEY_HOLD_MS: u32 = 33;
const CONSIDER_TIMEOUT: Duration = Duration::from_millis(2600);
const FIGHT_TICK: Duration = Duration::from_millis(900);
const RECOVER_TIMEOUT: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_millis(120);
const INTER_KEY_DELAY: Duration = Duration::from_millis(250);
const MAX_TARGET_CYCLES: u32 = 3;
const DEFAULT_HOTBAR_ALIAS: &str = "hotbar4";
// Roam/chase timing scaffolding. The roam/chase movement loop that consumes
// these is not yet wired; kept so the policy is ready when it lands. EverQuest
// is a legacy target (see docs/computergames/08_supported_use_policy.md §7.1.1).
/// One roam move: hold `forward` for this long after a small turn, then re-scan.
#[expect(dead_code, reason = "roam/chase movement loop not yet wired")]
const ROAM_FORWARD_HOLD_MS: u32 = 1200;
/// One roam turn: hold a turn alias for this long to face a fresh direction.
#[expect(dead_code, reason = "roam/chase movement loop not yet wired")]
const ROAM_TURN_HOLD_MS: u32 = 350;
/// One chase burst: hold `forward` for this long, then re-poll the fight window.
#[expect(dead_code, reason = "roam/chase movement loop not yet wired")]
const CHASE_FORWARD_HOLD_MS: u32 = 800;
/// A chase ends if no combat lines are seen for this many consecutive bursts.
#[expect(dead_code, reason = "roam/chase movement loop not yet wired")]
const CHASE_IDLE_BURSTS: u32 = 3;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActAutocombatParams {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_max_duration_s")]
    pub max_duration_s: u32,
    #[serde(default = "default_hp_floor")]
    pub hp_floor_percent: u32,
    #[serde(default = "default_mana_floor")]
    pub mana_floor_percent: u32,
    #[serde(default = "default_target_level_max")]
    pub target_level_max: u32,
    #[serde(default = "default_stop_at_level")]
    pub stop_at_level: u32,
    #[serde(default = "default_cast_mana_cost")]
    pub cast_mana_cost_percent: u32,
    #[serde(default = "default_engagement_timeout_s")]
    pub engagement_timeout_s: u32,
    #[serde(default = "default_hotbar_alias")]
    pub hotbar_alias: String,
    #[serde(default = "default_max_roam_steps")]
    pub max_roam_steps: u32,
    #[serde(default = "default_max_chase_s")]
    pub max_chase_s: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

const fn default_max_iterations() -> u32 {
    8
}
const fn default_max_duration_s() -> u32 {
    120
}
const fn default_hp_floor() -> u32 {
    50
}
const fn default_mana_floor() -> u32 {
    30
}
const fn default_target_level_max() -> u32 {
    2
}
const fn default_stop_at_level() -> u32 {
    2
}
const fn default_cast_mana_cost() -> u32 {
    70
}
const fn default_engagement_timeout_s() -> u32 {
    30
}
fn default_hotbar_alias() -> String {
    DEFAULT_HOTBAR_ALIAS.to_owned()
}
const fn default_max_roam_steps() -> u32 {
    6
}
const fn default_max_chase_s() -> u32 {
    12
}

/// Validated, clamped loop policy derived from `ActAutocombatParams`.
#[derive(Clone, Debug)]
struct Policy {
    max_iterations: u32,
    max_duration: Duration,
    hp_floor: u32,
    mana_floor: u32,
    target_level_max: u32,
    stop_at_level: u32,
    cast_mana_cost: u32,
    engagement_timeout: Duration,
    hotbar_alias: String,
    max_roam_steps: u32,
    max_chase: Duration,
    run_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActAutocombatIteration {
    pub index: u32,
    pub target_summary: Option<String>,
    pub target_level: Option<u32>,
    pub con_decision: String,
    /// Whether melee auto-attack was asserted for this engagement.
    pub melee_started: bool,
    /// Nuke casts emitted during this engagement.
    pub casts: u32,
    /// Whether at least one nuke cast was emitted (kept for back-compat).
    pub cast: bool,
    /// Number of roam moves taken during the find phase for this iteration.
    pub roam_steps: u32,
    /// Whether the wizard chased a fleeing target during this engagement.
    pub chased: bool,
    pub outcome: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActAutocombatResponse {
    pub ok: bool,
    pub iterations: u32,
    pub kills: u32,
    pub casts: u32,
    pub casts_resisted: u32,
    pub casts_fizzled: u32,
    pub started_level: Option<u32>,
    pub final_level: Option<u32>,
    pub final_xp_percent: Option<u32>,
    pub stop_reason: String,
    pub run_row_key: String,
    pub looting_note: String,
    pub per_iteration: Vec<ActAutocombatIteration>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AutocombatRunRow {
    schema_version: u32,
    row_kind: String,
    profile_id: String,
    run_id: String,
    generated_at: DateTime<Utc>,
    response: ActAutocombatResponse,
}

/// Distinct, machine-readable reasons the loop halts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopReason {
    ReachedTargetLevel,
    MaxIterations,
    MaxDuration,
    OperatorPanic,
    ForegroundLost,
    ChatUnsafe,
    HpFloor,
    NoSafeTarget,
}

impl StopReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReachedTargetLevel => "reached_target_level",
            Self::MaxIterations => "max_iterations",
            Self::MaxDuration => "max_duration",
            Self::OperatorPanic => "operator_panic",
            Self::ForegroundLost => "foreground_lost",
            Self::ChatUnsafe => "chat_unsafe",
            Self::HpFloor => "hp_floor",
            Self::NoSafeTarget => "no_safe_target",
        }
    }
    const fn is_success(self) -> bool {
        matches!(
            self,
            Self::ReachedTargetLevel | Self::MaxIterations | Self::MaxDuration
        )
    }
}

/// Consider-line classification for a level-1 wizard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConDecision {
    Safe,
    TooHigh,
    NonNpc,
    Unknown,
}

impl ConDecision {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::TooHigh => "too_high",
            Self::NonNpc => "non_npc",
            Self::Unknown => "unknown",
        }
    }
}

/// How a single fight tick should be interpreted from the latest log window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FightSignal {
    /// `<mob> has been slain by ...` — engagement won.
    Slain,
    /// The mob is fleeing at low HP (`flees`, `tries to flee`, `runs away`) and
    /// is still alive — chase it to stay in melee range and finish it. The
    /// detector that produces this is not yet wired; the fight loop handles the
    /// variant as "continue" until the chase movement loop lands.
    #[expect(dead_code, reason = "fleeing detector / chase loop not yet wired")]
    Fleeing,
    /// Target lost / no target (auto-attack can no longer reach it).
    TargetLost,
    /// Combat ongoing (hits exchanged, resist, fizzle, miss) — keep fighting.
    Continue,
    /// No combat-relevant lines parsed in this window.
    Idle,
}

/// Terminal outcome of one engagement (one iteration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EngagementOutcome {
    Slain,
    Fled,
    HpFloor,
    Timeout,
    NoTarget,
    OperatorPanic,
}

impl EngagementOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Slain => "slain",
            Self::Fled => "fled",
            Self::HpFloor => "hp_floor",
            Self::Timeout => "timeout",
            Self::NoTarget => "no_target",
            Self::OperatorPanic => "operator_panic",
        }
    }
}

#[tool_router(router = everquest_autocombat_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Run a bounded, operator-attended, server-side EverQuest combat loop for the level-1 wizard (acquire -> consider -> melee + nuke-when-mana -> confirm kill -> recover -> re-acquire)"
    )]
    pub async fn everquest_autocombat(
        &self,
        params: Parameters<ActAutocombatParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ActAutocombatResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_autocombat"
        );
        let policy = normalize_policy(params.0);
        let request_details = json!({ "run_id": policy.run_id, "policy": policy_details(&policy) });
        let profile = match self.autocombat_preflight() {
            Ok(profile) => profile,
            Err(error) => {
                self.audit_action_denied_with_details(TOOL, &error, &request_details);
                return Err(error);
            }
        };
        self.audit_action_started_with_details(TOOL, &request_details)?;
        let action_session_id =
            super::context::mcp_session_id_from_request_context(&request_context)?;
        let result = self
            .run_autocombat_loop(&policy, &profile, action_session_id.as_deref())
            .await;
        if let Ok(response) = &result {
            let _ = self.persist_autocombat_run(&policy, response);
        }
        self.audit_action_result(TOOL, &result)?;
        result.map(Json)
    }
}

impl SynapseService {
    fn autocombat_preflight(&self) -> Result<Profile, ErrorData> {
        self.ensure_supported_use_allows_action(TOOL)?;
        self.ensure_active_everquest_profile(TOOL)?;
        self.ensure_literal_command_chat_guard(TOOL, "/autocombat")?;
        self.resolve_active_everquest_log()
            .map_err(|detail| mcp_error(error_codes::ACTION_TARGET_INVALID, detail))?;
        let runtime = self.profile_runtime()?;
        runtime
            .profile(EVERQUEST_PROFILE_ID)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::PROFILE_NOT_FOUND,
                    format!("active profile {EVERQUEST_PROFILE_ID} was not found"),
                )
            })
    }

    async fn run_autocombat_loop(
        &self,
        policy: &Policy,
        profile: &Profile,
        action_session_id: Option<&str>,
    ) -> Result<ActAutocombatResponse, ErrorData> {
        let started = Instant::now();
        let panic_epoch = synapse_action::operator_release_epoch();
        let started_level = self.read_level();
        let ctx = AutocombatLoopContext {
            policy,
            profile,
            panic_epoch,
            run_started: started,
            action_session_id,
        };
        let mut state = LoopState::default();
        let mut stop = StopReason::MaxIterations;
        for index in 0..policy.max_iterations {
            if let Some(reason) = self.evaluate_stop(policy, panic_epoch, started) {
                stop = reason;
                self.handle_stop_recovery(reason, profile, action_session_id)
                    .await;
                break;
            }
            let iteration = self.run_engagement(index, &ctx, &mut state).await?;
            let outcome = iteration.outcome.clone();
            state.iterations.push(iteration);
            if outcome == EngagementOutcome::OperatorPanic.as_str() {
                stop = StopReason::OperatorPanic;
                self.handle_stop_recovery(StopReason::OperatorPanic, profile, action_session_id)
                    .await;
                break;
            }
            if outcome == EngagementOutcome::HpFloor.as_str() {
                stop = StopReason::HpFloor;
                self.handle_stop_recovery(StopReason::HpFloor, profile, action_session_id)
                    .await;
                break;
            }
            if outcome == EngagementOutcome::Slain.as_str()
                && self
                    .read_level()
                    .is_some_and(|lvl| lvl >= policy.stop_at_level)
            {
                stop = StopReason::ReachedTargetLevel;
                break;
            }
            if state.consecutive_no_target >= MAX_TARGET_CYCLES {
                stop = StopReason::NoSafeTarget;
                break;
            }
        }
        if started.elapsed() >= policy.max_duration && stop == StopReason::MaxIterations {
            stop = StopReason::MaxDuration;
        }
        Ok(self.finalize(policy, started_level, stop, state))
    }

    /// Stop conditions checked BEFORE emitting any input each engagement.
    fn evaluate_stop(
        &self,
        policy: &Policy,
        panic_epoch: u64,
        started: Instant,
    ) -> Option<StopReason> {
        if synapse_action::operator_release_requested_since(panic_epoch) {
            return Some(StopReason::OperatorPanic);
        }
        if started.elapsed() >= policy.max_duration {
            return Some(StopReason::MaxDuration);
        }
        let row = self.build_survival_readiness_row().ok()?;
        if !row.foreground.is_everquest_foreground {
            return Some(StopReason::ForegroundLost);
        }
        if row.ui_context.login_screen_visible || !chat_safe(&row.chat_input_state.decision) {
            return Some(StopReason::ChatUnsafe);
        }
        if row.hud.hp_percent.is_some_and(|hp| hp < policy.hp_floor) {
            return Some(StopReason::HpFloor);
        }
        None
    }

    async fn handle_stop_recovery(
        &self,
        reason: StopReason,
        profile: &Profile,
        action_session_id: Option<&str>,
    ) {
        match reason {
            StopReason::OperatorPanic => {
                let _ = self.release_all_best_effort().await;
            }
            // HP-floor flee: stop auto-attack (toggle off) then sit is unsafe while
            // being hit, so only drop melee and let the operator take over.
            StopReason::HpFloor => {
                let _ = self
                    .press_alias("auto_attack", profile, action_session_id)
                    .await;
            }
            _ => {}
        }
    }

    /// Acquire (or continue with) a single target and fight it to a terminal
    /// outcome. One engagement counts as one "iteration".
    async fn run_engagement(
        &self,
        index: u32,
        ctx: &AutocombatLoopContext<'_>,
        state: &mut LoopState,
    ) -> Result<ActAutocombatIteration, ErrorData> {
        let policy = ctx.policy;
        let profile = ctx.profile;
        let panic_epoch = ctx.panic_epoch;
        let run_started = ctx.run_started;
        let action_session_id = ctx.action_session_id;
        let log_path = self.autocombat_log_path()?;

        // If we are already trading blows with a living mob, skip the fresh
        // target+con (the live run saw `con_decision: unknown` on a mob that was
        // already engaged) and keep attacking the current target.
        let recent_combat = recent_combat_active(&log_path);

        let acquire_offset = file_len(&log_path);
        let (consider, decision, target_level) = if recent_combat {
            (None, ConDecision::Safe, None)
        } else {
            self.press_alias("target_nearest_npc", profile, action_session_id)
                .await?;
            sleep(INTER_KEY_DELAY).await;
            let con_offset = file_len(&log_path);
            self.press_alias("con", profile, action_session_id).await?;
            let summary = self.poll_consider(&log_path, con_offset).await;
            let decision = classify_con(summary.as_deref(), policy.target_level_max);
            let level = parse_target_level(summary.as_deref());
            (summary, decision, level)
        };

        let mut iteration = ActAutocombatIteration {
            index,
            target_summary: consider.clone(),
            target_level,
            con_decision: decision.as_str().to_owned(),
            melee_started: false,
            casts: 0,
            cast: false,
            // Roam/chase scaffolding is present but the movement logic is not yet
            // implemented; report inert values so the iteration shape is stable.
            roam_steps: 0,
            chased: false,
            outcome: EngagementOutcome::NoTarget.as_str().to_owned(),
        };

        if decision != ConDecision::Safe {
            state.consecutive_no_target += 1;
            // No live mob, safe to sit and recover toward mana floor for the next pull.
            self.recover_mana(policy, profile, panic_epoch, action_session_id)
                .await;
            return Ok(iteration);
        }
        state.consecutive_no_target = 0;

        // Engage: assert melee auto-attack ONCE for this fight (it is a toggle in
        // EQ and drops when the target dies). We track it on per-engagement.
        self.press_alias("auto_attack", profile, action_session_id)
            .await?;
        iteration.melee_started = true;

        let ctx = FightContext {
            log_path: &log_path,
            engage_offset: acquire_offset,
            panic_epoch,
            run_started,
        };
        let (outcome, tally) = self
            .fight_target(&ctx, policy, profile, &mut iteration, action_session_id)
            .await?;
        outcome.as_str().clone_into(&mut iteration.outcome);
        state.resisted += tally.resisted;
        state.fizzled += tally.fizzled;

        match outcome {
            EngagementOutcome::Slain => {
                state.kills += 1;
                // Auto-attack drops on death (no target); recover mana before next pull.
                self.recover_mana(policy, profile, panic_epoch, action_session_id)
                    .await;
            }
            EngagementOutcome::Fled | EngagementOutcome::NoTarget | EngagementOutcome::Timeout => {
                // Drop melee toggle so we don't carry it into the next pull, then recover.
                let _ = self
                    .press_alias("auto_attack", profile, action_session_id)
                    .await;
                self.recover_mana(policy, profile, panic_epoch, action_session_id)
                    .await;
            }
            EngagementOutcome::HpFloor | EngagementOutcome::OperatorPanic => {}
        }
        Ok(iteration)
    }

    /// Sustained single-target fight. Melee auto-attack is already on; here we
    /// only add nuke casts when mana allows and poll the log for the outcome.
    async fn fight_target(
        &self,
        ctx: &FightContext<'_>,
        policy: &Policy,
        profile: &Profile,
        iteration: &mut ActAutocombatIteration,
        action_session_id: Option<&str>,
    ) -> Result<(EngagementOutcome, CastTally), ErrorData> {
        let FightContext {
            log_path,
            engage_offset,
            panic_epoch,
            run_started,
        } = *ctx;
        let engage_started = Instant::now();
        // `read_fight_window` always reads from `engage_offset`, so the latest
        // window returned holds the full set of engagement events; tally
        // resist/fizzle once from it at fight end to avoid double-counting.
        let mut last_window: Vec<String> = Vec::new();
        loop {
            if synapse_action::operator_release_requested_since(panic_epoch) {
                return Ok((EngagementOutcome::OperatorPanic, tally_window(&last_window)));
            }
            if run_started.elapsed() >= policy.max_duration
                || engage_started.elapsed() >= policy.engagement_timeout
            {
                return Ok((EngagementOutcome::Timeout, tally_window(&last_window)));
            }

            // Per-tick HUD safety: HP-floor flee still fires mid-fight.
            let row = self.build_survival_readiness_row().ok();
            if let Some(row) = &row {
                if !row.foreground.is_everquest_foreground
                    || row.ui_context.login_screen_visible
                    || !chat_safe(&row.chat_input_state.decision)
                {
                    return Ok((EngagementOutcome::Timeout, tally_window(&last_window)));
                }
                if row.hud.hp_percent.is_some_and(|hp| hp < policy.hp_floor) {
                    return Ok((EngagementOutcome::HpFloor, tally_window(&last_window)));
                }
            }
            let mana = row.and_then(|row| row.hud.mana_percent);

            // Nuke when mana% is at/above the cast-cost threshold; otherwise melee.
            if should_cast_nuke(mana, policy.cast_mana_cost) {
                self.press_alias(&policy.hotbar_alias, profile, action_session_id)
                    .await?;
                iteration.casts += 1;
                iteration.cast = true;
            }

            // Poll the fight window for an outcome over one tick.
            last_window = self.read_fight_window(log_path, engage_offset).await;
            let refs: Vec<&str> = last_window.iter().map(String::as_str).collect();
            match classify_fight_signal(&refs) {
                FightSignal::Slain => {
                    return Ok((EngagementOutcome::Slain, tally_window(&last_window)));
                }
                FightSignal::TargetLost => {
                    let tally = tally_window(&last_window);
                    // Distinguish "fled/cleared" (was fighting) from "never engaged".
                    if iteration.casts > 0 || tally.has_combat {
                        return Ok((EngagementOutcome::Fled, tally));
                    }
                    return Ok((EngagementOutcome::NoTarget, tally));
                }
                // Resisted/fizzled/miss/hits-exchanged -> keep fighting. Fleeing
                // is treated as continue for now (chase movement is not yet
                // implemented): keep auto-attacking while the mob may be in range.
                FightSignal::Continue | FightSignal::Idle | FightSignal::Fleeing => {}
            }
        }
    }

    /// Read the current fight log window summaries (one fight tick of polling).
    async fn read_fight_window(&self, log_path: &std::path::Path, offset: u64) -> Vec<String> {
        let started = Instant::now();
        let mut latest: Vec<String> = Vec::new();
        while started.elapsed() < FIGHT_TICK {
            if let Ok(batch) = tail_log(log_path, offset, MAX_LOG_BYTES, MAX_LOG_EVENTS) {
                latest = batch
                    .events
                    .iter()
                    .map(|event| event.summary.clone())
                    .collect();
                let refs: Vec<&str> = latest.iter().map(String::as_str).collect();
                if matches!(
                    classify_fight_signal(&refs),
                    FightSignal::Slain | FightSignal::TargetLost
                ) {
                    return latest;
                }
            }
            sleep(POLL_INTERVAL).await;
        }
        latest
    }

    /// Sit to recover mana to the floor, bounded by `RECOVER_TIMEOUT`, then stand.
    /// Only call this when NOT in active combat (sitting breaks under damage).
    async fn recover_mana(
        &self,
        policy: &Policy,
        profile: &Profile,
        panic_epoch: u64,
        action_session_id: Option<&str>,
    ) {
        // Do not sit if a mob is currently meleeing us — that is unsafe and the
        // sit is interrupted anyway.
        if let Ok(log_path) = self.autocombat_log_path()
            && recent_combat_active(&log_path)
        {
            return;
        }
        if self
            .press_alias("sit", profile, action_session_id)
            .await
            .is_err()
        {
            return;
        }
        let started = Instant::now();
        while started.elapsed() < RECOVER_TIMEOUT {
            if synapse_action::operator_release_requested_since(panic_epoch) {
                let _ = self.release_all_best_effort().await;
                return;
            }
            let mana = self
                .build_survival_readiness_row()
                .ok()
                .and_then(|row| row.hud.mana_percent);
            if mana.is_some_and(|value| value >= policy.mana_floor) {
                break;
            }
            sleep(POLL_INTERVAL).await;
        }
        // Stand before the next pull.
        let _ = self.press_alias("sit", profile, action_session_id).await;
    }

    async fn poll_consider(&self, log_path: &std::path::Path, offset: u64) -> Option<String> {
        let started = Instant::now();
        while started.elapsed() < CONSIDER_TIMEOUT {
            if let Ok(batch) = tail_log(log_path, offset, MAX_LOG_BYTES, MAX_LOG_EVENTS)
                && let Some(summary) = consider_summary(&batch)
            {
                return Some(summary);
            }
            sleep(POLL_INTERVAL).await;
        }
        None
    }

    async fn press_alias(
        &self,
        alias: &str,
        profile: &Profile,
        action_session_id: Option<&str>,
    ) -> Result<(), ErrorData> {
        let (handle, recording, cancel) =
            self.m2_action_context_for_session_id(action_session_id.map(ToOwned::to_owned))?;
        let params = ActKeymapParams {
            alias: alias.to_owned(),
            hold_ms: KEY_HOLD_MS,
            backend: PressBackend::Auto,
            window_hwnd: None,
            cdp_target_id: None,
        };
        act_keymap_with_handle(handle, recording, cancel, profile, params)
            .await
            .map(|_response| ())
    }

    async fn release_all_best_effort(&self) -> Result<(), ErrorData> {
        let (handle, snapshot, reflex_runtime) = self.m2_release_all_context()?;
        release_all_with_handles(handle, snapshot, reflex_runtime, ReleaseAllParams {})
            .await
            .map(|_response| ())
    }

    fn read_level(&self) -> Option<u32> {
        self.build_survival_readiness_row()
            .ok()
            .and_then(|row| parse_level(row.hud.level_raw.as_deref()))
    }

    fn autocombat_log_path(&self) -> Result<std::path::PathBuf, ErrorData> {
        self.resolve_active_everquest_log()
            .map(|active| active.log.path)
            .map_err(|detail| mcp_error(error_codes::ACTION_TARGET_INVALID, detail))
    }

    fn finalize(
        &self,
        policy: &Policy,
        started_level: Option<u32>,
        stop: StopReason,
        state: LoopState,
    ) -> ActAutocombatResponse {
        let final_row = self.build_survival_readiness_row().ok();
        let final_level = final_row
            .as_ref()
            .and_then(|row| parse_level(row.hud.level_raw.as_deref()));
        let casts = state.iterations.iter().map(|it| it.casts).sum();
        ActAutocombatResponse {
            ok: stop.is_success(),
            iterations: u32::try_from(state.iterations.len()).unwrap_or(u32::MAX),
            kills: state.kills,
            casts,
            casts_resisted: state.resisted,
            casts_fizzled: state.fizzled,
            started_level,
            final_level,
            final_xp_percent: None,
            stop_reason: stop.as_str().to_owned(),
            run_row_key: format!("{RUN_ROW_PREFIX}/{}", policy.run_id),
            looting_note:
                "Looting is out of scope for the L1->L2 MVP; XP comes from kills + sit-recover."
                    .to_owned(),
            per_iteration: state.iterations,
        }
    }

    fn persist_autocombat_run(
        &self,
        policy: &Policy,
        response: &ActAutocombatResponse,
    ) -> Result<(), ErrorData> {
        let row = AutocombatRunRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_autocombat_run".to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            run_id: policy.run_id.clone(),
            generated_at: Utc::now(),
            response: response.clone(),
        };
        let key = response.run_row_key.clone();
        let encoded = serde_json::to_vec(&row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode autocombat run row: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while writing autocombat run row",
            )
        })?;
        let result = runtime
            .storage_put_kv_rows(vec![(key.into_bytes(), encoded)])
            .map_err(|error| mcp_error(error_codes::STORAGE_WRITE_FAILED, error.to_string()));
        drop(runtime);
        result
    }
}

/// Shared immutable state for one bounded autocombat loop.
#[derive(Clone, Copy)]
struct AutocombatLoopContext<'a> {
    policy: &'a Policy,
    profile: &'a Profile,
    panic_epoch: u64,
    run_started: Instant,
    action_session_id: Option<&'a str>,
}

/// Per-engagement timing/log context for the sustained fight loop.
#[derive(Clone, Copy)]
struct FightContext<'a> {
    log_path: &'a std::path::Path,
    engage_offset: u64,
    panic_epoch: u64,
    run_started: Instant,
}

#[derive(Debug, Default)]
struct LoopState {
    iterations: Vec<ActAutocombatIteration>,
    kills: u32,
    resisted: u32,
    fizzled: u32,
    consecutive_no_target: u32,
}

fn normalize_policy(params: ActAutocombatParams) -> Policy {
    Policy {
        max_iterations: params.max_iterations.clamp(1, 50),
        max_duration: Duration::from_secs(u64::from(params.max_duration_s.clamp(1, 600))),
        hp_floor: params.hp_floor_percent.min(100),
        mana_floor: params.mana_floor_percent.min(100),
        target_level_max: params.target_level_max,
        stop_at_level: params.stop_at_level.max(1),
        cast_mana_cost: params.cast_mana_cost_percent.min(100),
        engagement_timeout: Duration::from_secs(u64::from(
            params.engagement_timeout_s.clamp(1, 300),
        )),
        hotbar_alias: normalize_alias(&params.hotbar_alias),
        max_roam_steps: params.max_roam_steps.min(50),
        max_chase: Duration::from_secs(u64::from(params.max_chase_s.clamp(1, 120))),
        run_id: params
            .idempotency_key
            .map_or_else(default_run_id, |value| sanitize_run_id(&value)),
    }
}

fn normalize_alias(alias: &str) -> String {
    let trimmed = alias.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        DEFAULT_HOTBAR_ALIAS.to_owned()
    } else {
        trimmed
    }
}

fn default_run_id() -> String {
    format!("run-{}", Utc::now().format("%Y%m%dT%H%M%S%3fZ"))
}

fn sanitize_run_id(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        default_run_id()
    } else {
        cleaned
    }
}

fn policy_details(policy: &Policy) -> serde_json::Value {
    json!({
        "max_iterations": policy.max_iterations,
        "max_duration_s": policy.max_duration.as_secs(),
        "hp_floor_percent": policy.hp_floor,
        "mana_floor_percent": policy.mana_floor,
        "target_level_max": policy.target_level_max,
        "stop_at_level": policy.stop_at_level,
        "cast_mana_cost_percent": policy.cast_mana_cost,
        "engagement_timeout_s": policy.engagement_timeout.as_secs(),
        "hotbar_alias": policy.hotbar_alias,
        "max_roam_steps": policy.max_roam_steps,
        "max_chase_s": policy.max_chase.as_secs(),
    })
}

fn chat_safe(decision: &str) -> bool {
    decision == "allow_empty_chat_input"
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

fn consider_summary(batch: &synapse_everquest::EverQuestLogTailBatch) -> Option<String> {
    batch
        .events
        .iter()
        .rev()
        .find(|event| {
            event.kind == synapse_everquest::EverQuestLogKind::Consider
                || event.summary.to_ascii_lowercase().contains("regards you")
        })
        .map(|event| event.summary.clone())
}

/// Parse the target level from a consider summary (`(Lvl: N)` or `... level N`).
fn parse_target_level(summary: Option<&str>) -> Option<u32> {
    let text = summary?.to_ascii_lowercase();
    if let Some(rest) = text.split("lvl:").nth(1) {
        return rest
            .trim()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok();
    }
    if let Some(rest) = text.split("level ").nth(1) {
        return rest
            .trim()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok();
    }
    None
}

/// Parse the character level from the HUD level-raw OCR string.
fn parse_level(level_raw: Option<&str>) -> Option<u32> {
    level_raw?
        .split_ascii_whitespace()
        .find_map(|token| token.parse::<u32>().ok())
}

/// Heuristic: are we already in an active melee engagement with a living mob?
/// Reads the tail of the log and inspects recent combat lines.
fn recent_combat_active(log_path: &std::path::Path) -> bool {
    let offset = file_len(log_path).saturating_sub(MAX_LOG_BYTES as u64);
    let Ok(batch) = tail_log(log_path, offset, MAX_LOG_BYTES, MAX_LOG_EVENTS) else {
        return false;
    };
    let summaries: Vec<&str> = batch.events.iter().map(|e| e.summary.as_str()).collect();
    detect_active_combat(&summaries)
}

/// Counts of resist/fizzle (and whether any combat occurred) in a fight window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CastTally {
    resisted: u32,
    fizzled: u32,
    has_combat: bool,
}

/// Count resist/fizzle events and detect combat from a full engagement window.
fn tally_window(window: &[String]) -> CastTally {
    let mut tally = CastTally::default();
    for line in window {
        let lower = line.to_ascii_lowercase();
        if lower.contains("resist") {
            tally.resisted += 1;
        }
        if lower.contains("fizzle") {
            tally.fizzled += 1;
        }
        if line_combat_active(&lower) {
            tally.has_combat = true;
        }
    }
    tally
}

/// Decide whether to cast the nuke this fight tick. Cast only when mana% is
/// known AND at/above the cast-cost threshold (one Blast of Cold ~ most of a L1
/// mana bar). Unknown mana -> rely on melee, do not cast.
const fn should_cast_nuke(mana_percent: Option<u32>, cast_mana_cost_percent: u32) -> bool {
    match mana_percent {
        Some(mana) => mana >= cast_mana_cost_percent,
        None => false,
    }
}

/// Classify a consider line for a level-1 wizard. Safe = NPC, level within cap,
/// and the con phrase is not in the high-danger set.
fn classify_con(summary: Option<&str>, target_level_max: u32) -> ConDecision {
    let Some(text) = summary else {
        return ConDecision::Unknown;
    };
    let lower = text.to_ascii_lowercase();
    if lower.contains("merchant")
        || lower.contains(" player")
        || lower.contains("a player")
        || lower.contains("guard")
    {
        return ConDecision::NonNpc;
    }
    // "Red" cons mean the target is far above a level-1 wizard regardless of any
    // parsed level (and cover the no-level-parsed case); reject outright.
    if con_phrase_red(&lower) {
        return ConDecision::TooHigh;
    }
    // The absolute level is the primary safety gate. The con difficulty phrase
    // ("gamble" = yellow/even, "even fight" = white, "easy prey" = green) only
    // reflects RELATIVE level, which the absolute cap already bounds — so a
    // level-<=cap NPC is huntable for a ranged nuker even at a yellow ("gamble")
    // con. HP-floor flee + the operator panic hotkey remain the lethality guard.
    match parse_target_level(summary) {
        Some(level) if level > target_level_max => ConDecision::TooHigh,
        Some(_) => ConDecision::Safe,
        None => {
            if con_phrase_safe(&lower) {
                ConDecision::Safe
            } else {
                ConDecision::Unknown
            }
        }
    }
}

/// True only for cons that mean the target is much higher level than a level-1
/// wizard. "gamble" (yellow/even) and faction-hostility phrases are intentionally
/// NOT here — the absolute `target_level_max` gate handles level safety.
fn con_phrase_red(lower: &str) -> bool {
    lower.contains("crazy to attack")
        || lower.contains("kill you")
        || lower.contains("rip you")
        || lower.contains("deadly")
}

fn con_phrase_safe(lower: &str) -> bool {
    lower.contains("regards you indifferently")
        || lower.contains("looks upon you warmly")
        || lower.contains("even fight")
        || lower.contains("gamble")
        || lower.contains("easy prey")
        || lower.contains("afraid")
        || lower.contains("worthy opponent")
}

/// Whether a single log line means the target is gone (slain/fled/cleared).
fn line_target_lost(lower: &str) -> bool {
    lower.contains("has fled")
        || lower.contains("flees in terror")
        || lower.contains("you have no target")
        || lower.contains("you no longer have a target")
        || lower.contains("must first select a target")
        || lower.contains("must target")
}

/// Whether a single log line indicates ongoing melee/spell combat with a mob.
fn line_combat_active(lower: &str) -> bool {
    // Mob hitting us, or us hitting the mob (melee verbs + "for N points").
    (lower.contains(" you for ") && lower.contains("points of damage"))
        || (lower.contains(" for ") && lower.contains("points of damage") && melee_verb(lower))
        || lower.contains("auto attack is on")
        || lower.contains("you begin casting")
        || lower.contains("resisted your")
        || lower.contains("your target resisted")
}

fn melee_verb(lower: &str) -> bool {
    lower.contains(" pierce")
        || lower.contains(" slash")
        || lower.contains(" crush")
        || lower.contains(" hit ")
        || lower.contains(" bash")
        || lower.contains(" kick")
        || lower.contains(" bites")
        || lower.contains(" hits ")
        || lower.contains(" maul")
        || lower.contains(" claws")
}

/// Detect whether the recent log window shows an active melee engagement with a
/// living mob: combat lines are present and the most recent terminal line (if
/// any) is NOT a slain/target-lost line. Used to decide whether to keep
/// attacking the current target instead of re-acquiring.
fn detect_active_combat(summaries: &[&str]) -> bool {
    let mut combat_seen = false;
    for summary in summaries.iter().rev() {
        let lower = summary.to_ascii_lowercase();
        if lower.contains("has been slain by") || line_target_lost(&lower) {
            // Most recent terminal event ends the engagement — not active.
            return false;
        }
        if line_combat_active(&lower) {
            combat_seen = true;
        }
    }
    combat_seen
}

/// Classify the current fight tick from a window of log summaries (newest wins
/// for terminal signals). `resisted`/`fizzled`/`miss`/hits -> Continue.
fn classify_fight_signal(summaries: &[&str]) -> FightSignal {
    for summary in summaries.iter().rev() {
        let lower = summary.to_ascii_lowercase();
        if lower.contains("has been slain by") {
            return FightSignal::Slain;
        }
        if line_target_lost(&lower) {
            return FightSignal::TargetLost;
        }
    }
    // No terminal signal: is the mob still being fought?
    for summary in summaries {
        let lower = summary.to_ascii_lowercase();
        if line_combat_active(&lower)
            || lower.contains("fizzle")
            || lower.contains("resist")
            || lower.contains(" miss")
        {
            return FightSignal::Continue;
        }
    }
    FightSignal::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_safe_indifferent_low_level() {
        let line = "An araneidae spiderling regards you indifferently -- looks like an even fight. (Lvl: 1)";
        assert_eq!(classify_con(Some(line), 2), ConDecision::Safe);
    }

    #[test]
    fn classifies_gamble_level_two_within_cap_as_safe() {
        // Yellow ("gamble") con on a neutral-faction Lvl-2 NPC, cap 2: huntable
        // for a ranged nuker — the absolute level cap is the gate, not the phrase.
        let line =
            "A garter snake regards you indifferently -- looks like quite a gamble. (Lvl: 2)";
        assert_eq!(classify_con(Some(line), 2), ConDecision::Safe);
    }

    #[test]
    fn classifies_gamble_level_three_over_cap_as_too_high() {
        // Same yellow con but Lvl 3 > cap 2 -> rejected by the absolute level gate.
        let line = "An araneidae spiderling regards you indifferently -- looks like quite a gamble. (Lvl: 3)";
        assert_eq!(classify_con(Some(line), 2), ConDecision::TooHigh);
    }

    #[test]
    fn classifies_red_con_as_too_high_regardless_of_level() {
        let line = "An ancient wurm glares at you, ready to attack -- you would have to be crazy to attack it!";
        assert_eq!(classify_con(Some(line), 2), ConDecision::TooHigh);
    }

    #[test]
    fn classifies_level_over_cap_as_too_high() {
        let line = "consider a decaying skeleton level 5";
        assert_eq!(classify_con(Some(line), 2), ConDecision::TooHigh);
    }

    #[test]
    fn classifies_merchant_and_player_as_non_npc() {
        assert_eq!(
            classify_con(
                Some("Merchant Kinliat regards you indifferently. (Lvl: 1)"),
                2
            ),
            ConDecision::NonNpc
        );
        assert_eq!(
            classify_con(Some("Thenumberone a player regards you. (Lvl: 1)"), 2),
            ConDecision::NonNpc
        );
    }

    #[test]
    fn unknown_when_no_summary() {
        assert_eq!(classify_con(None, 2), ConDecision::Unknown);
    }

    #[test]
    fn parses_consider_levels() {
        assert_eq!(
            parse_target_level(Some("looks like a gamble. (Lvl: 3)")),
            Some(3)
        );
        assert_eq!(
            parse_target_level(Some("consider a skeleton level 5")),
            Some(5)
        );
        assert_eq!(parse_target_level(Some("no level here")), None);
    }

    #[test]
    fn parses_hud_level() {
        assert_eq!(
            parse_level(Some("Inventory Thenumberone 1 Wizard")),
            Some(1)
        );
        assert_eq!(parse_level(Some("Thenumberone 2 Wizard")), Some(2));
        assert_eq!(parse_level(None), None);
    }

    #[test]
    fn mana_cast_threshold_decision() {
        // Cast only when known mana% >= threshold.
        assert!(should_cast_nuke(Some(70), 70));
        assert!(should_cast_nuke(Some(100), 70));
        assert!(!should_cast_nuke(Some(69), 70));
        assert!(!should_cast_nuke(Some(0), 70));
        // Unknown mana -> rely on melee, do not cast.
        assert!(!should_cast_nuke(None, 70));
    }

    #[test]
    fn detects_active_combat_from_recent_hits() {
        let mob_hits_us = ["A moss snake bites YOU for 5 points of damage."];
        assert!(detect_active_combat(&mob_hits_us));

        let we_hit_mob = ["You pierce a moss snake for 5 points of damage."];
        assert!(detect_active_combat(&we_hit_mob));

        let auto_attack_on = ["Auto attack is on."];
        assert!(detect_active_combat(&auto_attack_on));
    }

    #[test]
    fn active_combat_false_after_slain_or_no_target() {
        let then_slain = [
            "You pierce a moss snake for 5 points of damage.",
            "a moss snake has been slain by Thenumberone!",
        ];
        assert!(!detect_active_combat(&then_slain));

        let no_target = ["You must first select a target for this spell!"];
        assert!(!detect_active_combat(&no_target));

        let empty: [&str; 0] = [];
        assert!(!detect_active_combat(&empty));
    }

    #[test]
    fn fight_signal_slain_wins_over_earlier_combat() {
        let summaries = [
            "A moss snake bites YOU for 5 points of damage.",
            "You pierce a moss snake for 5 points of damage.",
            "a moss snake has been slain by Thenumberone!",
        ];
        assert_eq!(classify_fight_signal(&summaries), FightSignal::Slain);
    }

    #[test]
    fn fight_signal_resist_and_fizzle_continue() {
        assert_eq!(
            classify_fight_signal(&["Your Blast of Cold spell fizzles!"]),
            FightSignal::Continue
        );
        assert_eq!(
            classify_fight_signal(&["A moss snake resisted your Blast of Cold!"]),
            FightSignal::Continue
        );
        assert_eq!(
            classify_fight_signal(&["You try to pierce a moss snake, but miss!"]),
            FightSignal::Continue
        );
        assert_eq!(
            classify_fight_signal(&["A moss snake bites YOU for 5 points of damage."]),
            FightSignal::Continue
        );
    }

    #[test]
    fn fight_signal_target_lost_when_fled_or_cleared() {
        assert_eq!(
            classify_fight_signal(&["A moss snake flees in terror."]),
            FightSignal::TargetLost
        );
        assert_eq!(
            classify_fight_signal(&["You have no target for this attack."]),
            FightSignal::TargetLost
        );
    }

    #[test]
    fn fight_signal_idle_when_no_combat_lines() {
        assert_eq!(
            classify_fight_signal(&["You begin to feel your mana returning."]),
            FightSignal::Idle
        );
        let empty: [&str; 0] = [];
        assert_eq!(classify_fight_signal(&empty), FightSignal::Idle);
    }

    #[test]
    fn tally_window_counts_resist_and_fizzle() {
        let window = vec![
            "You pierce a moss snake for 5 points of damage.".to_owned(),
            "A moss snake resisted your Blast of Cold!".to_owned(),
            "Your Blast of Cold spell fizzles!".to_owned(),
            "A moss snake bites YOU for 5 points of damage.".to_owned(),
        ];
        let tally = tally_window(&window);
        assert_eq!(tally.resisted, 1);
        assert_eq!(tally.fizzled, 1);
        assert!(tally.has_combat);

        let quiet = vec!["You begin to feel your mana returning.".to_owned()];
        let tally = tally_window(&quiet);
        assert_eq!(tally, CastTally::default());
    }

    #[test]
    fn stop_reason_success_classification() {
        assert!(StopReason::ReachedTargetLevel.is_success());
        assert!(StopReason::MaxIterations.is_success());
        assert!(!StopReason::HpFloor.is_success());
        assert!(!StopReason::OperatorPanic.is_success());
        assert!(!StopReason::ForegroundLost.is_success());
    }

    #[test]
    fn policy_clamps_bounds() {
        let policy = normalize_policy(ActAutocombatParams {
            max_iterations: 999,
            max_duration_s: 9999,
            hp_floor_percent: 200,
            mana_floor_percent: 200,
            target_level_max: 2,
            stop_at_level: 0,
            cast_mana_cost_percent: 250,
            engagement_timeout_s: 9999,
            hotbar_alias: "  HOTBAR4 ".to_owned(),
            max_roam_steps: 999,
            max_chase_s: 9999,
            idempotency_key: Some("run/with:bad chars!".to_owned()),
        });
        assert_eq!(policy.max_iterations, 50);
        assert_eq!(policy.max_duration.as_secs(), 600);
        assert_eq!(policy.hp_floor, 100);
        assert_eq!(policy.mana_floor, 100);
        assert_eq!(policy.stop_at_level, 1);
        assert_eq!(policy.cast_mana_cost, 100);
        assert_eq!(policy.engagement_timeout.as_secs(), 300);
        assert_eq!(policy.hotbar_alias, "hotbar4");
        assert_eq!(policy.max_roam_steps, 50);
        assert_eq!(policy.max_chase.as_secs(), 120);
        assert_eq!(policy.run_id, "runwithbadchars");
    }

    #[test]
    fn policy_defaults_for_new_params() {
        let policy = normalize_policy(ActAutocombatParams {
            max_iterations: default_max_iterations(),
            max_duration_s: default_max_duration_s(),
            hp_floor_percent: default_hp_floor(),
            mana_floor_percent: default_mana_floor(),
            target_level_max: default_target_level_max(),
            stop_at_level: default_stop_at_level(),
            cast_mana_cost_percent: default_cast_mana_cost(),
            engagement_timeout_s: default_engagement_timeout_s(),
            hotbar_alias: default_hotbar_alias(),
            max_roam_steps: default_max_roam_steps(),
            max_chase_s: default_max_chase_s(),
            idempotency_key: None,
        });
        assert_eq!(policy.cast_mana_cost, 70);
        assert_eq!(policy.engagement_timeout.as_secs(), 30);
        assert_eq!(policy.max_roam_steps, 6);
        assert_eq!(policy.max_chase.as_secs(), 12);
    }
}
