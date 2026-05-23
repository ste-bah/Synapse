#[cfg(windows)]
pub use platform::{
    NOTEPAD_POLL_INTERVAL, NOTEPAD_STARTUP_TIMEOUT, NOTEPAD_TITLE_REGEX, NotepadHandle,
    launch_notepad, wait_for_window_title_regex,
};
#[cfg(not(windows))]
pub use platform::{
    NOTEPAD_POLL_INTERVAL, NOTEPAD_STARTUP_TIMEOUT, NOTEPAD_TITLE_REGEX, NotepadHandle,
    launch_notepad, wait_for_window_title_regex,
};

#[cfg(windows)]
mod platform {
    use std::{
        path::PathBuf,
        process::{Child, Command, ExitStatus, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context, bail};
    use regex::Regex;
    use synapse_core::ForegroundContext;

    pub const NOTEPAD_TITLE_REGEX: &str = r"^Untitled - Notepad$";
    pub const NOTEPAD_POLL_INTERVAL: Duration = Duration::from_millis(20);
    pub const NOTEPAD_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

    const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(20);
    const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
    const FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

    pub struct NotepadHandle {
        child: Option<Child>,
        pid: u32,
        hwnd: i64,
    }

    impl NotepadHandle {
        #[must_use]
        pub const fn pid(&self) -> u32 {
            self.pid
        }

        #[must_use]
        pub const fn hwnd(&self) -> i64 {
            self.hwnd
        }

        pub fn current_foreground_context(&self) -> anyhow::Result<ForegroundContext> {
            synapse_a11y::foreground_context(self.hwnd).with_context(|| {
                format!("read foreground context for Notepad hwnd 0x{:x}", self.hwnd)
            })
        }

        pub fn close(mut self) -> anyhow::Result<()> {
            self.close_inner()
        }

        fn close_inner(&mut self) -> anyhow::Result<()> {
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };

            if child
                .try_wait()
                .context("check Notepad process status")?
                .is_some()
            {
                return Ok(());
            }

            let graceful = terminate_process_tree(self.pid, false);
            let graceful_exited = wait_for_child_exit(&mut child, GRACEFUL_CLOSE_TIMEOUT)
                .context("wait for Notepad graceful close")?;
            if graceful_exited {
                return Ok(());
            }

            let forced = terminate_process_tree(self.pid, true);
            let forced_exited = wait_for_child_exit(&mut child, FORCE_CLOSE_TIMEOUT)
                .context("wait for Notepad forced close")?;
            if forced_exited {
                return Ok(());
            }

            self.child = Some(child);
            let graceful_status =
                graceful.map_or_else(|err| err.to_string(), |status| status.to_string());
            let forced_status =
                forced.map_or_else(|err| err.to_string(), |status| status.to_string());
            bail!(
                "Notepad pid {} remained alive after taskkill /T ({graceful_status}) and taskkill /T /F ({forced_status})",
                self.pid
            );
        }
    }

    impl Drop for NotepadHandle {
        fn drop(&mut self) {
            let _ = self.close_inner();
        }
    }

    #[allow(clippy::trivial_regex)]
    pub fn launch_notepad() -> anyhow::Result<NotepadHandle> {
        let title_regex = Regex::new(NOTEPAD_TITLE_REGEX).context("compile Notepad title regex")?;
        let mut child = Command::new(notepad_exe())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn notepad.exe")?;
        let pid = child.id();
        if pid == 0 {
            let _ = child.kill();
            let _ = child.wait();
            bail!("spawned notepad.exe without a process id");
        }

        let context = match wait_for_window_title_regex(pid, &title_regex) {
            Ok(context) => context,
            Err(err) => {
                let _ = terminate_process_tree(pid, true);
                let _ = wait_for_child_exit(&mut child, FORCE_CLOSE_TIMEOUT);
                return Err(err).context("Notepad did not reach the expected startup title");
            }
        };

        Ok(NotepadHandle {
            child: Some(child),
            pid,
            hwnd: context.hwnd,
        })
    }

    pub fn wait_for_window_title_regex(
        pid: u32,
        title_regex: &Regex,
    ) -> anyhow::Result<ForegroundContext> {
        if pid == 0 {
            bail!("Notepad pid must be non-zero");
        }

        let start = Instant::now();
        let mut last_title: Option<String> = None;
        let mut last_error: Option<String> = None;

        while start.elapsed() <= NOTEPAD_STARTUP_TIMEOUT {
            match context_for_process(pid) {
                Ok(context) => {
                    if title_regex.is_match(&context.window_title) {
                        return Ok(context);
                    }
                    last_title = Some(context.window_title);
                    last_error = None;
                }
                Err(err) => {
                    last_error = Some(err.to_string());
                }
            }
            thread::sleep(NOTEPAD_POLL_INTERVAL);
        }

        bail!(
            "timed out after {:?} waiting for pid {pid} window title to match {}; last_title={last_title:?}; last_error={last_error:?}",
            NOTEPAD_STARTUP_TIMEOUT,
            title_regex.as_str()
        );
    }

    fn context_for_process(pid: u32) -> anyhow::Result<ForegroundContext> {
        let window = synapse_a11y::window_for_process(pid)
            .with_context(|| format!("find window for pid {pid}"))?;
        let tree = synapse_a11y::snapshot(&window, 0)
            .with_context(|| format!("snapshot window root for pid {pid}"))?;
        let parts = tree
            .root
            .parts()
            .with_context(|| format!("parse root element id {}", tree.root))?;
        synapse_a11y::foreground_context(parts.hwnd).with_context(|| {
            format!(
                "read foreground context for pid {pid} hwnd 0x{:x}",
                parts.hwnd
            )
        })
    }

