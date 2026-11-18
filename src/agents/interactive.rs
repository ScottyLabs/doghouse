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
    _leases: Vec<Lease>,
}
impl Console {
    fn command(&self) -> Command {
        let mut command = Command::new("tmux");
        command.arg("-S").arg(&self.socket).env_remove("TMUX");
        command
    }
    fn status(&self) -> Result<Option<i32>> {
        console_status(&self.socket)
    }
}
pub(super) fn console_status(socket: &std::path::Path) -> Result<Option<i32>> {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .env_remove("TMUX")
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

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.command().arg("kill-server").output();
        let _ = fs::remove_file(&self.socket);
    }
}
impl Manager {
    pub(super) fn start_console(&mut self, id: &str, width: u16, height: u16) -> Result<()> {
        let run = self.read_record(id)?;
        let leases = self.execution_leases(&run)?;
        let mut run = self.read_record(id)?;
        ensure!(
            (!run.state.active() && run.state != State::Unattached) || self.console_available(id),
            "This session is still working; stop its current turn first"
        );
        ensure!(
            run.publication != "publishing",
            "Wait for result publication to finish"
        );
        ensure!(run.directory.is_dir(), "This checkout no longer exists");
        let native = backend::interactive_command(&run, &self.executables[run.provider.index()])?;
        let config = self.root.join(id).join("tmux.conf");
        write_console_config(&config, console_configuration(run.provider).as_bytes())?;
        let console = Console {
            socket: self.console_socket(id),
            _leases: leases,
        };
        if !self.console_available(id) {
            if self.console_socket(id).exists() {
                fs::remove_file(self.console_socket(id))?;
            }
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
        }
        self.consoles.insert(id.into(), console);
        run.state = State::Detached;
        run.native_continued = true;
        run.event("Conversation", "Native conversation available. Press o to open it; Ctrl+Z returns; quitting Shepherd keeps it open.");
        self.runs.insert(id.into(), run);
        self.persist(id)?;
        Ok(())
    }

    pub fn is_detached(&self, id: &str) -> bool {
        self.console_available(id)
    }
    fn console_socket(&self, id: &str) -> PathBuf {
        self.root
            .join(id)
            .canonicalize()
            .unwrap_or_else(|_| self.root.join(id))
            .join("terminal.socket")
    }
    fn attach_command(&self, id: &str) -> Command {
        let mut command = Command::new("tmux");
        command
            .arg("-S")
            .arg(self.console_socket(id))
            .env_remove("TMUX")
            .args(["attach-session", "-t", "agent"]);
        command
    }
    pub(super) fn console_available(&self, id: &str) -> bool {
        valid_id(id)
            && self.console_socket(id).exists()
            && matches!(console_status(&self.console_socket(id)), Ok(None))
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
        let shared =
            self.detached || (self.console_available(id) && !self.consoles.contains_key(id));
        worker::reconcile(&self.root, &mut run)?;
        if self.detached
            && run.state != State::Unattached
            && self.console_available(id)
            && !worker::live(&self.root, id)?
        {
            let (width, height) = crossterm::terminal::size()?;
            worker::launch(self, id, None, width, height)?;
        }
        if !self.console_available(id) {
            if self.detached {
                let (width, height) = crossterm::terminal::size()?;
                worker::launch(self, id, None, width, height)?;
            } else {
                let (width, height) = crossterm::terminal::size()?;
                self.start_console(id, width, height)?;
            }
        }
        let config = self.root.join(id).join("tmux.conf");
        write_console_config(&config, console_configuration(run.provider).as_bytes())?;
        let output = Command::new("tmux")
            .arg("-S")
            .arg(self.console_socket(id))
            .env_remove("TMUX")
            .arg("source-file")
            .arg(config)
            .output()?;
        ensure!(
            output.status.success(),
            "Could not update conversation controls: {}",
            clean(&String::from_utf8_lossy(&output.stderr))
        );
        if !shared {
            run.state = State::Running;
            run.native_continued = true;
            run.event(
                "Conversation",
                "Opened native CLI; Ctrl+Z detaches while work continues",
            );
            self.runs.insert(id.into(), run);
            self.persist(id)?;
        }
        let child = self
            .attach_command(id)
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
                            code == 0 || !self.console_available(id),
                            "Native terminal attachment exited with status {code}; the agent remains in the background. Press o to retry"
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
        if !shared && self.consoles.contains_key(id) {
            if let Some(run) = self.runs.get_mut(id) {
                run.state = State::Detached;
                run.event("Conversation", "Detached; agent continues in background. o returns to the same terminal; x confirms stopping the session.");
            }
            self.persist(id)?;
        }
        result
    }
}

