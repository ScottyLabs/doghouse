mod agents;
pub(super) mod help;
mod markdown;
use super::*;
use ratatui::{
    layout::Rect,
    widgets::{BorderType, Clear, List, ListItem, ListState, Padding},
};

const BG: Color = Color::Reset;
const PANEL: Color = Color::Reset;
const EDGE: Color = Color::DarkGray;
const TEXT: Color = Color::Reset;
const MUTED: Color = Color::DarkGray;
const BLUE: Color = Color::Blue;
const GREEN: Color = Color::Green;
const AMBER: Color = Color::Yellow;
fn style(color: Color) -> Style {
    Style::default().fg(color)
}
fn label(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), style(color))
}
fn block(title: impl Into<String>, active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style(if active { BLUE } else { EDGE }))
        .style(style(TEXT).bg(PANEL))
        .title(Line::from(label(
            format!(" {} ", title.into()),
            if active { BLUE } else { MUTED },
        )))
}
fn reference(task: &Value) -> String {
    if let Some(n) = task["source"]["number"].as_u64() {
        format!(
            "{}{}",
            if task["source"]["kind"] == "pr" {
                "PR "
            } else {
                "#"
            },
            n
        )
    } else if task["localProject"].is_string() {
        "LOCAL".into()
    } else {
        "DRAFT".into()
    }
}
fn stage_color(stage: usize) -> Color {
    [BLUE, Color::Cyan, AMBER, Color::Magenta, GREEN][stage]
}
fn source_color(task: &Value) -> Color {
    if task["source"]["kind"] == "pr" {
        Color::Magenta
    } else if task["source"].is_null() {
        Color::Cyan
    } else {
        BLUE
    }
}
fn priority(task: &Value) -> (&str, Color) {
    match string(task, "priority") {
        "high" => ("HIGH", AMBER),
        "low" => ("LOW", MUTED),
        _ => ("normal", MUTED),
    }
}
fn short_repo(task: &Value) -> String {
    if task["localProject"].is_string() {
        return PathBuf::from(string(task, "directory"))
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
    }
    string(task, "repo")
        .rsplit('/')
        .next()
        .unwrap_or("workspace")
        .to_owned()
}
fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(4));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}
fn section(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push(Line::raw(""));
    lines.push(Line::from(label(title.to_owned(), BLUE)));
    lines.push(Line::raw(""));
}
fn freshness(value: &Value) -> String {
    let Some(stamp) = value.as_str().and_then(|s| s.parse::<u128>().ok()) else {
        return "cached / refresh to verify".into();
    };
    let current = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seconds = current.saturating_sub(stamp) / 1_000_000_000;
    if seconds < 60 {
        "read just now".into()
    } else if seconds < 3600 {
        format!("read {}m ago", seconds / 60)
    } else {
        format!("read {}h ago", seconds / 3600)
    }
}
fn detail_lines(task: &Value, store: &Store, width: u16) -> Vec<Line<'static>> {
    let (p, color) = priority(task);
    let mut lines = vec![
        Line::styled(
            clean(string(task, "title")),
            style(TEXT).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![
            label(reference(task), source_color(task)),
            label(format!("  {}", short_repo(task)), MUTED),
        ]),
        Line::from(vec![
            label(format!("{}  ", string(task, "stage")), GREEN),
            label(format!("{p} priority"), color),
        ]),
        Line::from(label(
            format!(
                "Owner  {}",
                if string(task, "owner").is_empty() {
                    "unassigned"
                } else {
                    string(task, "owner")
                }
            ),
            MUTED,
        )),
    ];
    if task["source"].is_object() {
        lines.push(Line::from(label(
            format!(
                "GCD {}  /  {}",
                string(&task["source"], "state"),
                freshness(&task["source"]["syncedAt"])
            ),
            MUTED,
        )))
    } else {
        lines.push(Line::from(label(
            if task["localProject"].is_string() {
                "Local task / stored only on this machine"
            } else {
                "Unsent draft / stored only on this machine"
            },
            MUTED,
        )))
    }
    section(&mut lines, "DESCRIPTION");
    if string(task, "body").is_empty() {
        lines.push(Line::from(label("No description.", MUTED)))
    }
    lines.extend(markdown::render(string(task, "body"), width));
    let children: Vec<_> = store
        .tasks()
        .iter()
        .filter(|t| t["parentId"] == task["id"])
        .collect();
    if !children.is_empty() {
        section(&mut lines, "SUBTASKS");
        for t in children {
            lines.push(Line::from(vec![
                label(
                    format!("[{}] ", if t["stage"] == "done" { "x" } else { " " }),
                    GREEN,
                ),
                label(clean(string(t, "title")), TEXT),
            ]))
        }
    }
    if task["claim"].is_object() {
        section(&mut lines, "LOCAL AGENT NOTES");
        lines.push(Line::from(label(
            format!(
                "{} is working on this task",
                clean(string(&task["claim"], "agent"))
            ),
            AMBER,
        )))
    }
    if task["report"].is_object() {
        section(&mut lines, "LOCAL RESULT TO REVIEW");
        for line in format!(
            "{}\n\n{}",
            string(&task["report"], "summary"),
            string(&task["report"], "evidence")
        )
        .lines()
        {
            lines.push(Line::raw(clean(line)))
        }
    }
    if let Some(proposals) = task["proposals"].as_array() {
        for p in proposals {
            section(&mut lines, &format!("PLAN / {}", string(p, "status")));
            if p["status"] == "pending" {
                lines.push(Line::from(label(
                    "y accept as unsent drafts   x decline",
                    AMBER,
                )))
            }
            if let Some(summary) = p["summary"].as_str() {
                lines.push(Line::raw(clean(summary)))
            }
            if let Some(tasks) = p["tasks"].as_array() {
                for t in tasks {
                    lines.push(Line::raw(format!("- {}", clean(string(t, "title")))))
                }
            }
        }
    }
    if task["source"]["kind"] == "issue" {
        let related = store
            .tasks()
            .iter()
            .filter(|p| {
                p["repo"] == task["repo"]
                    && p["source"]["kind"] == "pr"
                    && p["source"]["closes"]
                        .as_array()
                        .is_some_and(|refs| refs.contains(&task["source"]["number"]))
            })
            .collect::<Vec<_>>();
        if !related.is_empty() {
            section(&mut lines, "RELATED CHANGES");
            for pr in related {
                lines.push(Line::from(vec![
                    label(
                        format!(
                            "PR {} / {}  ",
                            pr["source"]["number"],
                            string(&pr["source"], "state")
                        ),
                        Color::Magenta,
                    ),
                    label(clean(string(pr, "title")), TEXT),
                ]));
            }
        }
    }
    if let Some(reports) = task["source"]["comments"]
        .as_array()
        .or_else(|| task["source"]["agent_reports"].as_array())
        .filter(|r| !r.is_empty())
    {
        section(&mut lines, "DISCUSSION / GCD COMMENTS");
        for report in reports {
            lines.push(Line::from(label(
                report["user"]["login"]
                    .as_str()
                    .or_else(|| report["author"]["login"].as_str())
                    .unwrap_or("Unknown author")
                    .to_string(),
                BLUE,
            )));
            let body = string(report, "body");
            let body = if body.starts_with("<!-- shepherd-run:") {
                body.split_once("-->")
                    .map(|(_, body)| body.trim())
                    .unwrap_or(body)
            } else {
                body
            };
            lines.extend(markdown::render(body, width));
            lines.push(Line::raw(""));
        }
    }
    if let Some(events) = task["events"]
        .as_array()
        .filter(|events| !events.is_empty())
    {
        section(&mut lines, "RECENT ACTIVITY");
        for e in events.iter().rev().take(5) {
            lines.push(Line::from(label(clean(string(e, "message")), MUTED)))
        }
    }
    lines
}
fn inspector(ui: &Ui, f: &mut ratatui::Frame, store: &Store, area: Rect, expanded: bool) {
    let container = block(
        if expanded {
            "Task detail · Esc back · j/k scroll"
        } else {
            "Preview · Enter expand"
        },
        expanded,
    )
    .padding(Padding::horizontal(1));
    if let Some(task) = ui.selected(store) {
        f.render_widget(
            Paragraph::new(detail_lines(task, store, area.width.saturating_sub(4)))
                .wrap(Wrap { trim: false })
                .scroll((if expanded { ui.scroll } else { 0 }, 0))
                .block(container),
            area,
        )
    } else {
        let message = if ui.local_scope() {
            "Local directory project.\n\nn  Add a local task\nA  Start an agent for a task\n+  Add a directory\nP  Choose a project"
        } else if store.catalog.workspaces.is_empty() {
            "Governance has not loaded.\n\ng  Refresh through Collar\nw  Choose a cached workspace"
        } else if ui.repos(store).is_empty() {
            "This Governance project has no repositories.\n\nG  Open its Governance declaration\nP  Choose another project\nw  Choose a workspace"
        } else {
            "No task selected.\n\nn  Create a GCD issue\ns  Refresh shared tasks\n/  Search this scope"
        };
        f.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .style(style(MUTED))
                .block(container),
            area,
        );
    }
}
fn task_list(ui: &Ui, f: &mut ratatui::Frame, store: &Store, area: Rect) {
    let items: Vec<_> = ui
        .visible(store)
        .into_iter()
        .filter(|t| ui.my_work || t["stage"] == STAGES[ui.column])
        .collect();
    let title = format!(
        "{} / {} tasks",
        if ui.my_work {
            "My work"
        } else {
            NAMES[ui.column]
        },
        items.len()
    );
    if items.is_empty() {
        f.render_widget(Paragraph::new(if ui.my_work {if ui.personal_loading {"\n Reading your assignments, PRs, and review requests through Collar..."} else {"\n No items in this My work view.\n\n s  Refresh your account inbox\n f  Change source filter\n u  All accessible projects"}} else if ui.local_scope(){"\n No local tasks in this view.\n\n n  Add a task\n +  Add a directory\n P  Choose a local project"}else if ui.repos(store).is_empty(){"\n No repositories declared in this project.\n\n P  Choose a project\n G  Open Governance declaration"}else{"\n No tasks in this view.\n\n n  Create a GCD issue\n s  Refresh shared tasks\n h/l  Browse stages\n Esc  Clear search"}).style(style(MUTED)).block(block(title,true).padding(Padding::horizontal(1))),area);
        return;
    }
    let rows: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(index, t)| {
            let (p, color) = priority(t);
            let mut meta = vec![
                label(format!("{:<8} ", reference(t)), source_color(t)),
                label(short_repo(t), MUTED),
                label(
                    if p == "normal" {
                        String::new()
                    } else {
                        format!("  {p}")
                    },
                    color,
                ),
            ];
            if !string(t, "owner").is_empty() {
                meta.push(label(format!("  @{}", clean(string(t, "owner"))), GREEN))
            }
            if ui.my_work
                && let Some(roles) = store.data["myWork"]["roles"][string(t, "id")].as_array()
            {
                meta.push(label(
                    format!(
                        "  {}",
                        roles
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" / ")
                    ),
                    BLUE,
                ));
            }
            if let Some(todos) = t["source"]["todos"]
                .as_array()
                .filter(|todos| !todos.is_empty())
            {
                meta.push(label(
                    format!(
                        "  {}/{} todos",
                        todos.iter().filter(|t| t["done"] == true).count(),
                        todos.len()
                    ),
                    MUTED,
                ))
            }

            if t["proposals"]
                .as_array()
                .is_some_and(|p| p.iter().any(|p| p["status"] == "pending"))
            {
                meta.push(label("  PLAN", AMBER))
            }
            if t["claim"].is_object() {
                meta.push(label("  AGENT", AMBER))
            }
            let title = clean(string(t, "title"));
            let selected = index == ui.row;
            let width = usize::from(area.width.saturating_sub(6));
            let padding = width.saturating_sub(Line::raw(title.clone()).width());
            let title = if selected {
                format!("{title}{}", " ".repeat(padding))
            } else {
                title
            };
            ListItem::new(vec![
                Line::styled(
                    title,
                    if selected {
                        style(TEXT).add_modifier(Modifier::REVERSED | Modifier::BOLD)
                    } else {
                        style(TEXT)
                    },
                ),
                Line::from(meta),
                Line::raw(""),
            ])
        })
        .collect();
    let mut selected = ListState::default().with_selected(Some(ui.row));
    f.render_stateful_widget(
        List::new(rows)
            .block(block(title, true).padding(Padding::horizontal(1)))
            .highlight_symbol("▎ "),
        area,
        &mut selected,
    );
}
fn board(ui: &Ui, f: &mut ratatui::Frame, store: &Store, area: Rect) {
    let visible = ui.visible(store);
    let stages: Vec<_> = if area.width >= 140 {
        (0..5).collect()
    } else {
        vec![ui.column]
    };
    let columns = Layout::horizontal(vec![
        Constraint::Ratio(1, stages.len() as u32);
        stages.len()
    ])
    .spacing(1)
    .split(area);
    for (position, stage) in stages.into_iter().enumerate() {
        let items: Vec<_> = visible
            .iter()
            .filter(|t| t["stage"] == STAGES[stage])
            .collect();
        let capacity = usize::from(area.height.saturating_sub(2) / 4).max(1);
        let start = if stage == ui.column {
            ui.row.saturating_sub(capacity - 1)
        } else {
            0
        };
        let width = usize::from(columns[position].width.saturating_sub(5)).max(1);
        let mut lines = vec![];
        for (i, t) in items.iter().enumerate().skip(start).take(capacity) {
            let selected = stage == ui.column && i == ui.row;
            let base = if selected {
                style(BLUE).add_modifier(Modifier::BOLD)
            } else {
                style(TEXT)
            };
            lines.push(
                Line::from(vec![
                    label(
                        format!("{} {}  ", if selected { ">" } else { " " }, reference(t)),
                        BLUE,
                    ),
                    label(short_repo(t), MUTED),
                ])
                .style(base),
            );
            let words = title_rows(string(t, "title"), width);
            for n in 0..2 {
                lines.push(Line::styled(
                    format!(" {}", words.get(n).map(String::as_str).unwrap_or("")),
                    base,
                ))
            }
            lines.push(Line::raw(""));
        }
        if items.is_empty() {
            lines.push(Line::from(label(" No tasks", MUTED)))
        }
        f.render_widget(
            Paragraph::new(lines).block(block(
                format!("{}  {}", NAMES[stage], items.len()),
                stage == ui.column,
            )),
            columns[position],
        );
    }
}
fn title_rows(title: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in clean(title).split_whitespace() {
        if !row.is_empty() && row.chars().count() + 1 + word.chars().count() > width {
            rows.push(std::mem::take(&mut row));
        }
        for part in word.chars().collect::<Vec<_>>().chunks(width.max(1)) {
            let part: String = part.iter().collect();
            if !row.is_empty() {
                row.push(' ')
            }
            row.push_str(&part);
            if row.chars().count() >= width {
                rows.push(std::mem::take(&mut row));
            }
        }
    }
    if !row.is_empty() {
        rows.push(row)
    }
    if rows.len() > 2 {
        rows.truncate(2);
        if let Some(last) = rows.last_mut() {
            while last.chars().count() > width.saturating_sub(3) {
                last.pop();
            }
            last.push_str(&".".repeat(width.min(3)))
        }
    }
    rows
}
pub(super) fn draw(ui: &Ui, f: &mut ratatui::Frame, store: &Store) {
    let area = f.area();
    f.render_widget(Block::default().style(style(TEXT).bg(BG)), area);
    if area.width < 60 || area.height < 16 {
        f.render_widget(
            Paragraph::new("Shepherd\nResize to 60 columns and 16 rows.\nq quit")
                .style(style(TEXT)),
            area,
        );
        return;
    }
    let regions = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(3),
    ])
    .margin(1)
    .split(area);
    let repos = ui.repos(store);
    let selected_repo = ui.repo.checked_sub(1).and_then(|i| repos.get(i));
    let repo = selected_repo
        .map(String::as_str)
        .unwrap_or(ui.workspace_name(store));
    let visible = ui.visible(store);
    let open = visible.iter().filter(|t| t["stage"] != "done").count();
    let source = if ui.local_scope() && ui.source == 1 {
        "Local tasks"
    } else {
        ["Issues + PRs", "Issues", "Pull requests", "Unsent drafts"][ui.source]
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("SHEPHERD", style(BLUE).add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled(
                    "All accessible",
                    if !ui.my_work {
                        style(BLUE).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else {
                        style(MUTED)
                    },
                ),
                Span::raw("   "),
                Span::styled(
                    "My work",
                    if ui.my_work {
                        style(BLUE).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else {
                        style(MUTED)
                    },
                ),
                label("  [u]  /  ", MUTED),
                label(
                    if ui.my_work {
                        format!("@{}", string(&store.data["myWork"], "login"))
                    } else {
                        format!("{} / {}", clean(repo), clean(ui.project_name(store)))
                    },
                    TEXT,
                ),
            ]),
            Line::from(vec![
                label(
                    if ui.agent_view {
                        format!(
                            "{} runs / {} active total / {}",
                            ui.visible_runs(store).len(),
                            ui.active_runs,
                            if ui.run_all {
                                "all projects"
                            } else {
                                "current scope"
                            }
                        )
                    } else {
                        format!(
                            "{open} open / {source}{}",
                            if ui.personal_loading {
                                " / account refreshing"
                            } else {
                                ""
                            }
                        )
                    },
                    MUTED,
                ),
                label(if ui.agents { "  ·  agent work" } else { "" }, AMBER),
                label(
                    if ui.query.is_empty() {
                        String::new()
                    } else {
                        format!("  / {}", clean(&ui.query))
                    },
                    BLUE,
                ),
                label(
                    if ui.discovering {
                        "  /  Governance refreshing".into()
                    } else if ui.offline {
                        "  /  OFFLINE cache".into()
                    } else if ui.writing {
                        "  /  saving to GCD".into()
                    } else if ui.syncing > 0 {
                        format!("  ·  syncing {} repos", ui.syncing)
                    } else {
                        String::new()
                    },
                    AMBER,
                ),
            ]),
        ]),
        regions[0],
    );
    if ui.agent_view {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                label("AGENTS / THIS MACHINE  ", BLUE),
                label(format!("{} active  /  ", ui.active_runs), MUTED),
                label("Codex ", BLUE),
                label(
                    if crate::agents::Provider::Codex.available() {
                        "available  "
                    } else {
                        "not installed  "
                    },
                    if crate::agents::Provider::Codex.available() {
                        GREEN
                    } else {
                        MUTED
                    },
                ),
                label("Claude Code ", Color::Magenta),
                label(
                    if crate::agents::Provider::Claude.available() {
                        "available"
                    } else {
                        "not installed"
                    },
                    if crate::agents::Provider::Claude.available() {
                        GREEN
                    } else {
                        MUTED
                    },
                ),
            ]))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(style(EDGE)),
            ),
            regions[1],
        );
    } else if ui.my_work {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                label("MY WORK", BLUE),
                label("  Assignments / your PRs / review requests    ", MUTED),
                label("u  All accessible", TEXT),
                label(
                    if ui.personal_loading {
                        " / refreshing"
                    } else {
                        ""
                    },
                    AMBER,
                ),
            ]))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(style(EDGE)),
            ),
            regions[1],
        );
    } else {
        let stage_names = if area.width < 100 {
            ["Backlog", "Ready", "Active", "Review", "Done"]
        } else {
            NAMES
        };
        let spans: Vec<Span> = stage_names
            .iter()
            .enumerate()
            .flat_map(|(i, name)| {
                let count = visible.iter().filter(|t| t["stage"] == STAGES[i]).count();
                let s = if i == ui.column {
                    style(stage_color(i)).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    style(stage_color(i))
                };
                vec![
                    Span::styled(
                        if i == 4 {
                            format!(" {} {name} ", i + 1)
                        } else {
                            format!(" {} {name} {count} ", i + 1)
                        },
                        s,
                    ),
                    Span::raw(" "),
                ]
            })
            .collect();
        f.render_widget(
            Paragraph::new(Line::from(spans)).block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(style(EDGE)),
            ),
            regions[1],
        );
    }
    let with_sidebar = area.width >= 125 && !ui.detail;
    let body = if with_sidebar {
        let split = Layout::horizontal([Constraint::Length(26), Constraint::Min(60)])
            .spacing(1)
            .split(regions[2]);
        let mut lines = vec![
            Line::from(label("WORKSPACE  [w]", MUTED)),
            Line::raw(""),
            Line::styled(
                clean(ui.workspace_name(store)),
                style(BLUE).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::from(label("PROJECTS  [P]", MUTED)),
            Line::raw(""),
        ];
        lines.push(Line::styled(
            "  All projects",
            if ui.project.is_none() {
                style(BLUE).add_modifier(Modifier::BOLD)
            } else {
                style(MUTED)
            },
        ));
        for team in store
            .catalog
            .workspaces
            .iter()
            .filter(|w| ui.workspace.as_ref().is_some_and(|s| w.slug == *s))
        {
            for project in &team.projects {
                let count = store
                    .tasks()
                    .iter()
                    .filter(|t| {
                        let local =
                            team.slug == local::WORKSPACE && t["localProject"] == project.slug;
                        (project
                            .repositories
                            .iter()
                            .any(|repository| t["repo"] == *repository)
                            || local)
                            && t["stage"] != "done"
                            && (ui.source == 0
                                || (ui.source == 1 && (t["source"]["kind"] == "issue" || local))
                                || (ui.source == 2 && t["source"]["kind"] == "pr")
                                || (ui.source == 3 && t["source"].is_null() && !local))
                    })
                    .count();
                lines.push(Line::styled(
                    format!(
                        "{} {}  {}",
                        if ui.project.as_ref() == Some(&project.slug) {
                            ">"
                        } else {
                            " "
                        },
                        clean(&project.name),
                        count
                    ),
                    if ui.project.as_ref() == Some(&project.slug) {
                        style(BLUE).add_modifier(Modifier::BOLD)
                    } else {
                        style(MUTED)
                    },
                ));
            }
        }
        lines.extend([
            Line::raw(""),
            Line::from(label("REPOSITORIES  [r]", MUTED)),
            Line::raw(""),
        ]);
        for (i, repo) in repos.iter().enumerate().take(8) {
            lines.push(Line::styled(
                format!(
                    "{} {}",
                    if ui.repo == i + 1 { ">" } else { " " },
                    repo.rsplit('/').next().unwrap_or(repo)
                ),
                if ui.repo == i + 1 {
                    style(BLUE).add_modifier(Modifier::BOLD)
                } else {
                    style(MUTED)
                },
            ));
        }
        lines.extend([
            Line::raw(""),
            Line::from(label(
                if ui.local_scope() {
                    "LOCAL / THIS MACHINE"
                } else {
                    "GOVERNANCE"
                },
                MUTED,
            )),
            Line::from(label(
                if ui.discovering {
                    "Refreshing teams...".into()
                } else if store.catalog.revision.is_empty() {
                    "Not loaded / g refresh".into()
                } else {
                    format!(
                        "revision {}",
                        store.catalog.revision.chars().take(8).collect::<String>()
                    )
                },
                BLUE,
            )),
            Line::from(label("g refresh teams/projects", MUTED)),
            Line::from(label("G open declaration", MUTED)),
        ]);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .border_style(style(EDGE)),
            ),
            split[0],
        );
        split[1]
    } else {
        regions[2]
    };
    if ui.agent_view {
        agents::view(ui, f, store, body);
    } else if ui.detail {
        inspector(ui, f, store, body, true)
    } else if ui.board {
        board(ui, f, store, body)
    } else if body.width >= 95 {
        let panes = Layout::horizontal([Constraint::Percentage(53), Constraint::Percentage(47)])
            .spacing(1)
            .split(body);
        task_list(ui, f, store, panes[0]);
        inspector(ui, f, store, panes[1], false)
    } else {
        task_list(ui, f, store, body)
    }
    let status = if let Some(('/', value)) = &ui.prompt {
        format!("/ {}▏   Enter keep · Esc cancel", clean(value))
    } else if ui.message.is_empty() {
        if let Some(task) = ui.selected(store) {
            format!("{}  ·  {}", reference(task), clean(string(task, "title")))
        } else {
            "w Choose workspace  ·  P Choose project  ·  s Refresh tasks".into()
        }
    } else {
        clean(ui.message.lines().next().unwrap_or_default())
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(label(
                status,
                if ui.error { Color::LightRed } else { MUTED },
            )),
            if ui.agent_view {
                Line::from(label(
                    "Enter Talk / v Result / d Diff / s Stop / Delete Run / a Tasks / ? Help",
                    BLUE,
                ))
            } else {
                Line::from(vec![
                    label("n", BLUE),
                    label(
                        if ui.local_scope() {
                            " task  "
                        } else {
                            " GCD issue  "
                        },
                        TEXT,
                    ),
                    label("m", BLUE),
                    label(" move  ", TEXT),
                    label("c", BLUE),
                    label(" comment  ", TEXT),
                    label("z", BLUE),
                    label(" close/reopen  ", TEXT),
                    label("/", BLUE),
                    label(" search  ", TEXT),
                    label("Enter", BLUE),
                    label(" details  ", TEXT),
                    label("v", BLUE),
                    label(" view  ", TEXT),
                    label("a", BLUE),
                    label(" agents  ", TEXT),
                    label("A", BLUE),
                    label(" launch  ", TEXT),
                    label("?", BLUE),
                    label(" actions  ", TEXT),
                    label("q", BLUE),
                    label(" quit", TEXT),
                ])
            },
        ])
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(style(EDGE)),
        ),
        regions[3],
    );
    if ui.help {
        help::draw(ui, f, store);
    }
    if let Some(task) = &ui.confirm_state {
        let closing = task["source"]["state"] == "OPEN";
        let is_pr = task["source"]["kind"] == "pr";
        let rect = popup(area, 78, 12);
        f.render_widget(Clear, rect);
        let action = if closing { "Close" } else { "Reopen" };
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    clean(string(task, "title")),
                    style(TEXT).add_modifier(Modifier::BOLD),
                ),
                Line::from(label(
                    format!("{} / {}", string(task, "repo"), reference(task)),
                    MUTED,
                )),
                Line::raw(""),
                Line::raw(format!(
                    "{action} this {} on GCD through Collar?",
                    if is_pr { "pull request" } else { "issue" }
                )),
                Line::from(label(
                    if is_pr {
                        "Closing a PR does not merge it. Its branch is retained."
                    } else {
                        "Its comments and history are retained."
                    },
                    MUTED,
                )),
                Line::raw(""),
                Line::from(label(format!("Enter {action} / Esc cancel"), BLUE)),
            ])
            .wrap(Wrap { trim: false })
            .block(
                block(
                    format!("{action} {}", if is_pr { "pull request" } else { "issue" }),
                    true,
                )
                .padding(Padding::new(2, 1, 1, 0)),
            ),
            rect,
        );
    }
    if let Some(stage) = ui.moving {
        let rect = popup(area, 44, 11);
        f.render_widget(Clear, rect);
        let shared = ui
            .selected(store)
            .is_some_and(|task| !task["source"].is_null());
        let mut lines = vec![
            Line::from(label(
                if shared {
                    "Enter applies this stage on GCD."
                } else {
                    "Enter moves this task locally."
                },
                MUTED,
            )),
            Line::raw(""),
        ];
        for (i, name) in NAMES.iter().enumerate() {
            lines.push(Line::styled(
                format!(
                    "{} {}  {}",
                    if i == stage { ">" } else { " " },
                    i + 1,
                    if i == 4 && shared {
                        "Done / close issue on GCD"
                    } else {
                        name
                    }
                ),
                if i == stage {
                    style(BLUE).add_modifier(Modifier::BOLD)
                } else {
                    style(TEXT)
                },
            ))
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(label(
            "j/k select  ·  Enter move  ·  Esc cancel",
            MUTED,
        )));
        f.render_widget(
            Paragraph::new(lines)
                .block(block("Change task stage", true).padding(Padding::horizontal(1))),
            rect,
        )
    }
    if let Some(picker) = &ui.picker {
        let choices = ui.choices(store, picker.kind, &picker.query);
        let rect = popup(area, 82, 24);
        f.render_widget(Clear, rect);
        let regions = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(block("", true).inner(rect));
        let title = match picker.kind {
            PickKind::Workspace => "Governance workspaces",
            PickKind::Project => "Governance projects",
            PickKind::Repository => "Project repositories",
        };
        f.render_widget(block(title, true), rect);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(label(format!("Filter: {}", clean(&picker.query)), BLUE)),
                Line::from(label(
                    "Defined in Governance / shared across machines",
                    MUTED,
                )),
            ]),
            regions[0],
        );
        let width = usize::from(regions[1].width.saturating_sub(3));
        let items = choices
            .iter()
            .enumerate()
            .map(|(i, choice)| {
                let name = clean(&choice.name);
                let pad = width.saturating_sub(Line::raw(name.clone()).width());
                ListItem::new(vec![
                    Line::styled(
                        format!("{name}{}", " ".repeat(pad)),
                        if i == picker.row {
                            style(TEXT).add_modifier(Modifier::REVERSED | Modifier::BOLD)
                        } else {
                            style(TEXT)
                        },
                    ),
                    Line::from(label(clean(&choice.description), MUTED)),
                    Line::raw(""),
                ])
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(picker.row));
        f.render_stateful_widget(
            List::new(items).highlight_symbol("> "),
            regions[1],
            &mut state,
        );
        f.render_widget(
            Paragraph::new("Arrows/j/k select  ·  Enter open  ·  Type to filter  ·  Esc cancel")
                .style(style(MUTED)),
            regions[2],
        );
    }
    if ui.error_popup {
        let rect = popup(area, 96, 22);
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(clean(&ui.message))
                .wrap(Wrap { trim: false })
                .block(
                    block("Error details / any key returns", true)
                        .padding(Padding::new(2, 1, 1, 0)),
                ),
            rect,
        );
    }
    agents::overlays(ui, f);
    if let Some((mode, value)) = &ui.prompt {
        if *mode == '/' {
            return;
        }
        let (mut title, mut hint) = match mode {
            'C' => (
                "Comment through Collar",
                "Enter new line / Ctrl+S posts / Esc cancels",
            ),
            '/' => ("Search tasks", "Enter apply  ·  Esc cancel"),
            'n' if ui.local_scope() => (
                "New local task",
                "Enter saves in this directory project / Esc cancel",
            ),
            'n' => ("Create GCD issue", "Enter creates on GCD  ·  Esc cancel"),
            'N' => ("Unsent local draft", "Enter saves locally  ·  Esc cancel"),
            'c' => (
                "Add local directory",
                "Directory path / Enter adds / Esc cancels",
            ),
            'X' => (
                "Forget local project",
                "Type remove / keeps files, tasks, and runs / Esc cancels",
            ),
            'e' => ("Edit issue title", "Enter saves on GCD  ·  Esc cancel"),
            'p' => (
                "GCD priority",
                "high / normal / low  ·  Enter applies existing label  ·  Esc cancel",
            ),
            'd' => (
                "Edit description",
                "Enter new line  ·  Ctrl+S saves on GCD  ·  Esc cancel",
            ),
            'o' => (
                "GCD assignees",
                "Comma-separated GCD logins  ·  Enter saves on GCD  ·  Esc cancel",
            ),
            _ => ("Input", "Enter apply  ·  Esc cancel"),
        };
        if ui
            .selected(store)
            .is_some_and(|task| task["source"].is_null())
        {
            match mode {
                'e' => {
                    title = "Edit local task title";
                    hint = "Enter saves locally / Esc cancels";
                }
                'd' => {
                    title = "Edit local description";
                    hint = "Enter new line / Ctrl+S saves locally / Esc cancels";
                }
                'o' => {
                    title = "Local task owner";
                    hint = "Enter saves locally / Esc cancels";
                }
                'p' => {
                    title = "Local task priority";
                    hint = "high / normal / low / Enter saves locally";
                }
                _ => {}
            }
        }
        let rect = popup(area, 76, if matches!(*mode, 'd' | 'C') { 18 } else { 8 });
        f.render_widget(Clear, rect);
        let mut text = vec![Line::from(label(hint, MUTED)), Line::raw("")];
        if *mode == 'C'
            && let Some((id, _)) = &ui.edit_target
            && let Some(task) = store.tasks().iter().find(|task| task["id"] == *id)
        {
            text.insert(
                0,
                Line::from(label(
                    format!(
                        "{} / {} / {}",
                        string(task, "repo"),
                        reference(task),
                        string(task, "title")
                    ),
                    BLUE,
                )),
            );
        }
        if *mode == 'c' {
            text.insert(
                0,
                Line::from(label(
                    "Any existing directory. This project stays on this machine.",
                    TEXT,
                )),
            );
        }
        if matches!(*mode, 'n' | 'N') && !ui.local_scope() {
            text.insert(
                0,
                Line::from(label(
                    format!("Repository: {repo}  ·  Tab chooses target"),
                    BLUE,
                )),
            );
        }
        if ui.error {
            text.insert(0, Line::from(label(clean(&ui.message), Color::LightRed)));
        }
        let chars = value.chars().collect::<Vec<_>>();
        let cursor = ui.cursor.min(chars.len());
        let before = chars[..cursor].iter().collect::<String>();
        let after = chars[cursor..].iter().collect::<String>();
        for line in clean(&format!("{before}▏{after}")).split('\n') {
            text.push(Line::raw(line.to_owned()));
        }
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .block(block(title, true).padding(Padding::horizontal(1))),
            rect,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_wrapping_keeps_words_and_marks_truncation() {
        assert_eq!(
            title_rows("Fix the preview deployment", 12),
            vec!["Fix the", "preview..."]
        );
        assert_eq!(title_rows("Short task", 20), vec!["Short task"]);
    }
}
