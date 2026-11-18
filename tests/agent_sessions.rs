use serde_json::Value;
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_shepherd");

struct Fixture {
    temporary: tempfile::TempDir,
    board: PathBuf,
    registry: PathBuf,
    project: String,
    executable: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        // Unix socket paths are limited; nested Nix TMPDIRs can exceed that limit.
        let temporary = tempfile::Builder::new()
            .prefix("shepherd-")
            .tempdir_in("/tmp")
            .unwrap();
        let board = temporary.path().join("board");
        let registry = temporary.path().join("registry");
        let project_directory = temporary.path().join("project");
        fs::create_dir(&project_directory).unwrap();
        let executable = temporary.path().join("fixture-agent");
        fs::write(&executable, r#"#!/bin/sh
case "$1" in
exec)
    cat >/dev/null
    printf '%s\n' '{"type":"thread.started","thread_id":"fixture-session"}'
    printf '%s\n' '{"type":"item.completed","item":{"type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"fixture-session","receiver_thread_ids":["child-fixture"],"prompt":"Review child fixtures","agents_states":{"child-fixture":{"status":"running","message":"Checking fixtures"}}}}'
    while [ ! -f "$SHEPHERD_TEST_FINISH" ]; do sleep 0.05; done
    printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","text":"Fixture finished"}}' '{"type":"turn.completed","usage":{"input_tokens":5,"output_tokens":3}}'
    ;;
resume)
    echo 'NATIVE-CONVERSATION-READY'
    count=0
    while read -r line; do count=$((count + 1)); echo "REPLY:$line:$count"; done
    ;;
