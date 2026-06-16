//! Routine setup-plan tools (#859) — own router, merged in `server.rs`.
//!
//! `routine_compile_plan` compiles a mined routine into an inspectable setup
//! plan (and persists it); `plan_get` reads a stored plan. Thin wrappers around
//! [`crate::m3::plan`].

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};

use crate::m3::plan::{
    PlanGetParams, PlanGetResponse, RoutineCompilePlanParams, RoutineCompilePlanResponse,
    compile_routine_plan, get_plan, required_permissions_compile, required_permissions_get,
};

#[tool_router(router = plan_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Compile a mined routine into an executable SETUP plan (#859): each template step becomes a plan step with an action backend (act_launch for apps, cdp_open_tab for background browser tabs, shell-open for documents) and an explicit POSTCONDITION the executor (#860) must verify against the physical SoT before the next step — the no-silent-success doctrine. Steps that need judgment (a non-browser app remembered only by a window title, or a browser page title with no resolvable URL host) degrade to an agent_task stub and are NEVER silently dropped. Returns the plan document (steps/backends/postconditions, deterministic vs agent-task counts, fully_deterministic flag) and persists it to CF_KV (plan/v1/<routine_id>) unless store=false. ROUTINE_NOT_MINED if the id has no CF_ROUTINES template."
    )]
    pub async fn routine_compile_plan(
        &self,
        params: Parameters<RoutineCompilePlanParams>,
    ) -> Result<Json<RoutineCompilePlanResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "routine_compile_plan",
            routine_id = %params.0.routine_id,
            store = params.0.store,
            "tool.invocation kind=routine_compile_plan"
        );
        self.require_m3_permissions(
            "routine_compile_plan",
            &required_permissions_compile(&params.0),
        )?;
        let db = self.m3_storage()?;
        compile_routine_plan(&db, &params.0).map(Json)
    }

    #[tool(
        description = "Read the stored setup plan for a routine (#859), compiled by routine_compile_plan and persisted in CF_KV. Returns found=false when no plan has been compiled yet. Read-only."
    )]
    pub async fn plan_get(
        &self,
        params: Parameters<PlanGetParams>,
    ) -> Result<Json<PlanGetResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "plan_get",
            routine_id = %params.0.routine_id,
            "tool.invocation kind=plan_get"
        );
        self.require_m3_permissions("plan_get", &required_permissions_get(&params.0))?;
        let db = self.m3_storage()?;
        get_plan(&db, &params.0).map(Json)
    }
}
