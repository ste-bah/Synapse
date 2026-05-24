use super::M3ToolStub;

#[must_use]
pub const fn profile_list() -> M3ToolStub {
    M3ToolStub::new("profile_list")
}

#[must_use]
pub const fn profile_activate() -> M3ToolStub {
    M3ToolStub::new("profile_activate")
}