esac
"#).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut fixture = Self {
            temporary,
            board,
            registry,
            project: String::new(),
            executable,
        };
        let output = fixture
            .command()
            .args(["local", "add"])
            .arg(project_directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fixture.project = serde_json::from_slice::<Value>(&output.stdout).unwrap()["id"]
            .as_str()
            .unwrap()
            .into();
        let output = fixture
            .command()
            .args(["local", "task", &fixture.project, "Durable session fixture"])
            .output()
            .unwrap();
        assert!(output.status.success());
        fixture
    }
    fn command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .arg("--data-dir")
            .arg(&self.board)
            .env("XDG_CACHE_HOME", self.temporary.path().join("legacy-cache"))
            .env("SHEPHERD_AGENT_DIR", &self.registry)
            .env("SHEPHERD_CODEX", &self.executable)
            .env("SHEPHERD_TEST_FINISH", self.temporary.path().join("finish"));
        command
    }
    fn client(&self, board: Option<&Path>) -> Client {
        let mut command = self.command();
        if let Some(board) = board {
            command = Command::new(BINARY);
            command
                .arg("--data-dir")
                .arg(board)
                .arg("--offline")
                .env("XDG_CACHE_HOME", self.temporary.path().join("legacy-cache"))
                .env("SHEPHERD_AGENT_DIR", &self.registry)
                .env("SHEPHERD_CODEX", &self.executable)
                .env("SHEPHERD_TEST_FINISH", self.temporary.path().join("finish"));
        } else {
            command.args(["--workspace", "@local", "--project", &self.project]);
        }
        Client::open(command)
    }
    fn record(&self) -> (PathBuf, Value) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(entries) = fs::read_dir(self.registry.join("agents")) {
                for entry in entries.flatten() {
                    let path = entry.path().join("run.json");
                    if let Ok(bytes) = fs::read(&path) {
                        let record: Value = serde_json::from_slice(&bytes).unwrap();
                        if record["session_id"] == "fixture-session" {
                            return (path, record);
                        }
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "No saved native session appeared"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn launch(&self) -> (Client, PathBuf) {
        let mut client = self.client(None);
        client.wait_for("Durable session fixture");
        client.send(b"A");
        client.wait_for("Enter start");
        client.send(b"Inspect this fixture\r");
        let (path, _) = self.record();
        (client, path)
    }
    fn wait_state(path: &Path, expected: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            if record["state"] == expected {
                return record;
            }
            assert!(
                Instant::now() < deadline,
                "Expected {expected}, got {}",
                record["state"]
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::write(self.temporary.path().join("finish"), "done");
        if let Ok(entries) = fs::read_dir(self.registry.join("agents")) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && entry.path().join("run.json").is_file()
                {
                    let _ = fs::write(entry.path().join("stop-request"), b"stop\n");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}
struct Client {
    child: Child,
    input: File,
    output: Receiver<Vec<u8>>,
    text: Vec<u8>,
}
impl Client {
    fn open(mut command: Command) -> Self {
        let size = nix::pty::Winsize {
            ws_row: 35,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let terminal = nix::pty::openpty(Some(&size), None).unwrap();
        let slave = File::from(terminal.slave);
        let input = File::from(terminal.master);
        command
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe session and terminal syscalls run after fork.
        unsafe {
            command.pre_exec(|| {
                nix::unistd::setsid().map_err(std::io::Error::from)?;
                if nix::libc::ioctl(0, nix::libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let (sender, output) = mpsc::channel();
        let mut reader = input.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            let mut query = Vec::new();
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                query.extend_from_slice(&buffer[..count]);
                if query.windows(4).any(|bytes| bytes == b"\x1b[6n") {
                    let _ = reader.write_all(b"\x1b[1;1R");
                    query.clear();
                } else if query.len() > 4 {
                    query.drain(..query.len() - 4);
                }
                if sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
            text: Vec::new(),
        }
    }
    fn send(&mut self, text: &[u8]) {
        self.input.write_all(text).unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }
    fn wait_for(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if String::from_utf8_lossy(&self.text).contains(text) {
                self.text.clear();
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Terminal did not show {text}: {}",
                String::from_utf8_lossy(&self.text[self.text.len().saturating_sub(4000)..])
            );
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(50)) {
                self.text.extend(bytes);
            }
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn quitting_and_killing_clients_preserves_workers_and_another_board_can_stop_them() {
    for graceful in [true, false] {
        let fixture = Fixture::new();
        let (mut client, path) = fixture.launch();
        if graceful {
            client.send(b"q");
            assert!(client.child.wait().unwrap().success());
        }
        drop(client);
        std::thread::sleep(Duration::from_millis(150));
        let record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(record["state"], "running");
        let mut viewer = fixture.client(Some(&fixture.temporary.path().join("other-board")));
        viewer.send(b"a");
        viewer.wait_for("Durable session fixture");
        viewer.send(b"x");
        viewer.wait_for("Stop session and all subagents?");
        viewer.send(b"\r");
        Fixture::wait_state(&path, "stopped");
    }
}

#[test]
fn completed_session_can_open_and_detach_native_conversation_across_client_exit() {
    let fixture = Fixture::new();
    let (client, path) = fixture.launch();
    drop(client);
    fs::write(fixture.temporary.path().join("finish"), "done").unwrap();
    Fixture::wait_state(&path, "completed");
    let mut viewer = fixture.client(None);
    viewer.wait_for("Durable session fixture");
    viewer.send(b"a");
    viewer.wait_for("Space Actions");
    viewer.send(b"\r");
    viewer.wait_for("NATIVE-CONVERSATION-READY");
    viewer.send(b"part");
    viewer.send(&[26]);
    viewer.wait_for("Conversation is open");
    viewer.send(b"q");
    assert!(viewer.child.wait().unwrap().success());
    drop(viewer);
    let mut next = fixture.client(None);
    next.wait_for("Durable session fixture");
    next.send(b"a");
    next.wait_for("Space Actions");
    next.send(b"o");
    next.send(b"ial\r");
    next.wait_for("REPLY:partial:1");
    drop(next);
    let mut attached_again = fixture.client(None);
    attached_again.wait_for("Durable session fixture");
    attached_again.send(b"a");
    attached_again.wait_for("Space Actions");
    attached_again.send(b"o");
    attached_again.send(b"again\r");
    attached_again.wait_for("REPLY:again:2");
    attached_again.send(&[26]);
    attached_again.send(b"x\r");
    Fixture::wait_state(&path, "stopped");
}

#[test]
fn worker_interruption_preserves_files_and_requires_confirmation_to_recover() {
    let fixture = Fixture::new();
    let (client, path) = fixture.launch();
    drop(client);
    let note = fixture
        .temporary
        .path()
        .join("project")
        .join("unsaved-work.txt");
    fs::write(&note, "Keep this work").unwrap();
    let metadata: Value =
        serde_json::from_slice(&fs::read(path.parent().unwrap().join("worker.json")).unwrap())
            .unwrap();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(metadata["pid"].as_u64().unwrap()).unwrap()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    Fixture::wait_state(&path, "interrupted");
    let mut viewer = fixture.client(None);
    viewer.wait_for("Durable session fixture");
    viewer.send(b"a");
    viewer.wait_for("Space Actions");
    viewer.send(b"o");
    viewer.wait_for("Recover interrupted conversation?");
    assert!(!path.parent().unwrap().join("terminal.socket").exists());
    viewer.send(b"\r");
    viewer.wait_for("NATIVE-CONVERSATION-READY");
    assert_eq!(fs::read_to_string(note).unwrap(), "Keep this work");
    viewer.send(&[26]);
    viewer.send(b"x\r");
    Fixture::wait_state(&path, "stopped");
}

#[test]
fn killed_worker_cannot_allow_a_second_agent_to_use_the_orphaned_checkout() {
    let fixture = Fixture::new();
    let (client, path) = fixture.launch();
    drop(client);
    let metadata: Value =
        serde_json::from_slice(&fs::read(path.parent().unwrap().join("worker.json")).unwrap())
            .unwrap();
    let worker =
        nix::unistd::Pid::from_raw(i32::try_from(metadata["pid"].as_u64().unwrap()).unwrap());
    nix::sys::signal::kill(worker, nix::sys::signal::Signal::SIGKILL).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut viewer = fixture.client(None);
    viewer.wait_for("Durable session fixture");
    viewer.send(b"a");
    viewer.wait_for("Working in another process");
    // End the fixture agent and its inherited checkout lease before trying recovery.
    let agent =
        nix::unistd::Pid::from_raw(i32::try_from(metadata["agent_pid"].as_u64().unwrap()).unwrap());
    nix::sys::signal::killpg(agent, nix::sys::signal::Signal::SIGTERM).unwrap();
    viewer.wait_for("Interrupted");
    viewer.send(b"o");
    viewer.wait_for("Recover interrupted conversation?");
    viewer.send(b"\x1b");
}

#[test]
fn legacy_session_records_are_registered_without_moving_their_files() {
    let fixture = Fixture::new();
    let (client, path) = fixture.launch();
    drop(client);
    fs::write(fixture.temporary.path().join("finish"), "done").unwrap();
    let record = Fixture::wait_state(&path, "completed");
    std::thread::sleep(Duration::from_millis(100));
    let identifier = record["id"].as_str().unwrap();
    let legacy = fixture.board.join("agents").join(identifier);
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::rename(path.parent().unwrap(), &legacy).unwrap();
    let mut viewer = fixture.client(None);
    viewer.wait_for("Durable session fixture");
    viewer.send(b"a");
    viewer.wait_for("Space Actions");
    let alias = fixture.registry.join("agents").join(identifier);
    assert!(alias.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        alias.canonicalize().unwrap(),
        legacy.canonicalize().unwrap()
    );
    let saved: Value = serde_json::from_slice(&fs::read(alias.join("run.json")).unwrap()).unwrap();
    assert_eq!(saved["directory"], record["directory"]);
    assert_eq!(saved["session_id"], "fixture-session");
}

#[test]
fn subagents_are_shared_and_inspecting_them_or_refreshing_never_stops_parent() {
    let fixture = Fixture::new();
    let (mut client, path) = fixture.launch();
    client.send(b"\x1b[C");
    client.wait_for("Review child fixtures");
    client.send(b"\r");
    client.wait_for("child-fixture");
    client.send(b"x");
    let record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(record["state"], "running");
    assert_eq!(record["subagents"][0]["id"], "child-fixture");
    assert!(!path.parent().unwrap().join("stop-request").exists());
    client.send(b"\x1b");
    client.send(b"\x1b");
    client.send(b"s");
    client.send(b" ");
    client.wait_for("Session actions");
    assert!(!path.parent().unwrap().join("stop-request").exists());
    drop(client);
    let mut viewer = fixture.client(Some(&fixture.temporary.path().join("other-board")));
    viewer.send(b"a");
    viewer.wait_for("Durable session fixture");
    viewer.send(b"\x1b[C");
    viewer.wait_for("Review child fixtures");
    viewer.send(b"\x1b");
    viewer.send(b"x");
    viewer.wait_for("Stop session and all subagents?");
    viewer.send(b"\r");
    Fixture::wait_state(&path, "stopped");
}

#[test]
fn another_window_can_confirm_deleting_a_working_session() {
    let fixture = Fixture::new();
    let (client, path) = fixture.launch();
    let mut viewer = fixture.client(Some(&fixture.temporary.path().join("other-board")));
    viewer.send(b"a");
    viewer.wait_for("Durable session fixture");
    viewer.send(b"d");
    viewer.wait_for("Delete agent run");
    // Cancel leaves the working session untouched.
    viewer.send(b"\x1b");
    viewer.send(b"d");
    viewer.wait_for("Delete agent run");
    viewer.send(b"\r");
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(10);
    while path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !path.exists(),
        "Shared deletion should stop and remove the session"
    );
    assert!(fixture.temporary.path().join("project").exists());
}

#[test]
fn shared_board_open_waits_for_an_update_instead_of_rejecting_another_window() {
    let fixture = Fixture::new();
    let lock = File::options()
        .read(true)
        .write(true)
        .open(fixture.board.join("rust.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let mut child = fixture
        .command()
        .args(["local", "list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        child.try_wait().unwrap().is_none(),
        "Another window should wait for the short board update"
    );
    fs2::FileExt::unlock(&lock).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
