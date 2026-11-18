use super::*;
use crate::agents::{Mode, Run, State};
fn state_color(state: State) -> Color {
    match state {
        State::Running => BLUE,
        State::Completed => GREEN,
        State::Continued => BLUE,
        State::Suspended => AMBER,
        State::Detached => BLUE,
        State::Stopping | State::Attention => AMBER,
        State::Failed => Color::Red,
        State::Stopped | State::Unattached => MUTED,
        State::Interrupted => AMBER,
    }
}
fn selected_run<'a>(ui: &'a Ui, store: &Store) -> Option<&'a Run> {
    ui.visible_runs(store).get(ui.run_row).copied()
}
fn lines_for(run: &Run, output: bool, width: u16) -> Vec<Line<'static>> {
    if output {
        let mut lines = vec![
            Line::styled(
                format!("{} / {}", run.provider.name(), run.state.name()),
                style(state_color(run.state)).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
        ];
        if run.native_continued {
            lines.push(Line::from(label(
                "Last automated result; this conversation continued in the native CLI.",
                AMBER,
            )));
            lines.push(Line::raw(""));
        }
        if run.result.is_empty() {
            lines.push(Line::from(label(
                "No automated result yet. v returns to live activity.",
                MUTED,
            )))
        }
        lines.extend(markdown::render(&run.result, width));
        if run.result_truncated {
            lines.push(Line::from(label(
                "Result preview capped; inspect the saved transcript.",
                AMBER,
            )))
        }
        return lines;
    }
    let mut lines = vec![
        Line::styled(
            clean(&run.task_title),
            style(TEXT).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![
            label(run.provider.name(), BLUE),
            label(format!("  {}", run.mode.name()), MUTED),
        ]),
        Line::from(label(run.state.name(), state_color(run.state))),
        Line::from(label(
            if run.repository.is_empty() {
                "Local directory project".into()
            } else {
                format!("Repository  {}", run.repository)
            },
            MUTED,
        )),
        Line::from(label(
            format!("Checkout    {}", run.directory.display()),
            MUTED,
        )),
    ];
    lines.push(Line::from(label(
        format!(
            "Subagents   {} reported / Right opens their details",
            run.subagents.len()
        ),
        BLUE,
    )));
    if let Some(model) = &run.model {
        lines.push(Line::from(label(
            format!("Model       {}", clean(model)),
            MUTED,
        )))
    }
    if let Some(session) = &run.session_id {
        lines.push(Line::from(label(
            format!("Session     {}", clean(session)),
            MUTED,
        )))
    }
    if let Some(input) = run.usage["input_tokens"].as_u64() {
        lines.push(Line::from(label(
            format!(
                "Tokens      {input} input / {} output",
                run.usage["output_tokens"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "not reported".into())
            ),
            MUTED,
        )))
    }
    if let Some(cost) = run.estimated_cost {
        lines.push(Line::from(label(
            format!("CLI-estimated session cost  ${cost:.4}"),
            MUTED,
        )))
    }
    if run.publication != "not-published" {
        lines.push(Line::from(label(
            format!("GCD result  {}", run.publication),
            if run.publication == "published" {
                GREEN
            } else {
                AMBER
            },
        )))
    }
    if run.state == State::Unattached {
        lines.push(Line::from(label(
            "A process is still using this checkout. Stop its current turn before opening the conversation.",
            AMBER,
        )))
    }
    if run.state == State::Interrupted {
        lines.push(Line::from(label(
            if run.session_id.is_some() {
                "o recovers the saved conversation / r continues with a new goal"
            } else {
                "No native conversation ID was saved. Files are retained; A starts a new session."
            },
            AMBER,
        )));
    }
    if let Some(error) = &run.failure {
        section(&mut lines, "ATTENTION");
        lines.extend(
            clean(error)
                .lines()
                .map(|line| Line::from(label(line.to_owned(), Color::Red))),
        )
    }
    section(&mut lines, "GOAL");
    lines.extend(
        clean(&run.goal)
            .lines()
            .map(|line| Line::raw(line.to_owned())),
    );
    if !run.result.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(label(
            "Enter Open / Space Actions / d Delete / Tab Result",
            BLUE,
        )))
    }
    section(&mut lines, "LIVE ACTIVITY / NEWEST FIRST");
    for event in run.events.iter().rev() {
        lines.push(Line::from(label(event.kind.clone(), BLUE)));
        lines.extend(
            clean(&event.message)
                .lines()
                .map(|line| Line::raw(line.to_owned())),
        );
        lines.push(Line::raw(""));
    }
    if run.log_truncated {
        lines.push(Line::from(label(
            "Saved transcript reached its size limit.",
            AMBER,
        )))
    }
    lines
}
pub(super) fn view(ui: &Ui, f: &mut ratatui::Frame, store: &Store, area: Rect) {
    if let Some(diff) = &ui.agent_diff {
        f.render_widget(
            Paragraph::new(clean(diff))
                .wrap(Wrap { trim: false })
                .scroll((ui.run_scroll, 0))
                .block(block("Agent diff / Esc back", true).padding(Padding::horizontal(1))),
            area,
        );
        return;
    }
    let runs = ui.visible_runs(store);
    if runs.is_empty() {
        f.render_widget(Paragraph::new("No agent sessions yet.\n\nA  Start an agent for the selected task\na  Return to tasks\n\nChoose Codex or Claude Code, review the checkout and goal, then launch.\nPlanning runs inspect the checkout. Implementation runs use an isolated Git worktree.\n\nRuns execute on this machine using the selected CLI's existing login.\nResults are published to GCD only after you review and confirm them.").wrap(Wrap{trim:false}).style(style(MUTED)).block(block("Sessions / this machine",true).padding(Padding::new(2,1,1,0))),area);
        return;
    }
    let panes = if area.width >= 90 {
        Layout::horizontal([Constraint::Percentage(37), Constraint::Percentage(63)])
            .spacing(1)
            .split(area)
    } else {
        Layout::vertical([Constraint::Length(8), Constraint::Min(6)])
            .spacing(1)
            .split(area)
    };
    let width = usize::from(panes[0].width.saturating_sub(5));
    let items = runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            let title = clean(&run.task_title);
            let pad = width.saturating_sub(Line::raw(title.clone()).width());
            ListItem::new(vec![
                Line::styled(
                    format!("{title}{}", " ".repeat(pad)),
                    if index == ui.run_row {
                        style(TEXT).add_modifier(Modifier::REVERSED | Modifier::BOLD)
                    } else {
                        style(TEXT)
                    },
                ),
                Line::from(vec![
                    label(run.provider.name(), BLUE),
                    label(format!(" / {}", run.state.name()), state_color(run.state)),
                ]),
                Line::from(label(
                    format!(
                        "{} / {}",
                        if run.repository.is_empty() {
                            "local directory"
                        } else {
                            run.repository.rsplit('/').next().unwrap_or(&run.repository)
                        },
                        if run.mode == Mode::Plan {
                            "Plan"
                        } else {
                            "Implement"
                        }
                    ),
                    MUTED,
                )),
                Line::raw(""),
            ])
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(ui.run_row));
    f.render_stateful_widget(
        List::new(items).highlight_symbol("> ").block(
            block(
                format!(
                    "{} {}",
                    runs.len(),
                    if ui.run_history { "turns" } else { "sessions" }
                ),
                true,
            )
            .padding(Padding::horizontal(1)),
        ),
        panes[0],
        &mut state,
    );
    if let Some(run) = selected_run(ui, store) {
        f.render_widget(
            Paragraph::new(lines_for(
                run,
                ui.run_output,
                panes[1].width.saturating_sub(4),
            ))
            .wrap(Wrap { trim: false })
            .scroll((ui.run_scroll, 0))
            .block(
                block(
                    if ui.run_output {
                        "Result / Tab activity / o conversation"
                    } else {
                        "Session / Enter open / Space actions / Right subagents"
                    },
                    false,
                )
                .padding(Padding::horizontal(1)),
            ),
            panes[1],
        );
    }
}
pub(super) fn overlays(ui: &Ui, f: &mut ratatui::Frame, store: &Store) {
    if let Some(panel) = &ui.subagents
        && let Some(run) = ui.runs.iter().find(|run| run.id == panel.parent)
    {
        let rect = popup(f.area(), 100, 28);
        f.render_widget(Clear, rect);
        let inner = block("", true).inner(rect);
        let regions = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(2),
            Constraint::Length(3),
        ])
        .split(inner);
        f.render_widget(
            block(format!("Subagents / {}", clean(&run.task_title)), true),
            rect,
        );
        let hint = format!(
            "o opens the parent conversation; {} manages individual children.\n{}",
            run.provider.subagent_menu(),
            if run.native_continued {
                "Showing the last automated reports. Live activity is in the native conversation."
            } else {
                "Status and results below are the last reports received from the agent."
            }
        );
        f.render_widget(
            Paragraph::new(hint)
                .wrap(Wrap { trim: false })
                .style(style(MUTED)),
            regions[0],
        );
        if panel.details {
            if let Some(agent) = crate::agents::subagents::ordered(&run.subagents).get(panel.row) {
                let mut lines = vec![
                    Line::styled(
                        if agent.name.is_empty() {
                            agent.id.clone()
                        } else {
                            agent.name.clone()
                        },
                        style(TEXT).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(format!("{} / {}", agent.status.name(), agent.id)),
                    Line::raw(""),
                    Line::styled("Task", style(BLUE)),
                ];
                lines.extend(markdown::render(&agent.prompt, regions[1].width));
                lines.push(Line::styled("Last reported result", style(BLUE)));
                lines.extend(markdown::render(&agent.result, regions[1].width));
                f.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .scroll((ui.run_scroll, 0)),
                    regions[1],
                );
            }
        } else if run.subagents.is_empty() {
            f.render_widget(Paragraph::new(format!("No structured subagent reports have been received.\n\nOpen the parent conversation with o and use {} for its live subagents. This list does not mean there are no children.", run.provider.subagent_menu())).wrap(Wrap { trim:false }), regions[1]);
        } else {
            let items = crate::agents::subagents::ordered(&run.subagents)
                .into_iter()
                .map(|agent| {
                    let indent =
                        "  ".repeat(crate::agents::subagents::depth(&run.subagents, agent));
                    ListItem::new(vec![
                        Line::raw(format!(
                            "{indent}{}",
                            if agent.name.is_empty() {
                                &agent.id
                            } else {
                                &agent.name
                            }
                        )),
                        Line::styled(
                            format!("{indent}{}", agent.status.name()),
                            style(match agent.status {
                                crate::agents::subagents::Status::Working
                                | crate::agents::subagents::Status::Starting => BLUE,
                                crate::agents::subagents::Status::Finished => GREEN,
                                crate::agents::subagents::Status::Failed => Color::Red,
                                crate::agents::subagents::Status::Interrupted
                                | crate::agents::subagents::Status::Attention => AMBER,
                                _ => MUTED,
                            }),
                        ),
                        Line::raw(""),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = ListState::default().with_selected(Some(panel.row));
            f.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("> ")
                    .highlight_style(style(TEXT).add_modifier(Modifier::REVERSED)),
                regions[1],
                &mut state,
            );
        }
        f.render_widget(
            Paragraph::new(vec![
                Line::raw(if panel.details {
                    "Page Up/Down scroll / Left or Esc back"
                } else {
                    "Up/Down choose / Enter inspect / Left or Esc back"
                }),
                Line::raw("o Manage in conversation / F5 Refresh"),
                Line::raw(if run.subagents_omitted {
                    "Report list capped; all children remain available in the native conversation."
                } else {
                    "Stopping a session affects the parent and its child tools."
                }),
            ])
            .style(style(MUTED)),
            regions[2],
        );
    }
    if let Some(menu) = &ui.agent_actions {
        let row = menu.row;
        let rect = popup(f.area(), 76, 16);
        f.render_widget(Clear, rect);
        let container = block("Session actions", true).padding(Padding::horizontal(1));
        let inner = container.inner(rect);
        f.render_widget(container, rect);
        let regions = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(inner);
        f.render_widget(
            Paragraph::new("Up/Down choose / Enter apply / Esc back").wrap(Wrap { trim: false }),
            regions[0],
        );
        let items = crate::ui::agents::session_actions(ui, store)
            .into_iter()
            .map(|(name, _)| ListItem::new(name))
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(Some(row));
        f.render_stateful_widget(
            List::new(items)
                .highlight_symbol("> ")
                .highlight_style(style(BLUE).add_modifier(Modifier::BOLD)),
            regions[1],
            &mut state,
        );
    }

    for (id, title, description) in [
        (
            &ui.stop_run,
            "Stop session and all subagents?",
            "This stops the entire session, including its current work and child tools. Conversation history and checkout changes are retained. Quitting Shepherd instead keeps the session running.",
        ),
        (
            &ui.recover_run,
            "Recover interrupted conversation?",
            "Its worker is no longer running. Open the saved native conversation in the existing checkout. No automated turn is restarted by Shepherd; unsaved terminal input may be unavailable.",
        ),
    ] {
        if let Some(id) = id {
            let rect = popup(f.area(), 76, 12);
            f.render_widget(Clear, rect);
            let title_text = ui
                .runs
                .iter()
                .find(|run| run.id == *id)
                .map(|run| run.task_title.as_str())
                .unwrap_or("Session");
            f.render_widget(
                Paragraph::new(format!(
                    "{}\n\n{description}\n\nEnter confirms / Esc cancels",
                    clean(title_text)
                ))
                .wrap(Wrap { trim: false })
                .block(block(title, true).padding(Padding::horizontal(1))),
                rect,
            );
        }
    }

    let area = f.area();
    if let Some(form) = &ui.launch {
        let rect = popup(area, 94, 25);
        f.render_widget(Clear, rect);
        let container = block(
            if form.resume.is_some() {
                "Continue agent"
            } else {
                "Run agent on task"
            },
            true,
        )
        .padding(Padding::new(2, 2, 1, 0));
        let inner = container.inner(rect);
        f.render_widget(container, rect);
        let regions = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(2),
            Constraint::Length(3),
        ])
        .split(inner);
        let title = if let Some(id) = &form.resume {
            ui.runs
                .iter()
                .find(|run| run.id == *id)
                .map(|run| run.task_title.clone())
                .unwrap_or_default()
        } else {
            string(&form.task, "title").into()
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(clean(&title), style(TEXT).add_modifier(Modifier::BOLD)),
                Line::from(label(
                    if form.resume.is_some() {
                        "Follow up in the same native session"
                    } else {
                        if std::path::Path::new(&form.checkout).is_dir() {
                            "Describe the outcome. The project and agent are already selected."
                        } else {
                            "Choose where this project's files live, then describe the outcome."
                        }
                    },
                    MUTED,
                )),
            ])
            .wrap(Wrap { trim: false }),
            regions[0],
        );
        let goal = if form.goal.is_empty() {
            if form.resume.is_some() {
                "What should the agent do next?".into()
            } else {
                "Optional goal; Enter works on the selected task".into()
            }
        } else if form.field == 3 {
            with_cursor(&form.goal, form.cursor)
        } else {
            clean(&form.goal)
        };
        f.render_widget(
            Paragraph::new(goal)
                .wrap(Wrap { trim: false })
                .style(style(if form.goal.is_empty() { MUTED } else { TEXT }))
                .block(block("Goal", form.field == 3).padding(Padding::horizontal(1))),
            regions[1],
        );
        let mode = if form.mode == Mode::Plan {
            "Plan only / no edits"
        } else {
            "Implement task / separate checkout"
        };
        let row = |field: usize, name: &str, value: String| {
            Line::from(vec![
                label(
                    format!(
                        "{} {name:<10}",
                        if form.settings && form.field == field {
                            ">"
                        } else {
                            " "
                        }
                    ),
                    MUTED,
                ),
                Span::styled(
                    value,
                    style(if form.settings && form.field == field {
                        BLUE
                    } else {
                        TEXT
                    })
                    .add_modifier(if form.settings && form.field == field {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
            ])
        };
        f.render_widget(
            Paragraph::new(vec![
                row(
                    0,
                    "Agent",
                    format!(
                        "{}{}",
                        form.provider.name(),
                        if form.provider.available() {
                            ""
                        } else {
                            " / not installed"
                        }
                    ),
                ),
                row(1, "Task mode", mode.into()),
                row(
                    2,
                    "Directory",
                    if form.field == 2 && form.settings {
                        with_cursor(&form.checkout, form.cursor)
                    } else {
                        clean(&form.checkout)
                    },
                ),
                Line::from(label(
                    if form.settings {
                        "Left/Right changes options / Tab next / Enter returns to goal"
                    } else {
                        "Tab changes settings; the defaults are ready to use."
                    },
                    BLUE,
                )),
            ])
            .wrap(Wrap { trim: false }),
            regions[2],
        );
        f.render_widget(Paragraph::new(if form.mode==Mode::Implement{"SETUP\nRead AGENTS.md and project docs, outline the approach, implement, run checks, and leave changes for review."}else{"SETUP\nRead AGENTS.md and project docs, inspect the task, and outline an approach. No repository edits."}).wrap(Wrap{trim:false}).style(style(MUTED)),regions[3]);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(label(
                    if ui.error {
                        clean(ui.message.lines().next().unwrap_or_default())
                    } else {
                        "Enter start / Alt+Enter newline / Tab settings / Esc cancel".into()
                    },
                    if ui.error { Color::Red } else { BLUE },
                )),
                Line::from(label(
                    "Uses the CLI's existing login. Results stay local until you publish them.",
                    MUTED,
                )),
            ])
            .wrap(Wrap { trim: false }),
            regions[4],
        );
    }
    if let Some(id) = &ui.take_over
        && let Some(run) = ui.runs.iter().find(|run| run.id == *id)
    {
        let rect = popup(area, 80, 12);
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    clean(&run.task_title),
                    style(TEXT).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::raw(if run.state == State::Unattached {
                    "Stop the old process and open its saved conversation in a persistent worker?"
                } else {
                    "Stop this automated turn and its child tools, then open the conversation?"
                }),
                Line::from(label(
                    "Saved history and files are retained. Unsent terminal input may be lost.",
                    MUTED,
                )),
                Line::raw(""),
                Line::from(label("Enter take over / Esc cancel", BLUE)),
            ])
            .wrap(Wrap { trim: false })
            .block(block("Open active conversation", true).padding(Padding::new(2, 1, 1, 0))),
            rect,
        );
    }
    if let Some(id) = &ui.delete_run
        && let Some(run) = ui.runs.iter().find(|run| run.id == *id)
    {
        let stopping = ui.runs.iter().any(|candidate| {
            (candidate.id == run.id
                || (!ui.run_history && candidate.conversation_key() == run.conversation_key()))
                && (candidate.state.active() || candidate.state == crate::agents::State::Unattached)
        });
        let rect = popup(area, 82, 16);
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    clean(&run.task_title),
                    style(TEXT).add_modifier(Modifier::BOLD),
                ),
                Line::from(label(
                    format!("{} / {}", run.provider.name(), run.state.name()),
                    MUTED,
                )),
                Line::raw(""),
                Line::raw(if stopping {
                    "Stop this session and its children, then delete its history?"
                } else if ui.run_history { "Delete this turn's history and transcript?" } else { "Delete this session and its previous turns?" }),
                Line::raw(
                    "Project files, worktrees, saved native conversations, and GCD comments are kept.",
                ),
                Line::raw(""),
                Line::from(label(
                    format!("Checkout kept at {}", run.directory.display()),
                    MUTED,
                )),
                Line::raw(""),
                Line::from(label("Enter confirm / Esc cancel", BLUE)),
            ])
            .wrap(Wrap { trim: false })
            .block(block("Delete agent run", true).padding(Padding::new(2, 1, 1, 0))),
            rect,
        );
    }
    if let Some(id) = &ui.publish_run
        && let Some(run) = ui.runs.iter().find(|r| r.id == *id)
    {
        let rect = popup(area, 96, 28);
        f.render_widget(Clear, rect);
        let body = format!(
            "POST TO {} #{}\n\n{} / {}\n\n{}",
            run.repository,
            run.task_number.map(|n| n.to_string()).unwrap_or_default(),
            run.provider.name(),
            run.mode.name(),
            run.result
        );
        let regions = Layout::vertical([Constraint::Min(5), Constraint::Length(2)])
            .split(block("", true).inner(rect));
        f.render_widget(block("Publish reviewed result to GCD", true), rect);
        f.render_widget(
            Paragraph::new(clean(&body))
                .wrap(Wrap { trim: false })
                .scroll((ui.run_scroll, 0)),
            regions[0],
        );
        f.render_widget(
            Paragraph::new("Enter publishes this comment / Page Up-Down review / Esc cancel")
                .style(style(BLUE)),
            regions[1],
        );
    }
}
fn with_cursor(value: &str, cursor: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    clean(&format!(
        "{}▏{}",
        chars[..cursor].iter().collect::<String>(),
        chars[cursor..].iter().collect::<String>()
    ))
}
