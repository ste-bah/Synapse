use super::M3ToolStub;

#[must_use]
pub const fn audio_tail() -> M3ToolStub {
    M3ToolStub::new("audio_tail")
}

#[must_use]
pub const fn audio_transcribe() -> M3ToolStub {
    M3ToolStub::new("audio_transcribe")
}
