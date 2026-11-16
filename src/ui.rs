use crate::*;
use std::sync::mpsc::{self, Sender};

#[derive(Clone, Copy)]
enum PickKind {
    Workspace,
    Project,
    Repository,
}
struct Picker {
    kind: PickKind,
    row: usize,
    query: String,
}
struct Choice {
    id: Option<String>,
    name: String,
    description: String,
}
enum Job {
    Catalog(Result<governance::Catalog>),
    Snapshot(String, String, Result<(Vec<Value>, Vec<Value>)>),
    Written(String, Result<Value>, bool),
    RunPublished(String, Result<Value>),
    Reports(String, Result<Vec<Value>>),
}
fn refresh_failure(repository: &str, error: &anyhow::Error) -> String {
    let detail = format!("{error:#}");
    let explanation = if detail.contains("HTTP 404:") {
        "Repository unavailable. Check access or whether it exists."
    } else if detail.contains("HTTP 403:") || detail.contains("HTTP 401:") {
        "Access denied. Check Collar login and repository permissions."
    } else {
        "Could not refresh. Check the connection and Collar details below."
    };
    let capability_hint = if detail.contains("HTTP 404:") {
        "\nA 404 can also mean the issue tracker or requested API feature is disabled."
    } else {
        ""
    };
    format!(
        "{repository}: {explanation}\n\nThe repository remains in Governance. Previously cached tasks are retained.{capability_hint}\n\nCollar details:\n{detail}"
    )
}
#[derive(Default)]
pub(crate) struct Ui {
    column: usize,
    row: usize,
    repo: usize,
    workspace: Option<String>,
    project: Option<String>,
    query: String,
    detail: bool,
    scroll: u16,
    agents: bool,
    help: bool,
    prompt: Option<(char, String)>,
    picker: Option<Picker>,
    message: String,
    board: bool,
    source: usize,
    moving: Option<usize>,
    syncing: usize,
    discovering: bool,
    writing: bool,
    offline: bool,
    error: bool,
    search_before: String,
    sync_errors: Vec<String>,
    sync_succeeded: usize,
    edit_target: Option<(String, u64)>,
    pending_edit: Option<(char, String)>,
    cursor: usize,
    agent_view: bool,
    run_row: usize,
    run_scroll: u16,
    run_output: bool,
    run_all: bool,
    runs: Vec<crate::agents::Run>,
    active_runs: usize,
    launch: Option<agents::Launch>,
    agent_diff: Option<String>,
    publish_run: Option<String>,
    error_popup: bool,
}
impl Ui {
    fn visible_runs<'a>(&'a self, store: &Store) -> Vec<&'a crate::agents::Run> {
        let repos = self.repos(store);
        self.runs
            .iter()
            .filter(|run| {
                self.run_all
                    || repos.contains(&run.repository)
                    || store.tasks().iter().any(|task| {
                        task["id"] == run.task_id && self.local_task_visible(store, task)
                    })
            })
            .collect()
    }
    fn local_task_visible(&self, store: &Store, task: &Value) -> bool {
        task["localProject"].as_str().is_some_and(|id| {
            store.catalog.workspaces.iter().any(|workspace| {
                workspace.slug == local::WORKSPACE
                    && workspace.projects.iter().any(|project| project.slug == id)
            }) && self
                .workspace
                .as_deref()
                .is_none_or(|workspace| workspace == local::WORKSPACE)
                && self.project.as_deref().is_none_or(|project| project == id)
        })
    }
    fn local_scope(&self) -> bool {
        self.workspace.as_deref() == Some(local::WORKSPACE)
    }
    fn open_prompt(&mut self, mode: char, value: String) {
        self.cursor = value.chars().count();
        self.prompt = Some((mode, value));
    }
    fn repos(&self, store: &Store) -> Vec<String> {
        store
            .catalog
            .workspaces
            .iter()
            .filter(|w| self.workspace.as_ref().is_none_or(|s| w.slug == *s))
            .flat_map(|w| {
                w.projects
                    .iter()
                    .filter(|p| self.project.as_ref().is_none_or(|s| p.slug == *s))
                    .flat_map(|p| p.repositories.clone())
            })
            .collect()
    }
    fn workspace_name<'a>(&self, store: &'a Store) -> &'a str {
        self.workspace
            .as_ref()
            .and_then(|slug| store.catalog.workspaces.iter().find(|w| w.slug == *slug))
            .map(|w| w.name.as_str())
            .unwrap_or("All workspaces")
    }
    fn project_name<'a>(&self, store: &'a Store) -> &'a str {
        self.project
            .as_ref()
            .and_then(|slug| {
                store
                    .catalog
                    .workspaces
                    .iter()
                    .filter(|w| self.workspace.as_ref().is_none_or(|s| w.slug == *s))
                    .flat_map(|w| &w.projects)
                    .find(|p| p.slug == *slug)
            })
            .map(|p| p.name.as_str())
            .unwrap_or("All projects")
    }
    fn visible<'a>(&self, store: &'a Store) -> Vec<&'a Value> {
        let repos = self.repos(store);
        let selected = self.repo.checked_sub(1).and_then(|i| repos.get(i));
        let query = self.query.to_lowercase();
        let mut tasks: Vec<_> = store
            .tasks()
            .iter()
            .filter(|t| {
                let local = t["localProject"].is_string();
                let draft = t["source"].is_null() && !local;
                (if local {
                    self.local_task_visible(store, t)
                } else if draft {
                    self.source == 3
                        && (string(t, "repo").is_empty() || repos.iter().any(|r| t["repo"] == *r))
                } else {
                    repos.iter().any(|r| t["repo"] == *r)
                }) && selected.is_none_or(|repo| t["repo"] == *repo)
                    && (self.source == 0
                        || (local && self.source == 1)
                        || match self.source {
                            1 => t["source"]["kind"] == "issue",
                            2 => t["source"]["kind"] == "pr",
                            _ => draft,
                        })
                    && (!self.agents
                        || t["claim"].is_object()
                        || t["report"].is_object()
                        || t["proposals"].as_array().is_some_and(|p| !p.is_empty()))
                    && (query.is_empty()
                        || format!(
                            "{} {} {} {}",
                            string(t, "title"),
                            string(t, "body"),
                            string(t, "owner"),
                            t["source"]["number"]
                        )
                        .to_lowercase()
                        .contains(&query))
            })
            .collect();
        tasks.sort_by_key(|t| match string(t, "priority") {
            "high" => 0,
            "normal" => 1,
            _ => 2,
        });
        tasks
    }
    fn selected<'a>(&self, store: &'a Store) -> Option<&'a Value> {
        self.visible(store)
            .into_iter()
            .filter(|t| t["stage"] == STAGES[self.column])
            .nth(self.row)
    }
    fn focus(&mut self, store: &Store, repo: &str, number: &Value) {
        let task = self
            .visible(store)
            .into_iter()
            .find(|t| t["repo"] == repo && t["source"]["number"] == *number);
        if let Some(task) = task
            && let Some(stage) = STAGES.iter().position(|s| task["stage"] == *s)
        {
            self.column = stage;
            self.row = self
                .visible(store)
                .iter()
                .filter(|t| t["stage"] == STAGES[stage])
                .position(|t| t["id"] == task["id"])
                .unwrap_or(0);
        }
    }
    fn clamp(&mut self, store: &Store) {
        let count = self
            .visible(store)
            .iter()
            .filter(|t| t["stage"] == STAGES[self.column])
            .count();
        self.row = self.row.min(count.saturating_sub(1));
    }
    fn choices(&self, store: &Store, kind: PickKind, query: &str) -> Vec<Choice> {
        let query = query.to_lowercase();
        let mut choices = match kind {
            PickKind::Workspace => {
                let mut items = vec![Choice {
                    id: None,
                    name: "All workspaces".into(),
                    description: "Every Governance team".into(),
                }];
                items.extend(store.catalog.workspaces.iter().map(|w| Choice {
                    id: Some(w.slug.clone()),
                    name: w.name.clone(),
                    description: format!("{} projects  /  {}", w.projects.len(), w.description),
                }));
                items
            }
            PickKind::Project => {
                let mut items = vec![Choice {
                    id: None,
                    name: "All projects".into(),
                    description: self.workspace_name(store).into(),
                }];
                items.extend(
                    store
                        .catalog
                        .workspaces
                        .iter()
                        .filter(|w| self.workspace.as_ref().is_none_or(|s| w.slug == *s))
                        .flat_map(|w| {
                            w.projects.iter().map(|p| Choice {
                                id: Some(p.slug.clone()),
                                name: p.name.clone(),
                                description: if w.slug == local::WORKSPACE {
                                    p.description.clone()
                                } else {
                                    format!("{} / {} repositories", w.name, p.repositories.len())
                                },
                            })
                        }),
                );
                items
            }
            PickKind::Repository => {
                let mut items = vec![Choice {
                    id: None,
                    name: "All repositories".into(),
                    description: self.project_name(store).into(),
                }];
                items.extend(self.repos(store).into_iter().map(|r| Choice {
                    id: Some(r.clone()),
                    name: r,
                    description: "GCD repository".into(),
                }));
                items
            }
        };
        choices.retain(|c| {
            query.is_empty()
                || format!("{} {}", c.name, c.description)
                    .to_lowercase()
                    .contains(&query)
        });
        choices
    }
    fn open_picker(&mut self, kind: PickKind) {
        self.picker = Some(Picker {
            kind,
            row: 0,
            query: String::new(),
        });
    }
    fn start_discovery(&mut self, sender: &Sender<Job>) -> Result<()> {
        ensure!(
            !self.offline,
            "Offline mode uses the cached Governance catalog"
        );
        ensure!(!self.discovering, "Governance refresh is already running");
        self.discovering = true;
        self.message = "Reading teams and projects from Governance through Collar".into();
        let sender = sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Job::Catalog(governance::discover()));
        });
        Ok(())
    }
    fn start_sync(&mut self, store: &Store, sender: &Sender<Job>) -> Result<()> {
        if self.local_scope() {
            self.message = "Local directory projects / no remote refresh needed".into();
            return Ok(());
        }
        ensure!(!self.offline, "Offline mode cannot refresh GCD");
        ensure!(self.syncing == 0, "A refresh is already running");
        let repos = self.repos(store);
        let repos: Vec<_> = repos
            .into_iter()
            .enumerate()
            .filter(|(i, _)| self.repo == 0 || *i + 1 == self.repo)
            .map(|(_, r)| r)
            .collect();
        ensure!(
            !repos.is_empty(),
            "This Governance project has no repositories"
        );
        self.syncing = repos.len();
        self.sync_errors.clear();
        self.sync_succeeded = 0;
        self.message = "Refreshing GCD; you can keep working".into();
        let sender = sender.clone();
        std::thread::spawn(move || {
            for repo in repos {
                let read_started = now();
                let result = shared::issues(&repo)
                    .and_then(|issues| collar(&repo, "pr").map(|prs| (issues, prs)));
                if sender
                    .send(Job::Snapshot(repo, read_started, result))
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(())
    }
    fn write_issue(
        &mut self,
        task: Value,
        field: &str,
        value: String,
        sender: &Sender<Job>,
    ) -> Result<()> {
        ensure!(!self.offline, "Offline mode cannot change GCD");
        ensure!(!self.writing, "A GCD write is already in progress");
        self.writing = true;
        self.error = false;
        self.message = "Saving the issue on GCD through Collar".into();
        let sender = sender.clone();
        let repo = string(&task, "repo").to_owned();
        let field = field.to_owned();
        std::thread::spawn(move || {
            let _ = sender.send(Job::Written(
                repo,
                shared::update(&task, &field, &value),
                false,
            ));
        });
        Ok(())
    }
    fn create_issue(&mut self, repo: String, title: String, sender: &Sender<Job>) -> Result<()> {
        ensure!(!self.offline, "Offline: use N for an unsent local draft");
        ensure!(!self.writing, "A GCD write is already in progress");
        self.writing = true;
        self.message = format!("Creating an issue in {repo}");
        let sender = sender.clone();
        std::thread::spawn(move || {
            let result = shared::create(&repo, &title);
            let _ = sender.send(Job::Written(repo, result, true));
        });
        Ok(())
    }
    pub(crate) fn draw(&self, f: &mut ratatui::Frame, store: &Store) {
        render::draw(self, f, store)
    }
}
mod actions;
mod agents;
mod render;

pub(crate) fn tui(
    store: &mut Store,
    workspace: Option<String>,
    project: Option<String>,
    offline: bool,
) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "An interactive terminal is required"
    );
    let mut terminal = ratatui::init();
    let result = run_ui(&mut terminal, store, workspace, project, offline);
    ratatui::restore();
    result
}
fn initial_scope(
    workspace: Option<String>,
    project: Option<String>,
    preferences: &Value,
) -> (Option<String>, Option<String>) {
    let preferred_workspace = preferences["workspace"].as_str().map(str::to_owned);
    let workspace = workspace.or_else(|| preferred_workspace.clone());
    let project = project.or_else(|| {
        if workspace == preferred_workspace {
            preferences["project"].as_str().map(str::to_owned)
        } else {
            None
        }
    });
    (workspace, project)
}
fn run_ui(
    terminal: &mut ratatui::DefaultTerminal,
    store: &mut Store,
    workspace: Option<String>,
    project: Option<String>,
    offline: bool,
) -> Result<()> {
    let interrupted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGQUIT,
    ] {
        signal_hook::flag::register(signal, interrupted.clone())?;
    }
    let (workspace, project) = initial_scope(workspace, project, &store.data["preferences"]);
    let saved_view = store.data["preferences"]["workspace"].as_str() == workspace.as_deref()
        && store.data["preferences"]["project"].as_str() == project.as_deref();
    let source = if saved_view {
        store.data["preferences"]["source"]
            .as_u64()
            .filter(|source| *source <= 3)
            .unwrap_or(1) as usize
    } else {
        1
    };
    let column = if saved_view {
        store.data["preferences"]["column"]
            .as_u64()
            .filter(|column| *column <= 4)
            .unwrap_or(0) as usize
    } else {
        0
    };
    let mut ui = Ui {
        workspace,
        project,
        source,
        column,
        run_all: true,
        offline,
        ..Ui::default()
    };
    let (sender, receiver) = mpsc::channel::<Job>();
    let mut manager = crate::agents::Manager::open(&store.dir)?;
    if ui.local_scope() {
        ui.message = "Local directory projects / c add directory / n task / A agent".into();
    } else if offline {
        ui.message = "OFFLINE / cached Governance and GCD data".into();
    } else {
        ui.start_discovery(&sender)?;
    }
    loop {
        if interrupted.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        if let Err(error) = manager.poll() {
            ui.error = true;
            ui.message = format!("Agent state could not be saved: {error:#}");
        }
        ui.runs = manager.snapshots();
        ui.active_runs = manager.active_count();
        ui.run_row = ui
            .run_row
            .min(ui.visible_runs(store).len().saturating_sub(1));
        ui.clamp(store);
        terminal.draw(|f| ui.draw(f, store))?;
        FileExt::unlock(&store._lock)?;
        let pending = event::poll(Duration::from_millis(150));
        FileExt::lock_exclusive(&store._lock)?;
        store.reload()?;
        while let Ok(job) = receiver.try_recv() {
            let result = (|| -> Result<()> {
                match job {
                    Job::Catalog(result) => {
                        ui.discovering = false;
                        let previous_repo = ui
                            .repo
                            .checked_sub(1)
                            .and_then(|i| ui.repos(store).get(i).cloned());
                        governance::apply(store, result?)?;
                        let valid_workspace = ui.workspace.as_ref().is_none_or(|slug| {
                            store.catalog.workspaces.iter().any(|w| w.slug == *slug)
                        });
                        if !valid_workspace {
                            ui.workspace = None;
                            ui.project = None;
                            ui.repo = 0;
                            ui.open_picker(PickKind::Workspace);
                            ui.message="The previous workspace is no longer declared in Governance. Choose a workspace.".into();
                        } else if governance::repositories(
                            &store.catalog,
                            ui.workspace.as_deref(),
                            ui.project.as_deref(),
                        )
                        .is_err()
                        {
                            ui.project = None;
                            ui.repo = 0;
                            ui.open_picker(PickKind::Project);
                            ui.message="The previous project is no longer declared in Governance. Choose a project.".into();
                        } else {
                            ui.repo = previous_repo
                                .and_then(|repo| ui.repos(store).iter().position(|r| *r == repo))
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            ui.message =
                                "Governance refreshed. w workspace / P project / s refresh tasks"
                                    .into();
                            if ui.workspace.is_none() {
                                ui.open_picker(PickKind::Workspace)
                            } else if ui.syncing == 0 {
                                ui.start_sync(store, &sender)?;
                            }
                        }
                    }
                    Job::Snapshot(repo, read_started, result) => {
                        ui.syncing = ui.syncing.saturating_sub(1);
                        match result.and_then(|(issues, prs)| {
                            import_snapshot(store, &repo, &issues, &prs, &read_started)?;
                            Ok((issues.len(), prs.len()))
                        }) {
                            Ok((issues, prs)) => {
                                ui.sync_succeeded += 1;
                                ui.message = format!("{repo}: {issues} issues, {prs} PRs refreshed")
                            }
                            Err(error) => ui.sync_errors.push(refresh_failure(&repo, &error)),
                        }
                        if ui.syncing == 0 && !ui.sync_errors.is_empty() {
                            let headline = if ui.sync_errors.len() == 1 {
                                ui.sync_errors[0]
                                    .lines()
                                    .next()
                                    .unwrap_or_default()
                                    .to_owned()
                            } else {
                                format!("{} repositories could not refresh.", ui.sync_errors.len())
                            };
                            bail!(
                                "{headline} / {} refreshed / ! Details\n\n{} repositories refreshed successfully. Available tasks remain visible.\n\n{}",
                                ui.sync_succeeded,
                                ui.sync_succeeded,
                                ui.sync_errors.join("\n\n---\n\n")
                            )
                        }
                    }
                    Job::RunPublished(id, result) => {
                        ui.writing = false;
                        match result {
                            Ok(comment) => {
                                manager.set_publication(&id, "published")?;
                                let run = manager.get(&id)?;
                                if let Ok(task) = store.task_mut(&run.task_id) {
                                    if !task["source"]["agent_reports"].is_array() {
                                        task["source"]["agent_reports"] = json!([]);
                                    }
                                    if let Some(reports) =
                                        task["source"]["agent_reports"].as_array_mut()
                                    {
                                        reports.insert(0, comment);
                                    }
                                    store.save()?;
                                }
                                ui.message =
                                    "Reviewed agent result published to the GCD issue".into();
                            }
                            Err(error) => {
                                manager.set_publication(&id, "unknown")?;
                                return Err(error.context(
                                    "Publication is uncertain; inspect GCD before another attempt",
                                ));
                            }
                        }
                    }
                    Job::Reports(id, result) => {
                        let reports = result?;
                        if let Ok(task) = store.task_mut(&id) {
                            task["source"]["agent_reports"] = json!(reports);
                            store.save()?;
                        }
                    }
                    Job::Written(repo, result, created) => {
                        ui.writing = false;
                        match result {
                            Ok(raw) => {
                                let record = shared::normalize(&raw);
                                import_records(store, &repo, std::slice::from_ref(&record), &[])?;
                                ui.pending_edit = None;
                                ui.message = format!("Saved on GCD: {repo} #{}", raw["number"]);
                                if created {
                                    ui.source = 1;
                                    ui.query.clear();
                                    ui.agents = false;
                                }
                                ui.focus(store, &repo, &raw["number"]);
                            }
                            Err(error) => {
                                if let Some((mode, value)) = ui.pending_edit.take() {
                                    ui.open_prompt(mode, value)
                                }
                                return Err(error);
                            }
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                ui.error = true;
                ui.message = format!("{error:#}")
            } else {
                ui.error = false
            }
        }
        if !pending? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        if ui.error_popup {
            ui.error_popup = false;
            continue;
        }
        if !ui.help && ui.prompt.is_none() && ui.picker.is_none() && ui.moving.is_none() {
            match agents::key(&mut ui, store, key, &sender, &mut manager) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    ui.error = true;
                    ui.message = format!("{error:#}");
                    continue;
                }
            }
        }
        if key.code == KeyCode::Enter
            && !ui.detail
            && !ui.offline
            && !ui.agent_view
            && ui.prompt.is_none()
            && ui.picker.is_none()
            && let Some(task) = ui
                .selected(store)
                .filter(|t| t["source"]["kind"] == "issue")
                .cloned()
        {
            let id = string(&task, "id").to_owned();
            let sender = sender.clone();
            std::thread::spawn(move || {
                let _ = sender.send(Job::Reports(id, shared::agent_reports(&task)));
            });
        }
        match actions::key(&mut ui, store, key, &sender) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                ui.error = true;
                ui.message = format!("{error:#}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn fixture() -> Result<(tempfile::TempDir, Store)> {
        let dir = tempfile::tempdir()?;
        let mut store = Store::open(dir.path().into())?;
        governance::apply(
            &mut store,
            governance::from_records(governance::tests::records())?,
        )?;
        import_records(
            &mut store,
            "ScottyLabs/infrastructure",
            &[
                json!({"number":1,"title":"Deploy preview","body":"- [ ] Check deployment","state":"OPEN","labels":[{"name":"urgency/high"}],"_remote":{"assignees":[{"login":"krit"}]}}),
            ],
            &[],
        )?;
        import_records(
            &mut store,
            "ScottyLabs/housing",
            &[json!({"number":2,"title":"Housing survey","state":"OPEN","labels":[]})],
            &[],
        )?;
        Ok((dir, store))
    }
    #[test]
    fn canonical_workspace_and_project_filters_do_not_invent_membership() -> Result<()> {
        let (_dir, store) = fixture()?;
        let mut ui = Ui {
            workspace: Some("devops".into()),
            source: 1,
            ..Ui::default()
        };
        assert_eq!(ui.visible(&store).len(), 1);
        assert_eq!(string(ui.visible(&store)[0], "owner"), "krit");
        ui.project = Some("changelog".into());
        assert!(ui.repos(&store).is_empty());
        assert!(ui.visible(&store).is_empty());
        ui.workspace = Some("cmu-housing".into());
        ui.project = Some("housing".into());
        assert_eq!(string(ui.visible(&store)[0], "title"), "Housing survey");
        Ok(())
    }
    #[test]
    fn explicit_workspace_does_not_inherit_another_teams_project() {
        let preferences = json!({"workspace":"devops","project":"infrastructure"});
        assert_eq!(
            initial_scope(Some("cmu-housing".into()), None, &preferences),
            (Some("cmu-housing".into()), None)
        );
        assert_eq!(
            initial_scope(None, None, &preferences),
            (Some("devops".into()), Some("infrastructure".into()))
        );
    }
    #[test]
    fn local_drafts_require_explicit_draft_view() -> Result<()> {
        let (_dir, mut store) = fixture()?;
        store.add("Unsent", None, "")?;
        let mut ui = Ui::default();
        assert_eq!(ui.visible(&store).len(), 2);
        ui.source = 3;
        assert_eq!(ui.visible(&store).len(), 1);
        Ok(())
    }
    #[test]
    fn directory_tasks_are_scoped_and_visible_without_draft_mode() -> Result<()> {
        let (temporary, mut store) = fixture()?;
        let directory = temporary.path().join("plain");
        fs::create_dir(&directory)?;
        let project = local::add(&mut store, directory, None)?;
        local::task(&mut store, &project.id, "Local work")?;
        let shared = Ui {
            workspace: Some("devops".into()),
            source: 1,
            ..Ui::default()
        };
        assert!(
            shared
                .visible(&store)
                .iter()
                .all(|task| task["localProject"].is_null())
        );
        let local = Ui {
            workspace: Some(local::WORKSPACE.into()),
            project: Some(project.id),
            source: 1,
            ..Ui::default()
        };
        assert_eq!(local.visible(&store).len(), 1);
        assert_eq!(local.visible(&store)[0]["title"], "Local work");
        local::remove(
            &mut store,
            local.project.as_deref().context("Missing project")?,
        )?;
        let all = Ui {
            source: 1,
            ..Ui::default()
        };
        assert!(
            all.visible(&store)
                .iter()
                .all(|task| task["localProject"].is_null())
        );
        Ok(())
    }
    #[test]
    fn responsive_layouts_keep_metadata_outside_selection_band() -> Result<()> {
        let (_dir, store) = fixture()?;
        for (w, h) in [(60, 16), (80, 24), (100, 30), (140, 40), (190, 50)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h))?;
            for mut ui in [
                Ui::default(),
                Ui {
                    board: true,
                    ..Ui::default()
                },
                Ui {
                    detail: true,
                    ..Ui::default()
                },
                Ui {
                    agent_view: true,
                    runs: vec![crate::agents::Run::fixture(crate::agents::Provider::Codex)],
                    ..Ui::default()
                },
                Ui {
                    launch: Some(agents::Launch {
                        task: json!({"repo":"ScottyLabs/infrastructure","title":"Review task"}),
                        provider: crate::agents::Provider::Codex,
                        mode: crate::agents::Mode::Plan,
                        checkout: "/tmp/example".into(),
                        goal: "Inspect the task".into(),
                        field: 2,
                        cursor: 0,
                        resume: None,
                    }),
                    ..Ui::default()
                },
                Ui {
                    help: true,
                    ..Ui::default()
                },
                Ui {
                    moving: Some(1),
                    ..Ui::default()
                },
                Ui {
                    prompt: Some(('d', "Context".into())),
                    ..Ui::default()
                },
            ] {
                ui.workspace = Some("devops".into());
                terminal.draw(|f| ui.draw(f, &store))?;
                let buffer = terminal.backend().buffer();
                assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
                for y in 0..h {
                    let text = (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>();
                    if text.contains("#1") && text.contains("infrastructure") {
                        assert!(
                            (0..w).all(|x| !buffer[(x, y)].modifier.contains(Modifier::REVERSED))
                        );
                    }
                }
            }
            let mut ui = Ui::default();
            ui.open_picker(PickKind::Workspace);
            terminal.draw(|f| ui.draw(f, &store))?;
        }
        Ok(())
    }
}