fn console_configuration(provider: Provider) -> String {
    format!(
        "set -g status on\nset -g status-style 'fg=default,bg=default'\nset -g status-format[0] ' Shepherd | Ctrl+Z: return | {}: subagents '\nset -g prefix None\nset -g history-limit 2000\nset -g remain-on-exit on\nset -g destroy-unattached off\nbind-key -n C-z detach-client\n",
        provider.subagent_menu()
    )
}
fn write_console_config(path: &std::path::Path, content: &[u8]) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing console directory")?)?;
    file.write_all(content)?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_discover_and_stop_the_same_terminal() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let mut owner = Manager::open(temporary.path())?;
        owner.detached = false;
        let run = Run::fixture(Provider::Codex);
        fs::create_dir_all(owner.root.join(&run.id))?;
        owner.runs.insert(run.id.clone(), run.clone());
        owner.persist(&run.id)?;
        let console = Console {
            socket: owner.console_socket(&run.id),
            _leases: vec![lease(&owner.root, "test-terminal")?],
        };
        let output = console
            .command()
            .args([
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "agent",
                "--",
                "sleep",
                "30",
            ])
            .output()?;
        ensure!(
            output.status.success(),
            "Start fixture tmux: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        owner.consoles.insert(run.id.clone(), console);
        let mut viewer = Manager::open(temporary.path())?;
        viewer.detached = false;
        assert!(viewer.is_detached(&run.id));
        assert_eq!(viewer.get(&run.id)?.state, State::Detached);
        let first_tty = nix::pty::openpty(None, None)?;
        let second_tty = nix::pty::openpty(None, None)?;
        let attach = |manager: &Manager, slave: std::os::fd::OwnedFd| -> Result<NativeProcess> {
            let slave = File::from(slave);
            let child = manager
                .attach_command(&run.id)
                .env("TERM", "xterm-256color")
                .stdin(slave.try_clone()?)
                .stdout(slave.try_clone()?)
                .stderr(slave)
                .process_group(0)
                .spawn()?;
            let group = Pid::from_raw(i32::try_from(child.id())?);
            Ok(NativeProcess {
                child: Some(child),
                group,
            })
        };
        let first = attach(&owner, first_tty.slave)?;
        let second = attach(&viewer, second_tty.slave)?;
        let clients = || -> Result<Vec<String>> {
            let output = owner.consoles[&run.id]
                .command()
                .args(["list-clients", "-F", "#{client_name}"])
                .output()?;
            ensure!(output.status.success(), "List fixture clients");
            Ok(String::from_utf8(output.stdout)?
                .lines()
                .map(str::to_owned)
                .collect())
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while clients()?.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            clients()?.len(),
            2,
            "Both windows should attach simultaneously"
        );
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(3);
        while clients()?.len() != 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            clients()?.len(),
            1,
            "Detaching one client retains the other"
        );
        drop(second);
        drop(viewer);
        assert!(owner.consoles[&run.id].status()?.is_none());
        viewer = Manager::open(temporary.path())?;
        viewer.detached = false;
        viewer.stop(&run.id)?;
        owner.poll()?;
        viewer.refresh_external()?;
        assert_eq!(viewer.get(&run.id)?.state, State::Stopped);
        assert!(!viewer.is_detached(&run.id));
        assert!(!owner.root.join(&run.id).join("stop-request").exists());
        Ok(())
    }
}