    fn notepad_exe() -> PathBuf {
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            let candidate = PathBuf::from(system_root)
                .join("System32")
                .join("notepad.exe");
            if candidate.exists() {
                return candidate;
            }
        }
        PathBuf::from("notepad.exe")
    }

    fn terminate_process_tree(pid: u32, force: bool) -> anyhow::Result<ExitStatus> {
        let pid_arg = pid.to_string();
        let mut command = Command::new("taskkill");
        command
            .args(["/PID", &pid_arg, "/T"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if force {
            command.arg("/F");
        }
        command.status().context("run taskkill")
    }

    fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> anyhow::Result<bool> {
        let start = Instant::now();
        while start.elapsed() <= timeout {
            if child
                .try_wait()
                .context("check child process status")?
                .is_some()
            {
                return Ok(true);
            }
            thread::sleep(CLOSE_POLL_INTERVAL);
        }
        Ok(false)
    }
}

#[cfg(not(windows))]
mod platform {
    use std::time::Duration;

    use anyhow::bail;
    use regex::Regex;
    use synapse_core::ForegroundContext;

    pub const NOTEPAD_TITLE_REGEX: &str = r"^Untitled - Notepad$";
    pub const NOTEPAD_POLL_INTERVAL: Duration = Duration::from_millis(20);
    pub const NOTEPAD_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

    pub struct NotepadHandle {
        _private: (),
    }

    impl NotepadHandle {
        #[must_use]
        pub const fn pid(&self) -> u32 {
            0
        }

        #[must_use]
        pub const fn hwnd(&self) -> i64 {
            0
        }

        pub fn current_foreground_context(&self) -> anyhow::Result<ForegroundContext> {
            bail!("launch_notepad foreground readback requires Windows UI Automation");
        }

        pub const fn close(self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    pub fn launch_notepad() -> anyhow::Result<NotepadHandle> {
        bail!("launch_notepad requires Windows");
    }

    pub fn wait_for_window_title_regex(
        _pid: u32,
        _title_regex: &Regex,
    ) -> anyhow::Result<ForegroundContext> {
        bail!("wait_for_window_title_regex requires Windows");
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use regex::Regex;

    #[cfg(not(windows))]
    use super::wait_for_window_title_regex;
    use super::{
        NOTEPAD_POLL_INTERVAL, NOTEPAD_STARTUP_TIMEOUT, NOTEPAD_TITLE_REGEX, launch_notepad,
    };

    #[test]
    fn notepad_fixture_constants_match_m2_contract() {
        println!(
            "source_of_truth=fixtures_constants before=expected_poll_ms:20 expected_timeout_s:5 expected_regex:{NOTEPAD_TITLE_REGEX:?}"
        );
        assert_eq!(NOTEPAD_POLL_INTERVAL.as_millis(), 20);
        assert_eq!(NOTEPAD_STARTUP_TIMEOUT.as_secs(), 5);
        assert_eq!(NOTEPAD_TITLE_REGEX, r"^Untitled - Notepad$");
        println!(
            "source_of_truth=fixtures_constants after=poll_ms:{} timeout_s:{} regex:{NOTEPAD_TITLE_REGEX:?}",
            NOTEPAD_POLL_INTERVAL.as_millis(),
            NOTEPAD_STARTUP_TIMEOUT.as_secs()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn launch_notepad_fails_closed_off_windows() {
        println!("source_of_truth=launch_notepad_non_windows before=handle_present:false");
        let error = match launch_notepad() {
            Ok(_handle) => panic!("launch_notepad unexpectedly succeeded off Windows"),
            Err(error) => error,
        };
        println!("source_of_truth=launch_notepad_non_windows after=error:{error}");
        assert!(error.to_string().contains("requires Windows"));
    }

    #[cfg(not(windows))]
    #[allow(clippy::trivial_regex)]
    #[test]
    fn wait_for_window_title_regex_fails_closed_off_windows() {
        let regex = match Regex::new(NOTEPAD_TITLE_REGEX) {
            Ok(regex) => regex,
            Err(error) => panic!("failed to compile Notepad title regex: {error}"),
        };
        println!(
            "source_of_truth=wait_for_window_title_regex_non_windows before=pid:1 regex:{:?}",
            regex.as_str()
        );
        let error = match wait_for_window_title_regex(1, &regex) {
            Ok(_context) => {
                panic!("wait_for_window_title_regex unexpectedly succeeded off Windows")
            }
            Err(error) => error,
        };
        println!("source_of_truth=wait_for_window_title_regex_non_windows after=error:{error}");
        assert!(error.to_string().contains("requires Windows"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an interactive Windows desktop with Notepad and UIA"]
    fn launch_notepad_reaches_expected_window_title_on_windows() -> anyhow::Result<()> {
        println!(
            "source_of_truth=synapse_a11y::foreground_context(hwnd).window_title before=notepad_absent_or_unobserved"
        );
        let handle = launch_notepad()?;
        let context = handle.current_foreground_context()?;
        println!(
            "source_of_truth=synapse_a11y::foreground_context hwnd={} pid={} after_title={:?}",
            handle.hwnd(),
            handle.pid(),
            context.window_title
        );
        assert_eq!(context.pid, handle.pid());
        assert_eq!(context.hwnd, handle.hwnd());
        assert_eq!(context.window_title, "Untitled - Notepad");

        let pid = handle.pid();
        handle.close()?;
        println!("source_of_truth=NotepadHandle::close after=closed pid={pid}");
        Ok(())
    }
}
