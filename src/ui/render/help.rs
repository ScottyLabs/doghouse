use super::*;
fn group(lines: &mut Vec<Line<'static>>, title: &str, entries: &[(&str, &str)]) {
    lines.push(Line::styled(
        title.to_owned(),
        style(BLUE).add_modifier(Modifier::BOLD),
    ));
    for (key, action) in entries {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{key:<12}"),
                style(TEXT).add_modifier(Modifier::BOLD),
            ),
            label((*action).to_owned(), MUTED),
        ]));
    }
    lines.push(Line::raw(""));
}
pub(super) fn draw(ui: &Ui, f: &mut ratatui::Frame, store: &Store) {
    let rect = popup(f.area(), 112, 33);
    f.render_widget(Clear, rect);
    let container = block("Keyboard guide", true).padding(Padding::new(2, 2, 1, 0));
    let inner = container.inner(rect);
    f.render_widget(container, rect);
    let regions = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                if ui.agent_view {
                    "Agents"
                } else if ui.local_scope() {
                    "Local project"
                } else {
                    "Project management"
                },
                style(TEXT).add_modifier(Modifier::BOLD),
            ),
            Line::from(label(
                format!("{} / {}", ui.workspace_name(store), ui.project_name(store)),
                MUTED,
            )),
        ]),
        regions[0],
    );
    let (mut left, right) = contents(ui);
    let scroll = ui
        .help_scroll
        .min(scroll_limit(ui, f.area().width, f.area().height));
    if regions[1].width >= 86 {
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(3)
            .split(regions[1]);
        for (area, lines) in [(columns[0], left), (columns[1], right)] {
            f.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((scroll, 0)),
                area,
            );
        }
    } else {
        left.extend(right);
        f.render_widget(
            Paragraph::new(left)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            regions[1],
        );
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::from(label("Up/Down scroll   PgUp/PgDn page   Esc close", BLUE)),
            Line::from(label(
                if ui.agent_view {
                    "Native sessions; changes stay for review."
                } else if ui.local_scope() {
                    "Local edits stay on this machine."
                } else {
                    "Comments and state changes are shared through Collar."
                },
                MUTED,
            )),
        ]),
        regions[2],
    );
}

fn contents(ui: &Ui) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let mut left = vec![];
    let mut right = vec![];
    if ui.agent_view {
        group(
            &mut left,
            "START",
            &[
                ("A / n", "Run an agent for a task"),
                ("Enter", "Start from the launch dialog"),
                ("Tab", "Change launch settings"),
            ],
        );
        group(
            &mut left,
            "MANAGE",
            &[
                ("j / k", "Choose a run"),
                ("Enter", "Activity / result"),
                ("r", "Continue its native session"),
                ("s", "Stop the owned process group"),
                ("f", "Current project / all runs"),
            ],
        );
        group(
            &mut right,
            "REVIEW & SHARE",
            &[
                ("d", "Review changes"),
                ("P", "Review a result comment for GCD"),
                ("x", "Remove a clean worktree"),
                ("Delete / D", "Delete finished run history"),
                ("t", "Return to the original task"),
                ("O", "Open source on GCD"),
            ],
        );
        group(
            &mut right,
            "RETURN",
            &[
                ("a / Esc", "Back to tasks"),
                ("q", "Quit after runs finish"),
                ("Ctrl+C", "Stop owned runs and exit"),
            ],
        );
    } else {
        group(
            &mut left,
            "NAVIGATE",
            &[
                ("j / k", "Choose task"),
                ("h / l", "Previous / next stage"),
                ("1 - 5", "Jump to stage"),
                ("Enter", "Expand task and discussion"),
                ("/", "Search this view"),
                ("Esc", "Back / clear search"),
            ],
        );
        group(
            &mut left,
            "TASKS",
            &[
                (
                    "n",
                    if ui.local_scope() {
                        "New local task"
                    } else {
                        "New GCD issue"
                    },
                ),
                ("e / d", "Edit title / description"),
                ("o / p", "Owner / priority"),
                ("m", "Move task"),
                ("b", "Export task context"),
            ],
        );
        if !ui.local_scope() {
            group(
                &mut left,
                "THROUGH COLLAR",
                &[
                    ("C", "Write a comment"),
                    ("z", "Close / reopen, with confirmation"),
                    ("O", "Open the source on GCD"),
                ],
            );
        }
        group(
            &mut right,
            "WORKSPACE",
            &[
                ("u", "All accessible / My work"),
                ("w", "Choose workspace"),
                ("P", "Choose project"),
                ("r / Tab", "Choose / cycle repositories"),
                ("f", "Issues / PRs / drafts"),
                ("v", "List / kanban"),
                ("s", "Refresh shared tasks"),
                ("g / G", "Refresh / open Governance"),
            ],
        );
        group(
            &mut right,
            "AGENTS",
            &[
                ("A", "Run an agent on selected task"),
                ("a", "Activity and results"),
            ],
        );
        group(
            &mut right,
            "LOCAL",
            &[
                ("c", "Add a directory project"),
                ("N", "Unsent draft"),
                ("X", "Forget a local registration"),
                ("!", "Read the last error"),
                ("q", "Quit"),
            ],
        );
    }
    (left, right)
}

pub(crate) fn scroll_limit(ui: &Ui, width: u16, height: u16) -> u16 {
    let rect = popup(Rect::new(0, 0, width, height), 112, 33);
    let width = rect.width.saturating_sub(6).max(1);
    let available = usize::from(rect.height.saturating_sub(8));
    let (mut left, right) = contents(ui);
    let rows = |lines: &[Line<'_>], width: u16| {
        lines
            .iter()
            .map(|line| line.width().max(1).div_ceil(usize::from(width.max(1))))
            .sum::<usize>()
    };
    let total = if width >= 86 {
        let column = width.saturating_sub(3) / 2;
        rows(&left, column).max(rows(&right, column))
    } else {
        left.extend(right);
        rows(&left, width)
    };
    total
        .saturating_sub(available)
        .try_into()
        .unwrap_or(u16::MAX)
}
