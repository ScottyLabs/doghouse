use super::*;
use nix::{
    sys::{
        signal::{SigSet, SigmaskHow},
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::{tcgetpgrp, tcsetpgrp},
};
use std::sync::atomic::{AtomicBool, Ordering};
fn foreground(tty: &File, group: Pid) -> Result<()> {
    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGTTOU);
    let previous = blocked.thread_swap_mask(SigmaskHow::SIG_BLOCK)?;
    let result = tcsetpgrp(tty, group);
    previous.thread_set_mask()?;
    result.context("Give the native agent the terminal")
}
struct TerminalOwner {
    tty: File,
    previous: Pid,
}
impl TerminalOwner {
    fn open() -> Result<Self> {
        let tty = File::open("/dev/tty")
            .context("An interactive terminal is required to open an agent")?;
        let previous = tcgetpgrp(&tty)?;
        Ok(Self { tty, previous })
    }
}
impl Drop for TerminalOwner {
    fn drop(&mut self) {
        let _ = foreground(&self.tty, self.previous);
    }
}
struct NativeProcess {
    child: Option<Child>,
    group: Pid,
}
impl Drop for NativeProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = killpg(self.group, Signal::SIGKILL);
            let _ = child.wait();
        }
    }
}
pub(super) struct Suspended {
    process: NativeProcess,
    leases: Vec<Lease>,
}
impl Suspended {
    pub(super) fn into_active(mut self) -> Result<Active> {
        let child = self
            .process
            .child
            .take()
            .context("Native agent process disappeared")?;
        let _ = killpg(self.process.group, Signal::SIGTERM);
        let _ = killpg(self.process.group, Signal::SIGCONT);
        Ok(Active {
            child,
            stopping: Some(Instant::now()),
            open_streams: 0,
            exited: None,
            exited_at: None,
            _leases: self.leases,
            native_terminal: true,
        })
    }
}
impl Manager {
    pub fn is_suspended(&self, id: &str) -> bool {
        self.suspended.contains_key(id)
    }
    pub fn interact(&mut self, id: &str, cancel: &AtomicBool) -> Result<()> {
        ensure!(
            self.interactive.is_none(),
            "A native conversation is already open"
        );
        ensure!(
            !self.active.contains_key(id),
            "Wait for the background run to stop before opening its conversation"
        );
        let terminal = TerminalOwner::open()?;
        let mut run = self.read_record(id)?;
        let session = if let Some(session) = self.suspended.remove(id) {
            session
        } else {
            ensure!(
                !run.state.active() && run.state != State::Unattached,
                "This run is owned by another window"
            );
            let leases = vec![
                lease(&self.root, &format!("checkout:{}", run.directory.display()))?,
                lease(
                    &self.root,
                    &format!("task:{}:{}", run.repository, run.task_id),
                )?,
            ];
            run = self.read_record(id)?;
            ensure!(
                !run.state.active() && run.state != State::Unattached,
                "This run is still active elsewhere"
            );
            ensure!(
                run.publication != "publishing",
                "Wait for result publication to finish"
            );
            ensure!(run.directory.is_dir(), "This checkout no longer exists");
            let mut command =
                backend::interactive_command(&run, &self.executables[run.provider.index()])?;
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .process_group(0);
            let child = command
                .spawn()
                .context("Open the native agent conversation")?;
            let group = Pid::from_raw(
                i32::try_from(child.id()).context("Invalid native agent process ID")?,
            );
            Suspended {
                process: NativeProcess {
                    child: Some(child),
                    group,
                },
                leases,
            }
        };
        let group = session.process.group;
        if let Err(error) = foreground(&terminal.tty, group) {
            drop(session);
            run.state = State::Failed;
            run.native_continued = true;
            run.failure = Some(format!("{error:#}"));
            self.runs.insert(id.into(), run);
            self.persist(id)?;
            return Err(error);
        }
        let _ = killpg(group, Signal::SIGCONT);
        let _ = killpg(group, Signal::SIGWINCH);
        run.state = State::Running;
        run.native_continued = true;
        run.event("Conversation","Opened in the native CLI. Ctrl+Z returns to Shepherd; exiting closes the native process.");
        self.runs.insert(id.into(), run);
        self.persist(id)?;
        self.interactive = Some(id.into());
        let result = (|| -> Result<bool> {
            let mut stopping = None;
            loop {
                match waitpid(
                    group,
                    Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED | WaitPidFlag::WCONTINUED),
                ) {
                    Ok(WaitStatus::Exited(_, code)) => {
                        ensure!(
                            code == 0 || code == 130,
                            "Native agent exited with status {code}"
                        );
                        return Ok(false);
                    }
                    Ok(WaitStatus::Signaled(
                        _,
                        Signal::SIGINT | Signal::SIGTERM | Signal::SIGHUP,
                        _,
                    )) => return Ok(false),
                    Ok(WaitStatus::Signaled(_, signal, _)) => {
                        bail!("Native agent exited after {signal}")
                    }
                    Ok(WaitStatus::Stopped(_, _)) if !cancel.load(Ordering::Relaxed) => {
                        return Ok(true);
                    }
                    Ok(WaitStatus::Stopped(_, _)) => {
                        let _ = killpg(group, Signal::SIGCONT);
                    }
                    Ok(_) => {}
                    Err(nix::errno::Errno::EINTR) => {}
                    Err(error) => return Err(error).context("Wait for the native conversation"),
                }
                if cancel.load(Ordering::Relaxed) {
                    if stopping.is_none() {
                        let _ = killpg(group, Signal::SIGTERM);
                        stopping = Some(Instant::now());
                    } else if stopping.is_some_and(|time| time.elapsed() > Duration::from_secs(3)) {
                        let _ = killpg(group, Signal::SIGKILL);
                    }
                }
                self.poll()?;
                std::thread::sleep(Duration::from_millis(100));
            }
        })();
        self.interactive = None;
        let paused = matches!(result, Ok(true));
        if paused {
            self.suspended.insert(id.into(), session);
        } else {
            drop(session);
        }
        if let Some(run) = self.runs.get_mut(id) {
            run.state = if paused {
                State::Suspended
            } else if result.is_ok() {
                State::Continued
            } else {
                State::Failed
            };
            run.failure = result.as_ref().err().map(|error| format!("{error:#}"));
            run.event(
                "Conversation",
                if paused {
                    "Native conversation paused. Enter resumes the same process; s stops it."
                } else {
                    "Returned to Shepherd. The native conversation and project files are retained."
                },
            );
        }
        self.persist(id)?;
        result.map(|_| ())
    }
}
