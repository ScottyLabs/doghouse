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
pub(super) struct Console {
    socket: PathBuf,
    _temporary: tempfile::TempDir,
    _leases: Vec<Lease>,
}
impl Console {
    fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        command.arg("-S").arg(&self.socket).env_remove("TMUX");
        command
    }
    fn status(&self) -> Result<Option<i32>> {
        let output = self
            .command()
            .args([
                "display-message",
                "-p",
                "-t",
                "agent:0.0",
                "#{pane_dead}:#{pane_dead_status}",
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "Native terminal session disappeared"
        );
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(code) = text.trim().strip_prefix("1:") {
            return Ok(Some(code.parse().unwrap_or(1)));
        }
        Ok(None)
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
    }
}
impl Manager {
    pub fn is_detached(&self, id: &str) -> bool {
        self.consoles.contains_key(id)
    }
    pub(super) fn poll_consoles(&mut self) -> Result<()> {
        let finished = self
            .consoles
            .iter()
            .filter_map(|(id, console)| match console.status() {
                Ok(None) => None,
                result => Some((id.clone(), result)),
            })
            .collect::<Vec<_>>();
        for (id, result) in finished {
            self.consoles.remove(&id);
            if let Some(run) = self.runs.get_mut(&id) {
                run.state = if matches!(result, Ok(Some(0 | 130))) {
                    State::Continued
                } else {
                    State::Failed
                };
                run.failure = match result {
                    Ok(Some(0 | 130)) => None,
                    Ok(code) => Some(format!("Native conversation exited: {code:?}")),
                    Err(error) => Some(format!("{error:#}")),
                };
                run.event(
                    "Conversation",
                    "Native conversation ended; files and session history are retained",
                );
            }
            self.persist(&id)?;
        }
        Ok(())
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
        if !self.consoles.contains_key(id) {
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
            let native =
                backend::interactive_command(&run, &self.executables[run.provider.index()])?;
            let temporary = tempfile::Builder::new()
                .prefix("shepherd-console-")
                .tempdir()?;
            let config = temporary.path().join("tmux.conf");
            fs::write(
                &config,
                "set -g status off\nset -g prefix None\nset -g history-limit 2000\nset -g remain-on-exit on\nset -g destroy-unattached off\nbind-key -n C-z detach-client\n",
            )?;
            let console = Console {
                socket: temporary.path().join("socket"),
                _temporary: temporary,
                _leases: leases,
            };
            let (width, height) = crossterm::terminal::size()?;
            let output = console.command().arg("-f").arg(config)
                .args(["new-session", "-d", "-s", "agent", "-c"]).arg(&run.directory)
                .arg("-x").arg(width.to_string()).arg("-y").arg(height.to_string())
                .arg("--").arg(native.get_program()).args(native.get_args()).output()
                .context("Open native terminal (tmux is required; use the packaged Shepherd or nix develop)")?;
            ensure!(
                output.status.success(),
                "Open native terminal: {}",
                clean(&String::from_utf8_lossy(&output.stderr))
            );
            self.consoles.insert(id.into(), console);
        }
        run.state = State::Running;
        run.native_continued = true;
        run.event(
            "Conversation",
            "Opened native CLI; Ctrl+Z detaches while work continues",
        );
        self.runs.insert(id.into(), run);
        self.persist(id)?;
        let console = self
            .consoles
            .get(id)
            .context("Native terminal disappeared")?;
        let child = console
            .command()
            .args(["attach-session", "-t", "agent"])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .context("Attach native terminal")?;
        let group = Pid::from_raw(i32::try_from(child.id())?);
        let mut client = NativeProcess {
            child: Some(child),
            group,
        };
        foreground(&terminal.tty, group)?;
        let _ = killpg(group, Signal::SIGCONT);
        self.interactive = Some(id.into());
        let result = (|| -> Result<()> {
            loop {
                match waitpid(group, Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED)) {
                    Ok(WaitStatus::Exited(_, code)) => {
                        client.child.take();
                        ensure!(
                            code == 0 || !self.consoles.contains_key(id),
                            "Native terminal attachment exited with status {code}; the agent remains in the background. Press Enter to retry"
                        );
                        break;
                    }
                    Ok(WaitStatus::Signaled(_, _, _)) => {
                        client.child.take();
                        break;
                    }
                    Ok(WaitStatus::Stopped(_, _)) => {
                        let _ = killpg(group, Signal::SIGCONT);
                    }
                    Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                    Err(error) => return Err(error).context("Wait for native terminal"),
                }
                if cancel.load(Ordering::Relaxed) {
                    let _ = killpg(group, Signal::SIGTERM);
                }
                self.poll()?;
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        })();
        self.interactive = None;
        self.poll_consoles()?;
        if self.consoles.contains_key(id) {
            if let Some(run) = self.runs.get_mut(id) {
                run.state = State::Detached;
                run.event("Conversation", "Detached; agent continues in background. Enter returns to the same terminal; s stops it.");
            }
            self.persist(id)?;
        }
        result
    }
}
