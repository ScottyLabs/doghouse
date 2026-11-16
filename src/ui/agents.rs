use super::*;
use crate::agents::{Manager, Mode, Provider, Run};
use crossterm::event::KeyEvent;

pub(super) struct Launch {
    pub task: Value,
    pub provider: Provider,
    pub mode: Mode,
    pub checkout: String,
    pub goal: String,
    pub field: usize,
    pub cursor: usize,
    pub resume: Option<String>,
    pub settings: bool,
}
fn next_provider(provider: Provider) -> Provider {
    if provider == Provider::Codex {
        Provider::Claude
    } else {
        Provider::Codex
    }
}
fn launch(ui: &mut Ui, store: &Store, manager: &Manager, resume: Option<String>) -> Result<()> {
    ensure!(!ui.offline, "Offline mode cannot launch model runs");
    if let Some(id) = resume {
        let run = manager.get(&id)?;
        ensure!(
            !run.state.active(),
            "Stop or finish this run before resuming"
        );
        ui.launch = Some(Launch {
            task: Value::Null,
            provider: run.provider,
            mode: run.mode,
            checkout: run.directory.display().to_string(),
            goal: String::new(),
            field: 3,
            cursor: 0,
            resume: Some(id),
            settings: false,
        });
    } else {
        let task = if ui.agent_view {
            if let Some(run) = ui.visible_runs(store).get(ui.run_row) {
                Some(store.tasks().iter().find(|task| task["id"] == run.task_id)
                    .cloned().context("This run's task is not cached. Refresh its original project before launching another agent")?)
            } else { ui.selected(store).cloned() }
        } else { ui.selected(store).cloned() }
        .context("Choose a project task before starting an agent")?;
        let repository = string(&task, "repo");
        ensure!(
            repo_valid(repository) || task["localProject"].is_string(),
            "Choose a task with a project repository"
        );
        let checkout = task["directory"]
            .as_str()
            .or_else(|| store.data["checkouts"][repository].as_str())
            .map(str::to_owned)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| {
                    PathBuf::from(home)
                        .join("dev")
                        .join(repository.rsplit('/').next().unwrap_or(repository))
                        .display()
                        .to_string()
                })
            })
            .unwrap_or_default();
        let default_mode =
            if task["source"]["kind"] == "pr" || !PathBuf::from(&checkout).join(".git").exists() {
                Mode::Plan
            } else {
                Mode::Implement
            };
        let preferred: Option<Provider> =
            serde_json::from_value(store.data["agentDefaults"]["provider"].clone()).ok();
        let needs_checkout = !PathBuf::from(&checkout).is_dir();
        ui.launch = Some(Launch {
            task,
            provider: if let Some(provider) = preferred.filter(|provider| provider.available()) {
                provider
            } else if Provider::Codex.available() {
                Provider::Codex
            } else {
                Provider::Claude
            },
            mode: default_mode,
            cursor: if needs_checkout {
                checkout.chars().count()
            } else {
                0
            },
            checkout,
            goal: String::new(),
            field: if needs_checkout { 2 } else { 3 },
            resume: None,
            settings: needs_checkout,
        });
    }
    ui.error = false;
    Ok(())
}
fn selected_run(ui: &Ui, store: &Store) -> Result<Run> {
    ui.visible_runs(store)
        .get(ui.run_row)
        .cloned()
        .cloned()
        .context("Select an agent run first")
}
pub(super) fn key(
    ui: &mut Ui,
    store: &mut Store,
    key: KeyEvent,
    sender: &Sender<Job>,
    manager: &mut Manager,
) -> Result<bool> {
    if let Some(id) = ui.publish_run.clone() {
        match key.code {
            KeyCode::Esc => ui.publish_run = None,
            KeyCode::PageDown | KeyCode::Down => ui.run_scroll = ui.run_scroll.saturating_add(10),
            KeyCode::PageUp | KeyCode::Up => ui.run_scroll = ui.run_scroll.saturating_sub(10),
            KeyCode::Enter => {
                ensure!(!ui.offline, "Offline mode cannot publish to GCD");
                ensure!(!ui.writing, "A GCD write is in progress");
                let body = manager.begin_publication(&id)?;
                let run = manager.get(&id)?.clone();
                let number = run.task_number.context("Run has no GCD issue")?;
                ui.writing = true;
                ui.publish_run = None;
                ui.message = "Publishing the reviewed result to GCD".into();
                let sender = sender.clone();
                std::thread::spawn(move || {
                    let result = shared::publish_run(&run.repository, number, &body);
                    let _ = sender.send(Job::RunPublished(id, result));
                });
            }
            _ => {}
        }
        return Ok(true);
    }
    if let Some(mut form) = ui.launch.take() {
        let mut submit = false;
        match key.code {
            KeyCode::Esc => return Ok(true),
            KeyCode::Tab => {
                form.field = (form.field + 1) % 4;
                form.settings = form.field != 3;
                form.cursor = if form.field == 2 {
                    form.checkout.chars().count()
                } else {
                    form.goal.chars().count()
                };
            }
            KeyCode::BackTab => {
                form.field = (form.field + 3) % 4;
                form.settings = form.field != 3;
                form.cursor = if form.field == 2 {
                    form.checkout.chars().count()
                } else {
                    form.goal.chars().count()
                };
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => submit = true,
            KeyCode::Enter if form.field == 3 => submit = true,
            KeyCode::Enter => {
                form.field = 3;
                form.settings = false;
                form.cursor = form.goal.chars().count();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if form.field < 2 && form.resume.is_none() =>
            {
                if form.field == 0 {
                    form.provider = next_provider(form.provider)
                } else {
                    form.mode = if form.mode == Mode::Plan {
                        Mode::Implement
                    } else {
                        Mode::Plan
                    };
                    if [
                        "Inspect this task and propose a concrete implementation plan.",
                        "Implement this task, run relevant checks, and leave changes for review.",
                    ]
                    .contains(&form.goal.as_str())
                    {
                        form.goal=if form.mode==Mode::Plan{"Inspect this task and propose a concrete implementation plan."}else{"Implement this task, run relevant checks, and leave changes for review."}.into();
                    }
                }
            }
            _ if form.field >= 2 => {
                if form.field == 2 && form.resume.is_some() {
                    ui.launch = Some(form);
                    return Ok(true);
                }
                let text = if form.field == 2 {
                    &mut form.checkout
                } else {
                    &mut form.goal
                };
                let mut chars = text.chars().collect::<Vec<_>>();
                form.cursor = form.cursor.min(chars.len());
                match key.code {
                    KeyCode::Left => form.cursor = form.cursor.saturating_sub(1),
                    KeyCode::Right => form.cursor = (form.cursor + 1).min(chars.len()),
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        chars.clear();
                        form.cursor = 0;
                    }
                    KeyCode::Home => form.cursor = 0,
                    KeyCode::End => form.cursor = chars.len(),
                    KeyCode::Backspace if form.cursor > 0 => {
                        chars.remove(form.cursor - 1);
                        form.cursor -= 1;
                    }
                    KeyCode::Delete if form.cursor < chars.len() => {
                        chars.remove(form.cursor);
                    }
                    KeyCode::Enter if form.field == 3 => {
                        chars.insert(form.cursor, '\n');
                        form.cursor += 1;
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        chars.insert(form.cursor, c);
                        form.cursor += 1;
                    }
                    _ => {}
                }
                *text = chars.iter().collect();
            }
            _ => {}
        }
        if submit {
            if form.goal.trim().is_empty() && form.resume.is_none() {
                form.goal = if form.mode == Mode::Implement { "Implement this task following the project instructions. Outline the approach first, then implement and run relevant checks." } else { "Inspect this task following the project instructions. Provide a concise plan, risks, and suggested checks." }.into();
            }
            let (width, height) = crossterm::terminal::size()?;
            if width < 70 || height < 20 {
                ui.launch = Some(form);
                bail!("Enlarge the terminal to review all launch settings")
            }
            let result = if let Some(id) = &form.resume {
                manager.resume(id, form.goal.clone())
            } else {
                let current = store
                    .tasks()
                    .iter()
                    .find(|task| task["id"] == form.task["id"])
                    .context("This task is no longer cached; refresh it before launching")?;
                if crate::agents::task_context(current) != crate::agents::task_context(&form.task) {
                    form.task = current.clone();
                    ui.launch = Some(form);
                    bail!("Task context changed. Review the updated task before launching again");
                }
                manager.start(
                    &form.task,
                    form.provider,
                    form.mode,
                    PathBuf::from(&form.checkout),
                    form.goal.clone(),
                )
            };
            match result {
                Ok(id) => {
                    if form.resume.is_none() && form.task["localProject"].is_null() {
                        store.data["checkouts"][string(&form.task, "repo")] = json!(form.checkout);
                        store.save()?;
                    }
                    store.data["agentDefaults"]["provider"] = serde_json::to_value(form.provider)?;
                    store.save()?;
                    ui.agent_view = true;
                    ui.run_row = 0;
                    ui.message = format!("{} run started: {id}", form.provider.name());
                    ui.error = false;
                }
                Err(error) => {
                    ui.launch = Some(form);
                    return Err(error);
                }
            }
        } else {
            ui.launch = Some(form)
        }
        return Ok(true);
    }
    if key.code == KeyCode::Char('A') {
        launch(ui, store, manager, None)?;
        return Ok(true);
    }
    if key.code == KeyCode::Char('a') {
        ui.agent_view = !ui.agent_view;
        if ui.agent_view {
            ui.run_all = true;
        }
        ui.agent_diff = None;
        ui.run_scroll = 0;
        return Ok(true);
    }
    if !ui.agent_view {
        return Ok(false);
    }
    match key.code {
        KeyCode::Esc => {
            if ui.agent_diff.is_some() {
                ui.agent_diff = None
            } else {
                ui.agent_view = false
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if ui.agent_diff.is_some() {
                ui.run_scroll = ui.run_scroll.saturating_add(1)
            } else {
                ui.run_row += 1;
                ui.run_scroll = 0;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if ui.agent_diff.is_some() {
                ui.run_scroll = ui.run_scroll.saturating_sub(1)
            } else {
                ui.run_row = ui.run_row.saturating_sub(1);
                ui.run_scroll = 0;
            }
        }
        KeyCode::PageDown => ui.run_scroll = ui.run_scroll.saturating_add(12),
        KeyCode::PageUp => ui.run_scroll = ui.run_scroll.saturating_sub(12),
        KeyCode::Char('n') => launch(ui, store, manager, None)?,
        KeyCode::Char('r') => {
            let run = selected_run(ui, store)?;
            launch(ui, store, manager, Some(run.id))?;
        }
        KeyCode::Char('s') => {
            let run = selected_run(ui, store)?;
            manager.stop(&run.id)?;
            ui.message = "Stopping the selected agent and its tools".into();
        }
        KeyCode::Char('f') => {
            ui.run_all = !ui.run_all;
            ui.run_row = 0;
            ui.run_scroll = 0;
        }
        KeyCode::Char('t') => {
            let run = selected_run(ui, store)?;
            let task = store
                .tasks()
                .iter()
                .find(|task| task["id"] == run.task_id)
                .cloned()
                .context("Task is no longer cached; refresh its original project")?;
            if let Some(project) = task["localProject"].as_str() {
                ensure!(
                    local::projects(&store.data)?
                        .iter()
                        .any(|entry| entry.id == project),
                    "Local project was forgotten. Re-add its directory with c to restore its tasks"
                );
                ui.workspace = Some(local::WORKSPACE.into());
                ui.project = Some(project.into());
            } else {
                let (workspace, project) = store
                    .catalog
                    .workspaces
                    .iter()
                    .find_map(|w| {
                        w.projects
                            .iter()
                            .find(|p| p.repositories.contains(&run.repository))
                            .map(|p| (w.slug.clone(), p.slug.clone()))
                    })
                    .context("Repository is no longer in Governance; O opens its source")?;
                ui.workspace = Some(workspace);
                ui.project = Some(project);
            }
            ui.repo = 0;
            ui.source = if task["localProject"].is_string() || task["source"]["kind"] == "issue" {
                1
            } else if task["source"]["kind"] == "pr" {
                2
            } else {
                3
            };
            ui.query.clear();
            ui.agents = false;
            ui.column = STAGES
                .iter()
                .position(|stage| task["stage"] == *stage)
                .unwrap_or(0);
            ui.row = ui
                .visible(store)
                .iter()
                .filter(|task| task["stage"] == STAGES[ui.column])
                .position(|task| task["id"] == run.task_id)
                .unwrap_or(0);
            ui.agent_view = false;
        }
        KeyCode::Char('O') => {
            let run = selected_run(ui, store)?;
            let number = run.task_number.context("Unsent drafts have no GCD issue")?;
            ensure!(repo_valid(&run.repository), "Invalid run repository");
            let url = run.task_url.clone().unwrap_or_else(|| {
                format!("https://git.cmu.dev/{}/issues/{number}", run.repository)
            });
            ensure!(
                url.starts_with(&format!("https://git.cmu.dev/{}/", run.repository)),
                "Invalid run source URL"
            );
            let status = Command::new("xdg-open")
                .arg(url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;
            ensure!(status.success(), "Browser launch failed");
        }
        KeyCode::Enter => {
            ui.run_output = !ui.run_output;
            ui.run_scroll = 0;
        }
        KeyCode::Char('d') => {
            let run = selected_run(ui, store)?;
            ui.agent_diff = Some(manager.diff(&run.id)?);
            ui.run_scroll = 0;
        }
        KeyCode::Char('x') => {
            let run = selected_run(ui, store)?;
            manager.cleanup(&run.id)?;
            ui.message = "Removed clean checkout; transcript and result retained".into();
        }
        KeyCode::Char('P') => {
            let run = selected_run(ui, store)?;
            manager.publication_body(&run.id)?;
            ui.publish_run = Some(run.id);
            ui.run_scroll = 0;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn return_to_task_uses_source_and_identity() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let mut store = Store::open(temporary.path().into())?;
        governance::apply(
            &mut store,
            governance::from_records(governance::tests::records())?,
        )?;
        import_records(
            &mut store,
            "ScottyLabs/infrastructure",
            &[json!({"number":1,"title":"Issue","state":"OPEN"})],
            &[json!({"number":2,"title":"Change","state":"OPEN"})],
        )?;
        store.add("Local draft", Some("ScottyLabs/infrastructure"), "")?;
        let targets = store
            .tasks()
            .iter()
            .filter(|task| task["source"]["kind"] == "pr" || task["source"].is_null())
            .cloned()
            .collect::<Vec<_>>();
        let mut manager = Manager::open(&store.dir)?;
        let (sender, _receiver) = mpsc::channel();
        for task in targets {
            let mut run = Run::fixture(Provider::Codex);
            run.task_id = string(&task, "id").into();
            run.repository = "ScottyLabs/infrastructure".into();
            run.task_number = task["source"]["number"].as_u64();
            let mut ui = Ui {
                agent_view: true,
                run_all: true,
                runs: vec![run],
                ..Ui::default()
            };
            key(
                &mut ui,
                &mut store,
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
                &sender,
                &mut manager,
            )?;
            assert!(!ui.agent_view);
            assert_eq!(
                ui.selected(&store).map(|selected| &selected["id"]),
                Some(&task["id"])
            );
            assert_eq!(ui.source, if task["source"].is_null() { 3 } else { 2 });
        }
        Ok(())
    }
}
