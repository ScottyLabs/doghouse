use super::*;

pub(super) fn command(run: &Run, executable: &std::ffi::OsStr) -> Result<Command> {
    let mut command = Command::new(executable);
    command.current_dir(&run.directory);
    match run.provider {
        Provider::Codex => {
            command.args(["exec", "--json", "--color", "never"]);
            if run.repository.is_empty() {
                command.arg("--skip-git-repo-check");
            }
            if run.mode == Mode::Plan {
                command.args(["--sandbox", "read-only", "-c", "approval_policy=\"never\""]);
            } else {
                command.arg("--approve-for-me");
            }
            if let Some(session) = &run.session_id {
                validate_session(session)?;
                command.args(["resume", session, "-"]);
            } else {
                command.arg("-");
            }
        }
        Provider::Claude => {
            command.args([
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--permission-mode",
                if run.mode == Mode::Plan {
                    "plan"
                } else {
                    "acceptEdits"
                },
                "--permission-prompts",
                "none",
            ]);
            if run.mode == Mode::Plan {
                command.args(["--tools", "Read,Glob,Grep"]);
            }
            if let Some(session) = &run.session_id {
                validate_session(session)?;
                command.arg(format!("--resume={session}"));
            }
        }
    }
    Ok(command)
}
pub(super) fn interactive_command(run: &Run, executable: &std::ffi::OsStr) -> Result<Command> {
    let session = run
        .session_id
        .as_deref()
        .context("No native session was reported. Press A to start a new agent")?;
    validate_session(session)?;
    let mut command = Command::new(executable);
    command.current_dir(&run.directory);
    match run.provider {
        Provider::Codex => {
            command
                .args(["resume", session, "--cd"])
                .arg(&run.directory)
                .args([
                    "--sandbox",
                    if run.mode == Mode::Plan {
                        "read-only"
                    } else {
                        "workspace-write"
                    },
                ]);
        }
        Provider::Claude => {
            command.arg(format!("--resume={session}")).args([
                "--permission-mode",
                if run.mode == Mode::Plan {
                    "plan"
                } else {
                    "acceptEdits"
                },
            ]);
        }
    }
    Ok(command)
}
pub(super) fn validate_session(session: &str) -> Result<()> {
    ensure!(
        session
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && session.len() <= 160
            && session
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
        "Invalid native session ID"
    );
    Ok(())
}
fn bounded(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}
pub(super) fn consume(run: &mut Run, line: &str, stderr: bool) {
    if stderr {
        let text = clean(line);
        if !text.trim().is_empty() {
            run.event("CLI", &bounded(&text, 500));
        }
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        if !line.trim().is_empty() {
            run.event("Output", &bounded(&clean(line), 500));
        }
        return;
    };
    if subagents::observe(run, &value) {
        return;
    }
    match run.provider {
        Provider::Codex => match string(&value, "type") {
            "thread.started" => {
                if let Some(id) = value["thread_id"]
                    .as_str()
                    .filter(|s| validate_session(s).is_ok())
                {
                    run.session_id = Some(id.into());
                }
                run.event("Session", "Codex session started");
            }
            "turn.started" => run.event("Working", "Codex is working"),
            "item.started" | "item.completed" => {
                let item = &value["item"];
                match string(item, "type") {
                    "agent_message" => {
                        let text = string(item, "text");
                        if !text.is_empty() {
                            run.result = bounded(text, 20000);
                            run.result_truncated = text.chars().count() > 20000;
                            run.event("Message", &bounded(text, 600));
                        }
                    }
                    "command_execution" => {
                        run.event("Command", &bounded(string(item, "command"), 500))
                    }
                    "file_change" => run.event("Files", "Updated files in the isolated checkout"),
                    "mcp_tool_call" => run.event("Tool", &bounded(string(item, "tool"), 300)),
                    "error" => {
                        run.failure = Some(bounded(string(item, "message"), 2000));
                        run.event("Error", &bounded(string(item, "message"), 500));
                    }
                    _ => {}
                }
            }
            "turn.completed" => {
                run.completion_seen = true;
                run.usage = value["usage"].clone();
                run.event("Finished", "Codex turn completed");
            }
            "turn.failed" | "error" => {
                let message = value["error"]["message"]
                    .as_str()
                    .or_else(|| value["message"].as_str())
                    .unwrap_or("Codex reported a failed turn");
                run.failure = Some(bounded(message, 2000));
                run.event("Error", &bounded(message, 600));
            }
            _ => {}
        },
        Provider::Claude => {
            if let Some(id) = value["session_id"]
                .as_str()
                .filter(|s| validate_session(s).is_ok())
            {
                run.session_id = Some(id.into());
            }
            match string(&value, "type") {
                "system" if value["subtype"] == "permission_denied" => {
                    run.needs_attention = true;
                    run.event("Permission", "A tool needs attention in Claude Code");
                }
                "system" => {
                    if value["subtype"] == "init" {
                        run.model = value["model"].as_str().map(str::to_owned);
                        run.event("Session", "Claude Code session started");
                    }
                }
                "assistant" => {
                    if let Some(blocks) = value["message"]["content"].as_array() {
                        for block in blocks {
                            match string(block, "type") {
                                "text" => {
                                    let text = string(block, "text");
                                    run.event("Message", &bounded(text, 600));
                                }
                                "tool_use" => {
                                    run.event("Tool", &bounded(string(block, "name"), 300))
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "result" => {
                    run.completion_seen = true;
                    run.result = bounded(string(&value, "result"), 20000);
                    run.result_truncated = string(&value, "result").chars().count() > 20000;
                    run.usage = value["usage"].clone();
                    run.estimated_cost = value["total_cost_usd"].as_f64();
                    if value["is_error"] == true {
                        run.failure = Some(run.result.clone());
                    }
                    if value["permission_denials"]
                        .as_array()
                        .is_some_and(|v| !v.is_empty())
                    {
                        run.needs_attention = true;
                    }
                    run.event("Finished", "Claude Code turn finished");
                }
                _ => {}
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_codex_completion_and_failure() {
        let mut run = Run::fixture(Provider::Codex);
        consume(
            &mut run,
            r#"{"type":"thread.started","thread_id":"session-1"}"#,
            false,
        );
        consume(
            &mut run,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"Reviewed the code"}}"#,
            false,
        );
        consume(
            &mut run,
            r#"{"type":"turn.completed","usage":{"input_tokens":5,"output_tokens":3}}"#,
            false,
        );
        assert_eq!(run.session_id.as_deref(), Some("session-1"));
        assert_eq!(run.result, "Reviewed the code");
        assert!(run.completion_seen);
        consume(
            &mut run,
            r#"{"type":"turn.failed","error":{"message":"No authentication"}}"#,
            false,
        );
        assert!(run.failure.is_some());
    }
    #[test]
    fn claude_reports_permission_denial_even_on_success_exit() {
        let mut run = Run::fixture(Provider::Claude);
        consume(
            &mut run,
            r#"{"type":"result","session_id":"claude-1","is_error":false,"result":"Need permission","permission_denials":[{"tool_name":"Bash"}],"total_cost_usd":0.02}"#,
            false,
        );
        assert!(run.needs_attention);
        assert!(run.completion_seen);
        assert_eq!(run.estimated_cost, Some(0.02));
    }
}
