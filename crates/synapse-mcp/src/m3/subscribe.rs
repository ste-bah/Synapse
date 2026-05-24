use super::M3ToolStub;

#[must_use]
pub const fn subscribe() -> M3ToolStub {
    M3ToolStub::new("subscribe")
}

#[must_use]
pub const fn subscribe_cancel() -> M3ToolStub {
    M3ToolStub::new("subscribe_cancel")
}
