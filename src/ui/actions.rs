use super::*;
use crossterm::event::KeyEvent;

pub(super) fn shell_edit(chars: &mut Vec<char>, cursor: &mut usize, key: KeyEvent) -> bool {
    if !key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return false;
    }
    *cursor = (*cursor).min(chars.len());
    match key.code {
        KeyCode::Char('a') => *cursor = 0,
        KeyCode::Char('e') => *cursor = chars.len(),
        KeyCode::Char('u') => {
            chars.drain(..*cursor);
            *cursor = 0;
        }
        KeyCode::Char('k') => {
            chars.truncate(*cursor);
        }
        KeyCode::Char('w') => {
            let end = *cursor;
            while *cursor > 0 && chars[*cursor - 1].is_whitespace() {
                *cursor -= 1;
            }
            while *cursor > 0 && !chars[*cursor - 1].is_whitespace() {
                *cursor -= 1;
            }
            chars.drain(*cursor..end);
        }
        _ => return false,
    }
    true
}

fn selected(ui: &Ui, store: &Store) -> Result<Value> {
    ui.selected(store).cloned().context("Select a task first")
}
fn target(ui: &mut Ui, task: &Value) -> Result<()> {
    ui.edit_target = Some((
        string(task, "id").into(),
        task["revision"].as_u64().context("Missing task revision")?,
    ));
    Ok(())
}
fn edit(
    ui: &mut Ui,
    store: &mut Store,
    mode: char,
    value: String,
    sender: &Sender<Job>,
) -> Result<()> {
    let (id, revision) = ui.edit_target.clone().context("Select a task first")?;
    let task = store.task_mut(&id)?;
    ensure!(
        task["revision"] == revision,
        "Task changed. Cancel and reopen the editor."
    );
    let field = match mode {
        'e' => "title",
        'd' => "body",
        'o' => "owner",
        'p' => "priority",
        _ => bail!("Unknown task field"),
    };
    if task["source"].is_null() {
        ensure!(
            value.len() <= if mode == 'd' { 20000 } else { 300 },
            "Value is too long"
        );
        ensure!(mode != 'e' || !value.trim().is_empty(), "Title is required");
        ensure!(
            mode != 'p' || matches!(value.as_str(), "high" | "normal" | "low"),
            "Choose high, normal, or low"
        );
        task[field] = json!(value);
        touch(task, "Updated local task");
        store.save()?;
        ui.message = "Task updated locally".into();
    } else {
        ui.pending_edit = Some((mode, value.clone()));
        ui.write_issue(task.clone(), field, value, sender)?;
    }
    Ok(())
}
fn prompt(
    ui: &mut Ui,
    store: &mut Store,
    mode: char,
    value: String,
    sender: &Sender<Job>,
) -> Result<()> {
    match mode {
        '/' => {
            ui.query = value;
            ui.row = 0
        }
        'C' => {
            let (id, _) = ui
                .edit_target
                .clone()
                .context("Choose a shared issue or PR")?;
            let task = store
                .tasks()
                .iter()
                .find(|task| task["id"] == id)
                .cloned()
                .context("Task is no longer cached")?;
            ui.comment(task, value, sender)?;
        }
        'e' | 'd' | 'o' | 'p' => edit(ui, store, mode, value, sender)?,
        'c' => {
            let project = local::add(store, PathBuf::from(value), None)?;
            ui.workspace = Some(local::WORKSPACE.into());
            ui.project = Some(project.id);
            ui.source = 0;
            ui.repo = 0;
            ui.column = 0;
            ui.row = 0;
            store.data["preferences"] = json!({"workspace":ui.workspace,"project":ui.project});
            store.save()?;
            ui.message = "Local directory project added / n new task / A agent".into();
        }
        'X' => {
            ensure!(
                value == "remove",
                "Type remove to forget the registration; files and tasks are retained"
            );
            let id = ui
                .project
                .as_deref()
                .context("Select a local project first")?;
            local::remove(store, id)?;
            ui.project = None;
            if local::projects(&store.data)?.is_empty() {
                ui.workspace = None;
            }
            store.data["preferences"] = json!({"workspace":ui.workspace,"project":ui.project});
            store.save()?;
            ui.message = "Registration removed. Files, tasks, and agent runs retained.".into();
        }
        'n' if ui.local_scope() => {
            let id = local::task(
                store,
                ui.project
                    .as_deref()
                    .context("Choose a local project with P first")?,
                &value,
            )?;
            store.task_mut(&id)?["stage"] = json!(STAGES[ui.column]);
            store.save()?;
            ui.source = 0;
            ui.query.clear();
            ui.row = ui
                .visible(store)
                .iter()
                .filter(|task| ui.my_work || task["stage"] == STAGES[ui.column])
                .count()
                .saturating_sub(1);
            ui.message = "Task saved in this local project".into();
        }
        'n' => {
            let repo = ui
                .repo
                .checked_sub(1)
                .and_then(|i| ui.repos(store).get(i).cloned())
                .context("Choose the target repository with Tab")?;
            ui.pending_edit = Some((mode, value.clone()));
            ui.create_issue(repo, value, sender)?;
        }
        'N' => {
            let repos = ui.repos(store);
            let repo = ui.repo.checked_sub(1).and_then(|i| repos.get(i));
            store.add(&value, repo.map(String::as_str), "")?;
            if let Some(task) = store.data["tasks"]
                .as_array_mut()
                .and_then(|t| t.last_mut())
            {
                task["stage"] = json!(STAGES[ui.column])
            }
            store.save()?;
            ui.source = 3;
            ui.query.clear();
            ui.agents = false;
            ui.detail = false;
            ui.row = ui
                .visible(store)
                .iter()
                .filter(|t| t["stage"] == STAGES[ui.column])
                .count()
                .saturating_sub(1);
            ui.message = "Unsent local draft. It has not been published to GCD.".into();
        }
        _ => bail!("Unknown input"),
    }
    Ok(())
}
pub(super) fn key(
    ui: &mut Ui,
    store: &mut Store,
    key: KeyEvent,
    sender: &Sender<Job>,
) -> Result<bool> {
    if ui.help {
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        let limit = render::help::scroll_limit(ui, width, height);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | '?') => ui.help = false,
            KeyCode::Down | KeyCode::Char('j') => {
                ui.help_scroll = ui.help_scroll.saturating_add(1).min(limit)
            }
            KeyCode::Up | KeyCode::Char('k') => ui.help_scroll = ui.help_scroll.saturating_sub(1),
            KeyCode::PageDown => ui.help_scroll = ui.help_scroll.saturating_add(8).min(limit),
            KeyCode::PageUp => ui.help_scroll = ui.help_scroll.saturating_sub(8),
            KeyCode::Home => ui.help_scroll = 0,
            KeyCode::End => ui.help_scroll = limit,
            _ => {}
        }
        return Ok(false);
    }
    if let Some(task) = ui.confirm_state.take() {
        match key.code {
            KeyCode::Enter => {
                if let Err(error) = ui.change_state(task.clone(), sender) {
                    ui.confirm_state = Some(task);
                    return Err(error);
                }
            }
            KeyCode::Esc => {}
            _ => ui.confirm_state = Some(task),
        }
        return Ok(false);
    }
    if let Some(mut picker) = ui.picker.take() {
        let choices = ui.choices(store, picker.kind, &picker.query);
        match key.code {
            KeyCode::Esc if picker.searching => picker.searching = false,
            KeyCode::Esc => return Ok(false),
            KeyCode::Char('/') if !picker.searching => picker.searching = true,
            KeyCode::Char('k') if !picker.searching => picker.row = picker.row.saturating_sub(1),
            KeyCode::Char('j') if !picker.searching => {
                picker.row = (picker.row + 1).min(choices.len().saturating_sub(1))
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.row = picker.row.saturating_sub(1)
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.row = (picker.row + 1).min(choices.len().saturating_sub(1))
            }
            KeyCode::Up => picker.row = picker.row.saturating_sub(1),
            KeyCode::Down => picker.row = (picker.row + 1).min(choices.len().saturating_sub(1)),
            KeyCode::Backspace => {
                picker.query.pop();
                picker.row = 0
            }
            KeyCode::Home => picker.row = 0,
            KeyCode::End => picker.row = choices.len().saturating_sub(1),
            KeyCode::Char(c)
                if picker.searching
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                picker.query.push(c);
                picker.row = 0
            }
            KeyCode::Enter => {
                let choice = choices.get(picker.row).context("No matching selection")?;
                match picker.kind {
                    PickKind::Actions => {
                        let task = selected(ui, store)?;
                        ensure!(
                            ui.edit_target
                                .as_ref()
                                .is_some_and(|(id, revision)| task["id"] == *id
                                    && task["revision"] == *revision),
                            "Task changed; reopen its action menu"
                        );
                        let action = choice
                            .id
                            .as_deref()
                            .and_then(|id| id.chars().next())
                            .context("Choose an action")?;
                        return self::key(
                            ui,
                            store,
                            KeyEvent::new(KeyCode::Char(action), KeyModifiers::NONE),
                            sender,
                        );
                    }
                    PickKind::Priority => {
                        let value = choice.id.clone().context("Choose a priority")?;
                        if let Err(error) = edit(ui, store, 'p', value, sender) {
                            ui.picker = Some(picker);
                            return Err(error);
                        }
                        return Ok(false);
                    }
                    PickKind::Source => {
                        ui.source = choice.id.as_deref().context("Choose a source")?.parse()?;
                        ui.row = 0;
                        ui.scroll = 0;
                        ui.detail = false;
                        ui.message = format!("Showing {} / f change shown items", choice.name);
                        return Ok(false);
                    }
                    PickKind::Workspace => {
                        ui.source = 0;
                        ui.workspace = choice.id.clone();
                        ui.project = None;
                        ui.repo = 0;
                    }
                    PickKind::Project => {
                        ui.source = 0;
                        ui.project = choice.id.clone();
                        ui.repo = 0;
                    }
                    PickKind::Repository => {
                        ui.repo = choice
                            .id
                            .as_ref()
                            .and_then(|r| ui.repos(store).iter().position(|n| n == r))
                            .map(|i| i + 1)
                            .unwrap_or(0);
                    }
                }
                ui.row = 0;
                ui.query.clear();
                ui.scroll = 0;
                ui.detail = false;
                store.data["preferences"] = json!({"workspace":ui.workspace,"project":ui.project});
                store.save()?;
                if !ui.offline && ui.syncing == 0 && ui.workspace.is_some() {
                    ui.start_sync(store, sender)?;
                }
                return Ok(false);
            }
            _ => {}
        }
        ui.picker = Some(picker);
        return Ok(false);
    }
    if let Some(mut stage) = ui.moving {
        match key.code {
            KeyCode::Esc => ui.moving = None,
            KeyCode::Up | KeyCode::Char('k') => ui.moving = Some(stage.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => ui.moving = Some((stage + 1).min(4)),
            KeyCode::Char(c) if ('1'..='5').contains(&c) => {
                ui.moving = Some(c as usize - '1' as usize)
            }
            KeyCode::Enter => {
                stage = stage.min(4);
                let (id, revision) = ui.edit_target.clone().context("Select a task")?;
                let task = store.task_mut(&id)?;
                ensure!(
                    task["revision"] == revision,
                    "Task changed; reopen the stage picker"
                );
                if task["source"].is_null() {
                    task["stage"] = json!(STAGES[stage]);
                    touch(task, "Moved local task");
                    store.save()?;
                    ui.column = stage;
                    ui.message = "Task moved locally".into();
                } else {
                    ui.write_issue(task.clone(), "stage", STAGES[stage].into(), sender)?;
                }
                ui.moving = None;
            }
            _ => {}
        }
        return Ok(false);
    }
    if let Some((mode, value)) = ui.prompt.take() {
        let mut chars = value.chars().collect::<Vec<_>>();
        ui.cursor = ui.cursor.min(chars.len());
        let edited = shell_edit(&mut chars, &mut ui.cursor, key);
        let mut value = if edited {
            chars.iter().collect()
        } else {
            value
        };
        let mut retain = true;
        match key.code {
            _ if edited => {}
            KeyCode::Esc => {
                if mode == '/' {
                    ui.query = ui.search_before.clone();
                    ui.row = 0;
                }
                retain = false;
            }
            KeyCode::Enter
                if matches!(mode, 'd' | 'C') && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                chars.insert(ui.cursor, '\n');
                ui.cursor += 1;
                value = chars.iter().collect();
            }
            KeyCode::Enter | KeyCode::Char('s')
                if key.code == KeyCode::Enter || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                match prompt(ui, store, mode, value.clone(), sender) {
                    Ok(()) => retain = false,
                    Err(error) => {
                        ui.prompt = Some((mode, value));
                        return Err(error);
                    }
                }
            }
            KeyCode::Tab if matches!(mode, 'n' | 'N') => {
                let count = ui.repos(store).len();
                ensure!(count > 0, "This project has no repositories");
                ui.repo = ui.repo % count + 1;
            }
            KeyCode::Left => ui.cursor = ui.cursor.saturating_sub(1),
            KeyCode::Right => ui.cursor = (ui.cursor + 1).min(chars.len()),
            KeyCode::Home => ui.cursor = 0,
            KeyCode::End => ui.cursor = chars.len(),
            KeyCode::Backspace if ui.cursor > 0 => {
                chars.remove(ui.cursor - 1);
                ui.cursor -= 1;
                value = chars.iter().collect();
            }
            KeyCode::Delete if ui.cursor < chars.len() => {
                chars.remove(ui.cursor);
                value = chars.iter().collect();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                chars.insert(ui.cursor, c);
                ui.cursor += 1;
                value = chars.iter().collect();
            }
            _ => {}
        }
        if retain {
            if mode == '/' {
                ui.query = value.clone();
                ui.row = 0;
            }
            ui.prompt = Some((mode, value));
        }
        return Ok(false);
    }
    if key.modifiers == KeyModifiers::CONTROL {
        match key.code {
            KeyCode::Char('d') => {
                if ui.detail {
                    ui.scroll = ui.scroll.saturating_add(6);
                } else {
                    ui.row = ui.row.saturating_add(6);
                }
                return Ok(false);
            }
            KeyCode::Char('u') => {
                if ui.detail {
                    ui.scroll = ui.scroll.saturating_sub(6);
                } else {
                    ui.row = ui.row.saturating_sub(6);
                }
                return Ok(false);
            }
            _ => {}
        }
    }
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Ok(false);
    }
    if key.code == KeyCode::Char('!') {
        ui.error_popup = ui.error;
        return Ok(false);
    }
    ui.error = false;
    match key.code {
        KeyCode::Char('q') => {
            ensure!(
                !ui.writing,
                "Saving on GCD. Wait for confirmation before quitting."
            );
            return Ok(true);
        }
        KeyCode::Esc => {
            ui.detail = false;
            ui.query.clear();
            ui.scroll = 0;
        }
        KeyCode::Char('?') => {
            ui.help = true;
            ui.help_scroll = 0;
        }
        KeyCode::Char('c' | 'C' | 'z') => {
            ensure!(!ui.offline, "Offline mode cannot write to GCD");
            ensure!(!ui.writing, "A GCD write is already running");
            let task = selected(ui, store)?;
            ensure!(
                matches!(string(&task["source"], "kind"), "issue" | "pr"),
                "Choose a shared issue or PR; use m for local task stages"
            );
            if matches!(key.code, KeyCode::Char('c' | 'C')) {
                target(ui, &task)?;
                ui.open_prompt('C', String::new());
            } else {
                ensure!(
                    task["source"]["state"] != "MERGED",
                    "A merged PR cannot be reopened"
                );
                ui.confirm_state = Some(task);
            }
        }
        KeyCode::Char('v') => ui.board = !ui.board,
        KeyCode::Char('u') => ui.toggle_work(store, sender)?,
        KeyCode::Char('w') => ui.open_picker(PickKind::Workspace),
        KeyCode::Char('P') => {
            ensure!(ui.workspace.is_some(), "Choose a workspace with w first");
            ui.open_picker(PickKind::Project)
        }
        KeyCode::Char('r') => ui.open_picker(PickKind::Repository),
        KeyCode::Char('+') => ui.open_prompt('c', String::new()),
        KeyCode::Char('X') if ui.local_scope() => {
            ensure!(ui.project.is_some(), "Choose a local project with P first");
            ensure!(
                ui.active_runs == 0,
                "Stop active agents before forgetting a project"
            );
            ui.open_prompt('X', String::new());
        }
        KeyCode::Char('G') => {
            ensure!(
                !ui.local_scope(),
                "Local directory projects are managed here; + adds a directory"
            );
            let suffix = if let Some(slug) = &ui.workspace {
                ensure!(
                    slug.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)),
                    "Invalid workspace identity"
                );
                format!("/{slug}.yaml")
            } else {
                String::new()
            };
            let status = Command::new("xdg-open")
                .arg(format!(
                    "https://git.cmu.dev/ScottyLabs/governance/src/branch/main/data/teams{suffix}"
                ))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .context("Open Governance declaration")?;
            ensure!(status.success(), "Browser launch failed");
            ui.message = "Opened canonical Governance declaration".into();
        }
        KeyCode::Char('g') => ui.start_discovery(sender)?,
        KeyCode::Char('f') => ui.open_picker(PickKind::Source),
        KeyCode::Char('a') => {
            ui.agents = !ui.agents;
            ui.row = 0
        }
        KeyCode::Char('h') | KeyCode::Left => {
            ui.column = ui.column.saturating_sub(1);
            ui.row = 0;
            ui.scroll = 0
        }
        KeyCode::Char('l') | KeyCode::Right => {
            ui.column = (ui.column + 1).min(4);
            ui.row = 0;
            ui.scroll = 0
        }
        KeyCode::Home => {
            ui.row = 0;
            ui.scroll = 0;
        }
        KeyCode::End if !ui.detail => {
            ui.row = ui
                .visible(store)
                .iter()
                .filter(|task| task["stage"] == STAGES[ui.column])
                .count()
                .saturating_sub(1);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if ui.detail {
                ui.scroll = ui.scroll.saturating_add(1)
            } else {
                ui.row += 1
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if ui.detail {
                ui.scroll = ui.scroll.saturating_sub(1)
            } else {
                ui.row = ui.row.saturating_sub(1)
            }
        }
        KeyCode::PageDown => {
            if ui.detail {
                ui.scroll = ui.scroll.saturating_add(12)
            } else {
                ui.row += 8
            }
        }
        KeyCode::PageUp => {
            if ui.detail {
                ui.scroll = ui.scroll.saturating_sub(12)
            } else {
                ui.row = ui.row.saturating_sub(8)
            }
        }
        KeyCode::Enter => {
            ui.detail = !ui.detail;
            ui.scroll = 0
        }
        KeyCode::Tab => {
            ui.repo = (ui.repo + 1) % (ui.repos(store).len() + 1);
            ui.row = 0
        }
        KeyCode::BackTab => {
            let count = ui.repos(store).len() + 1;
            ui.repo = (ui.repo + count - 1) % count;
            ui.row = 0
        }
        KeyCode::Char(c) if ('1'..='5').contains(&c) => {
            ui.column = c as usize - '1' as usize;
            ui.row = 0
        }
        KeyCode::Char('/') => {
            ui.search_before = ui.query.clone();
            ui.open_prompt('/', ui.query.clone())
        }
        KeyCode::Char(c @ ('n' | 'N')) => {
            ensure!(!ui.writing, "A GCD write is already running");
            if c == 'n' && !ui.local_scope() {
                ensure!(
                    !ui.repos(store).is_empty(),
                    "This project has no repositories; choose another project with P"
                );
                if ui.repo == 0 {
                    ui.repo = 1
                }
            }
            ui.open_prompt(c, String::new());
        }
        KeyCode::Char(' ') => {
            let task = selected(ui, store)?;
            target(ui, &task)?;
            ui.open_picker(PickKind::Actions);
        }
        KeyCode::Char(c @ ('e' | 'd' | 'o' | 'p')) => {
            ensure!(!ui.writing, "A GCD write is already running");
            let task = selected(ui, store)?;
            target(ui, &task)?;
            let field = match c {
                'e' => "title",
                'd' => "body",
                'o' => "owner",
                _ => "priority",
            };
            if c == 'p' {
                ui.open_priority(string(&task, "priority"));
                return Ok(false);
            }
            let value = string(&task, field).into();
            ui.open_prompt(c, value);
        }
        KeyCode::Char('m') => {
            ensure!(!ui.writing, "A GCD write is already running");
            let task = selected(ui, store)?;
            ensure!(
                task["source"].is_null() || task["source"]["kind"] == "issue",
                "PR state comes from GCD; open the PR with O"
            );
            target(ui, &task)?;
            ui.moving = Some(ui.column)
        }
        KeyCode::Char('s') => {
            if ui.my_work {
                ui.personal_refresh(sender)?;
            } else {
                ui.start_sync(store, sender)?;
            }
        }
        KeyCode::Char('O') => {
            let task = selected(ui, store)?;
            let repo = string(&task, "repo");
            ensure!(repo_valid(repo), "Task has no GCD repository");
            let number = task["source"]["number"]
                .as_u64()
                .context("Unsent drafts have no GCD source")?;
            let url = format!(
                "https://git.cmu.dev/{repo}/{}/{number}",
                if task["source"]["kind"] == "pr" {
                    "pulls"
                } else {
                    "issues"
                }
            );
            let status = Command::new("xdg-open")
                .arg(url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .context("Open browser")?;
            ensure!(status.success(), "Browser launch failed");
            ui.message = "Opened GCD source".into();
        }
        KeyCode::Char('b') => {
            let task = selected(ui, store)?;
            let id = string(&task, "id");
            let path = store
                .dir
                .join(format!("brief-{}.json", id.replace(['/', ':'], "_")));
            let mut file = tempfile::NamedTempFile::new_in(&store.dir)?;
            serde_json::to_writer_pretty(&mut file, &brief(store, id)?)?;
            file.persist(&path)?;
            ui.message = format!("Brief saved: {}", path.display());
        }
        KeyCode::Char(c @ ('y' | 'x')) => {
            ensure!(ui.detail, "Open task details to review the plan first");
            let task = selected(ui, store)?;
            plan(store, string(&task, "id"), c == 'x')?;
            store.save()?;
            ui.message = "Plan reviewed locally. Accepted subtasks are unsent drafts.".into();
        }
        _ => {}
    }
    Ok(false)
}

