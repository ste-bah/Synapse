mod cancel;
mod common;
mod history;
mod list;
mod register;

#[cfg(test)]
mod tests;

pub use cancel::{
    ReflexCancelParams, ReflexCancelResponse, cancel_reflex, required_permissions_cancel,
};
pub use history::{
    ReflexHistoryParams, ReflexHistoryResponse, history_reflexes, required_permissions_history,
};
pub use list::{ReflexListParams, ReflexListResponse, list_reflexes, required_permissions_list};
pub use register::{
    ReflexRegisterParams, ReflexRegisterResponse, register_reflex, required_permissions_register,
};

use super::M3ToolStub;

#[must_use]
pub const fn reflex_register() -> M3ToolStub {
    M3ToolStub::new("reflex_register")
}

#[must_use]
pub const fn reflex_cancel() -> M3ToolStub {
    M3ToolStub::new("reflex_cancel")
}

#[must_use]
pub const fn reflex_list() -> M3ToolStub {
    M3ToolStub::new("reflex_list")
}

#[must_use]
pub const fn reflex_history() -> M3ToolStub {
    M3ToolStub::new("reflex_history")
}
