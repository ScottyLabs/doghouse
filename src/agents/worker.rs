use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn busy(root: &std::path::Path, key: &str) -> Result<bool> {
    match lease(root, key) {
        Ok(_lease) => Ok(false),
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::WouldBlock) =>
        {
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn live(root: &std::path::Path, id: &str) -> Result<bool> {
    busy(root, &format!("worker:{id}"))
}

pub(super) fn reconcile(root: &std::path::Path, run: &mut Run) -> Result<()> {
    if !run.state.active() && run.state != State::Unattached {
        return Ok(());
    }
    if live(root, &run.id)? {
        return Ok(());
    }
    let record_directory = root.join(&run.id).canonicalize()?;
    let socket = record_directory.join("terminal.socket");
    if socket.exists() && matches!(interactive::console_status(&socket), Ok(None)) {
        run.state = State::Detached;
        return Ok(());
    }
    let legacy_root = record_directory
        .parent()
        .map(std::path::Path::to_owned)
        .context("Missing session directory")?;
    for directory in [root, legacy_root.as_path()] {
        if busy(directory, &format!("checkout:{}", run.directory.display()))? {
            run.state = State::Unattached;
            return Ok(());
        }
    }
    run.state = State::Interrupted;
    run.event("Recovery", "Worker is no longer running. Files and saved conversation are retained; choose Resume to continue.");
    Ok(())
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing session directory")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub(super) fn launch(
    manager: &mut Manager,
    id: &str,
    prompt: Option<&str>,
    width: u16,
    height: u16,
) -> Result<()> {
    let _launch = lease(&manager.root, &format!("launch:{id}"))?;
    ensure!(
        !manager.root.join(id).join("delete-request").exists(),
        "This session is being deleted"
    );
    ensure!(
        !live(&manager.root, id)?,
        "This session already has a worker"
    );
    let directory = manager.root.join(id);
    if prompt.is_none() {
        let mut run = manager.read_record(id)?;
        reconcile(&manager.root, &mut run)?;
        ensure!(
            run.state != State::Unattached,
            "A native agent still holds this checkout; open its terminal or stop that process before restarting"
        );
        manager.runs.insert(id.into(), run);
        manager.persist(id)?;
    }
    if directory.join("stop-request").exists() {
        fs::remove_file(directory.join("stop-request"))?;
    }
    let ready = directory.join("worker-ready");
    if ready.exists() {
        fs::remove_file(&ready)?;
    }
    if let Some(prompt) = prompt {
        atomic_write(&directory.join("prompt"), prompt.as_bytes())?;
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--data-dir")
        .arg(manager.root.parent().context("Missing registry")?)
        .args([
            "agent-worker",
            id,
            "--width",
            &width.to_string(),
            "--height",
            &height.to_string(),
        ]);
    if prompt.is_none() {
        command.arg("--terminal");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the post-fork callback performs only the async-signal-safe setsid syscall.
    unsafe {
        command.pre_exec(|| nix::unistd::setsid().map(|_| ()).map_err(io::Error::from));
    }
    let mut child = command.spawn().context("Start persistent agent worker")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if ready.is_file() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            manager.runs.insert(id.into(), manager.read_record(id)?);
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            let run = manager.read_record(id)?;
            bail!(
                "Agent worker could not start ({status}): {}",
                run.failure.as_deref().unwrap_or("inspect session activity")
            );
        }
        if Instant::now() >= deadline {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            bail!(
                "Worker startup is taking longer than expected. Inspect the session before trying again."
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn run(
    cache: &std::path::Path,
    id: &str,
    terminal: bool,
    width: u16,
    height: u16,
) -> Result<()> {
    ensure!(valid_id(id), "Invalid session ID");
    let root = cache.join("agents");
    let _owner = lease(&root, &format!("worker:{id}"))?;
    let mut manager = Manager::open(cache)?;
    manager.detached = false;
    atomic_write(
        &root.join(id).join("worker.json"),
        &serde_json::to_vec(&json!({"pid":std::process::id(),"started_at":now()}))?,
    )?;
    let result = (|| -> Result<bool> {
        let _slot = if terminal {
            None
        } else {
            Some(
                (0..4)
                    .find_map(|index| lease(&root, &format!("worker-slot:{index}")).ok())
                    .context(
                        "Four agents are working; stop or finish one before starting another",
                    )?,
            )
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        for signal in [
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
        ] {
            signal_hook::flag::register(signal, Arc::clone(&cancelled))?;
        }
        if terminal {
            manager.start_console(id, width.clamp(20, 500), height.clamp(8, 200))?;
        } else {
            let path = root.join(id).join("prompt");
            ensure!(
                fs::metadata(&path)?.len() <= 512 * 1024,
                "Agent prompt is too large"
            );
            let prompt = fs::read_to_string(&path)?;
            fs::remove_file(path)?;
            manager.spawn(id, &prompt)?;
        }
        atomic_write(
            &root.join(id).join("worker.json"),
            &serde_json::to_vec(
                &json!({"pid":std::process::id(),"agent_pid":manager.active.get(id).map(|active| active.child.id()),"started_at":now()}),
            )?,
        )?;
        atomic_write(&root.join(id).join("worker-ready"), b"ready\n")?;
        let mut interrupted = None;
        while manager.active_count() > 0 {
            if cancelled.load(Ordering::Relaxed) && interrupted.is_none() {
                interrupted = Some(Instant::now());
                manager.stop(id)?;
            }
            manager.poll()?;
            if interrupted.is_some_and(|time| time.elapsed() >= Duration::from_secs(4)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(interrupted.is_some())
    })();
    match result {
        Ok(interrupted) => {
            drop(manager);
            if interrupted {
                let mut registry = Manager::open(cache)?;
                let run = registry.runs.get_mut(id).context("Session disappeared")?;
                run.state = State::Interrupted;
                run.event(
                    "Recovery",
                    "Worker was interrupted. Resume the saved conversation when ready.",
                );
                registry.persist(id)?;
            }
            Ok(())
        }
        Err(error) => {
            manager.failed_start(id, &error)?;
            Err(error)
        }
    }
}

pub(crate) fn registry_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SHEPHERD_AGENT_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("Set HOME or SHEPHERD_AGENT_DIR for the shared agent registry")?
        .join("shepherd"))
}

impl Manager {
    pub(crate) fn shared(board: &std::path::Path) -> Result<Self> {
        let registry = registry_directory()?;
        fs::create_dir_all(registry.join("agents"))?;
        fs::set_permissions(registry.join("agents"), fs::Permissions::from_mode(0o700))?;
        let default_legacy = if std::env::var_os("SHEPHERD_AGENT_DIR").is_some() {
            None
        } else {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
                .map(|path| path.join("shepherd"))
        };
        for legacy in [Some(board.to_path_buf()), default_legacy]
            .into_iter()
            .flatten()
        {
            let source = legacy.join("agents");
            if !source.is_dir()
                || source.canonicalize()? == registry.join("agents").canonicalize()?
            {
                continue;
            }
            for entry in fs::read_dir(source)? {
                let path = entry?.path();
                let Some(id) = path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .filter(|id| valid_id(id))
                else {
                    continue;
                };
                if !path.join("run.json").is_file() {
                    continue;
                }
                let destination = registry.join("agents").join(id);
                if destination.symlink_metadata().is_ok() {
                    continue;
                }
                // Preserve existing worktree and native-history paths while registering old sessions.
                match std::os::unix::fs::symlink(path.canonicalize()?, destination) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error).context("Register existing agent session"),
                }
            }
        }
        Self::open(&registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_sessions_keep_legacy_checkout_locks_during_transfer() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut legacy = Manager::open(&directory.path().join("old"))?;
        let mut record = Run::fixture(Provider::Codex);
        record.state = State::Completed;
        record.owns_worktree = true;
        record.directory = legacy.root.join(&record.id).join("checkout");
        fs::create_dir_all(&record.directory)?;
        legacy.runs.insert(record.id.clone(), record.clone());
        legacy.persist(&record.id)?;
        let registry = directory.path().join("new");
        fs::create_dir_all(registry.join("agents"))?;
        std::os::unix::fs::symlink(
            legacy.root.join(&record.id),
            registry.join("agents").join(&record.id),
        )?;
        let current = Manager::open(&registry)?;
        let leases = current.execution_leases(current.get(&record.id)?)?;
        let key = format!("checkout:{}", record.directory.display());
        assert!(busy(&legacy.root, &key)?);
        assert!(busy(&current.root, &key)?);
        drop(leases);
        assert!(!busy(&legacy.root, &key)?);
        Ok(())
    }

    #[test]
    fn registry_keeps_sessions_beyond_the_old_turn_limit() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut manager = Manager::open(directory.path())?;
        for index in 0..205 {
            let mut run = Run::fixture(Provider::Codex);
            run.id = format!("session-{index}");
            run.state = State::Completed;
            manager.runs.insert(run.id.clone(), run.clone());
            manager.persist(&run.id)?;
        }
        assert_eq!(Manager::open(directory.path())?.snapshots().len(), 205);
        Ok(())
    }

    #[test]
    fn worker_slots_apply_across_controllers() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut manager = Manager::open(directory.path())?;
        let record = Run::fixture(Provider::Codex);
        manager.runs.insert(record.id.clone(), record.clone());
        manager.persist(&record.id)?;
        let _slots = (0..4)
            .map(|index| lease(&manager.root, &format!("worker-slot:{index}")))
            .collect::<Result<Vec<_>>>()?;
        assert!(run(directory.path(), &record.id, false, 80, 24).is_err());
        let failed = Manager::open(directory.path())?;
        assert_eq!(failed.get(&record.id)?.state, State::Failed);
        assert!(
            failed
                .get(&record.id)?
                .failure
                .as_deref()
                .is_some_and(|message| message.contains("Four agents"))
        );
        Ok(())
    }

    #[test]
    fn stale_socket_after_reboot_is_interrupted_and_a_live_worker_remains_accessible() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let mut manager = Manager::open(directory.path())?;
        let mut record = Run::fixture(Provider::Codex);
        record.session_id = Some("saved-session".into());
        manager.runs.insert(record.id.clone(), record.clone());
        manager.persist(&record.id)?;
        let socket = manager.root.join(&record.id).join("terminal.socket");
        drop(std::os::unix::net::UnixListener::bind(&socket)?);
        let viewer = Manager::open(directory.path())?;
        assert_eq!(viewer.get(&record.id)?.state, State::Interrupted);
        assert_eq!(
            viewer.get(&record.id)?.session_id.as_deref(),
            Some("saved-session")
        );
        let _owner = lease(&manager.root, &format!("worker:{}", record.id))?;
        let another = Manager::open(directory.path())?;
        assert_eq!(another.get(&record.id)?.state, State::Running);
        assert_eq!(another.active_count(), 1);
        Ok(())
    }
}