pub(super) fn paste(ui: &mut Ui, text: &str) -> Result<()> {
    let (value, cursor, maximum) = if let Some(form) = ui.launch.as_mut() {
        if form.field < 2 || (form.field == 2 && form.resume.is_some()) {
            return Ok(());
        }
        (
            if form.field == 2 {
                &mut form.checkout
            } else {
                &mut form.goal
            },
            &mut form.cursor,
            6000usize,
        )
    } else if let Some((mode, value)) = ui.prompt.as_mut() {
        (
            value,
            &mut ui.cursor,
            if matches!(*mode, 'C' | 'd') {
                20000
            } else {
                4096
            },
        )
    } else {
        return Ok(());
    };
    ensure!(
        value.len() + text.len() <= maximum,
        "Pasted text is too long"
    );
    let text = text.replace("\r\n", "\n");
    let mut chars = value.chars().collect::<Vec<_>>();
    let inserted = text
        .chars()
        .filter(|character| !character.is_control() || matches!(*character, '\n' | '\t'))
        .collect::<Vec<_>>();
    *cursor = (*cursor).min(chars.len());
    let count = inserted.len();
    chars.splice(*cursor..*cursor, inserted);
    *cursor += count;
    *value = chars.into_iter().collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_editing_preserves_text_after_cursor() {
        let mut chars = "one two tail".chars().collect::<Vec<_>>();
        let mut cursor = 7;
        assert!(shell_edit(
            &mut chars,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
        ));
        assert_eq!(chars.iter().collect::<String>(), "one  tail");
        assert_eq!(cursor, 4);
        shell_edit(
            &mut chars,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert_eq!(chars.iter().collect::<String>(), " tail");
        assert_eq!(cursor, 0);
        shell_edit(
            &mut chars,
            &mut cursor,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        );
        assert_eq!(cursor, 5);
    }

    #[test]
    fn pickers_accept_j_and_k_as_search_text() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path().into())?;
        let (sender, _receiver) = mpsc::channel();
        let mut ui = Ui::default();
        ui.open_picker(PickKind::Workspace);
        key(
            &mut ui,
            &mut store,
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            &sender,
        )?;
        for letter in ['j', 'k'] {
            key(
                &mut ui,
                &mut store,
                KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE),
                &sender,
            )?;
        }
        assert_eq!(
            ui.picker.as_ref().map(|picker| picker.query.as_str()),
            Some("jk")
        );
        Ok(())
    }
    #[test]
    fn opening_error_details_preserves_the_error() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path().into())?;
        let (sender, _receiver) = mpsc::channel();
        let mut ui = Ui {
            error: true,
            message: "Repository unavailable".into(),
            ..Ui::default()
        };
        key(
            &mut ui,
            &mut store,
            KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
            &sender,
        )?;
        assert!(ui.error_popup);
        assert!(ui.error);
        assert_eq!(ui.message, "Repository unavailable");
        Ok(())
    }
    #[test]
    fn close_requires_confirmation_and_can_be_cancelled_without_writing() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let mut store = Store::open(temporary.path().into())?;
        governance::apply(
            &mut store,
            governance::from_records(governance::tests::records())?,
        )?;
        import_records(
            &mut store,
            "ScottyLabs/infrastructure",
            &[json!({"number":1,"title":"Task","state":"OPEN"})],
            &[],
        )?;
        let (sender, receiver) = mpsc::channel();
        let mut ui = Ui {
            source: 1,
            ..Ui::default()
        };
        key(
            &mut ui,
            &mut store,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            &sender,
        )?;
        assert!(ui.confirm_state.is_none());
        key(
            &mut ui,
            &mut store,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
            &sender,
        )?;
        assert!(ui.confirm_state.is_some());
        assert!(!ui.writing);
        assert!(receiver.try_recv().is_err());
        key(
            &mut ui,
            &mut store,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &sender,
        )?;
        assert!(ui.confirm_state.is_none());
        assert!(receiver.try_recv().is_err());
        Ok(())
    }
    #[test]
    fn normal_quit_waits_for_pending_remote_write() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path().into())?;
        let (sender, _receiver) = mpsc::channel();
        let mut ui = Ui {
            writing: true,
            ..Ui::default()
        };
        let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(key(&mut ui, &mut store, quit, &sender).is_err());
        ui.writing = false;
        assert!(key(&mut ui, &mut store, quit, &sender)?);
        Ok(())
    }
}
