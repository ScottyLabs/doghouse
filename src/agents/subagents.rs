use super::*;

const LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Starting,
    Working,
    Finished,
    Failed,
    Stopped,
    Interrupted,
    Attention,
    #[default]
    Unknown,
}
impl Status {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::Working => "Working",
            Self::Finished => "Finished",
            Self::Failed => "Failed",
            Self::Stopped => "Stopped",
            Self::Interrupted => "Interrupted",
            Self::Attention => "Needs attention",
            Self::Unknown => "Status not reported",
        }
    }
    fn parse(value: &str) -> Self {
        match value {
            "pending_init" | "pendingInit" | "starting" => Self::Starting,
            "running" | "in_progress" | "inProgress" => Self::Working,
            "completed" => Self::Finished,
            "failed" | "errored" | "not_found" | "notFound" => Self::Failed,
            "shutdown" | "stopped" => Self::Stopped,
            "interrupted" => Self::Interrupted,
            "needs_attention" | "needsAttention" => Self::Attention,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Subagent {
    pub id: String,
    pub parent_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub status: Status,
    pub result: String,
    pub updated_at: String,
}

fn text(value: &str, maximum: usize) -> String {
    clean(value)
        .char_indices()
        .take_while(|(index, character)| index + character.len_utf8() <= maximum)
        .map(|(_, character)| character)
        .collect()
}
fn identifier(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| backend::validate_session(id).is_ok())
}
fn entry<'a>(run: &'a mut Run, id: &str, tool: Option<&str>) -> Option<&'a mut Subagent> {
    if backend::validate_session(id).is_err() || run.session_id.as_deref() == Some(id) {
        return None;
    }
    let canonical = run.subagents.iter().position(|agent| agent.id == id);
    let alias = tool.and_then(|tool| {
        run.subagents
            .iter()
            .position(|agent| agent.tool_call_id.as_deref() == Some(tool))
    });
    if let (Some(canonical), Some(alias)) = (canonical, alias)
        && canonical != alias
    {
        let pending = run.subagents.remove(alias);
        let canonical = if alias < canonical {
            canonical - 1
        } else {
            canonical
        };
        let agent = &mut run.subagents[canonical];
        if agent.prompt.is_empty() {
            agent.prompt = pending.prompt;
        }
        if agent.name.is_empty() {
            agent.name = pending.name;
        }
        if agent.parent_id.is_none() {
            agent.parent_id = pending.parent_id;
        }
    }
    let found = run.subagents.iter().position(|agent| {
        agent.id == id || tool.is_some_and(|tool| agent.tool_call_id.as_deref() == Some(tool))
    });
    let index = if let Some(index) = found {
        index
    } else {
        if run.subagents.len() >= LIMIT {
            run.subagents_omitted = true;
            return None;
        }
        run.subagents.push(Subagent {
            id: id.into(),
            parent_id: None,
            tool_call_id: tool.map(str::to_owned),
            name: String::new(),
            prompt: String::new(),
            status: Status::Unknown,
            result: String::new(),
            updated_at: now(),
        });
        run.subagents.len() - 1
    };
    let agent = &mut run.subagents[index];
    if tool != Some(id) || agent.tool_call_id.as_deref() != Some(id) || agent.id == id {
        agent.id = id.into();
    }
    if let Some(tool) = tool {
        agent.tool_call_id = Some(tool.into());
    }
    agent.updated_at = now();
    Some(agent)
}

fn invocations(run: &mut Run, value: &Value) {
    if value["type"] != "assistant" {
        return;
    }
    let parent =
        identifier(&value["agent_id"]).or_else(|| identifier(&value["parent_tool_use_id"]));
    let parent = parent.map(|id| {
        run.subagents
            .iter()
            .find(|agent| agent.id == id || agent.tool_call_id.as_deref() == Some(id))
            .map(|agent| agent.id.clone())
            .unwrap_or_else(|| id.into())
    });
    if let Some(blocks) = value["message"]["content"].as_array() {
        for block in blocks {
            if block["type"] != "tool_use" || !matches!(string(block, "name"), "Agent" | "Task") {
                continue;
            }
            let Some(tool) = identifier(&block["id"]) else {
                continue;
            };
            let resume = identifier(&block["input"]["resume"]);
            let same_call = run
                .subagents
                .iter()
                .any(|agent| agent.tool_call_id.as_deref() == Some(tool));
            if let Some(agent) = entry(run, resume.unwrap_or(tool), Some(tool)) {
                agent.name = text(
                    string(&block["input"], "description")
                        .lines()
                        .next()
                        .unwrap_or_default(),
                    70,
                );
                if agent.prompt.is_empty() || resume.is_some() {
                    agent.prompt = text(string(&block["input"], "prompt"), 1000);
                }
                if let Some(parent) = &parent {
                    agent.parent_id = Some(parent.clone());
                }
                if matches!(agent.status, Status::Unknown | Status::Starting)
                    || (resume.is_some() && !same_call)
                {
                    agent.status = Status::Starting;
                }
            }
        }
    }
}

/// Consume structured native events only. Never infer children from model prose.
/// True means the message belongs to a child and must not change parent results.
pub(super) fn observe(run: &mut Run, value: &Value) -> bool {
    match run.provider {
        Provider::Codex => {
            let item = &value["item"];
            if !matches!(
                string(item, "type"),
                "collab_tool_call" | "collab_agent_tool_call" | "collabAgentToolCall"
            ) {
                return false;
            }
            let recipients = item["receiver_thread_ids"]
                .as_array()
                .or_else(|| item["receiverThreadIds"].as_array());
            let states = item
                .get("agents_states")
                .or_else(|| item.get("agentsStates"));
            let ids = recipients
                .into_iter()
                .flatten()
                .filter_map(identifier)
                .map(str::to_owned)
                .chain(
                    states
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|states| states.keys().cloned()),
                )
                .collect::<std::collections::BTreeSet<_>>();
            for id in ids {
                let Some(agent) = entry(run, &id, None) else {
                    continue;
                };
                if let Some(parent) = identifier(&item["sender_thread_id"])
                    .or_else(|| identifier(&item["senderThreadId"]))
                    && matches!(string(item, "tool"), "spawn_agent" | "spawnAgent")
                {
                    agent.parent_id = Some(parent.into());
                }
                let state = states.and_then(|states| states.get(&id));
                if let Some(state) = state {
                    agent.status = Status::parse(
                        state["status"]
                            .as_str()
                            .or_else(|| state.as_str())
                            .unwrap_or_default(),
                    );
                    if let Some(message) = state["message"].as_str() {
                        agent.result = text(message, 2000);
                    }
                } else if matches!(string(item, "tool"), "spawn_agent" | "spawnAgent")
                    && agent.status == Status::Unknown
                {
                    agent.status = Status::Starting;
                }
                if agent.prompt.is_empty() {
                    agent.prompt = text(string(item, "prompt"), 1000);
                }
                if agent.name.is_empty() {
                    agent.name = agent
                        .prompt
                        .lines()
                        .next()
                        .map(|line| text(line, 70))
                        .unwrap_or_default();
                }
            }
            false
        }
        Provider::Claude => {
            let kind = string(value, "type");
            let subtype = string(value, "subtype");
            invocations(run, value);
            if kind == "system"
                && matches!(
                    subtype,
                    "task_started" | "task_progress" | "task_notification"
                )
            {
                let Some(id) = identifier(&value["task_id"]) else {
                    return true;
                };
                let tool = identifier(&value["tool_use_id"]);
                let known = run.subagents.iter().any(|agent| {
                    agent.id == id
                        || tool.is_some_and(|tool| agent.tool_call_id.as_deref() == Some(tool))
                });
                let task_type = string(value, "task_type");
                if !known
                    && !matches!(task_type, "local_agent" | "remote_agent" | "agent")
                    && value["subagent_type"].as_str().is_none()
                {
                    return true;
                }
                if let Some(agent) = entry(run, id, tool) {
                    if let Some(parent) = identifier(&value["parent_task_id"]) {
                        agent.parent_id = Some(parent.into());
                    }
                    if let Some(description) = value["description"].as_str() {
                        agent.name = text(description.lines().next().unwrap_or_default(), 70);
                        if agent.prompt.is_empty() {
                            agent.prompt = text(description, 1000);
                        }
                    }
                    agent.status = if subtype == "task_notification" {
                        Status::parse(string(value, "status"))
                    } else {
                        Status::Working
                    };
                    if let Some(summary) = value["summary"].as_str() {
                        agent.result = text(summary, 2000);
                    }
                }
                return true;
            }
            if let Some(tool) =
                identifier(&value["parent_tool_use_id"]).or_else(|| identifier(&value["agent_id"]))
            {
                if let Some(agent) = entry(
                    run,
                    identifier(&value["agent_id"]).unwrap_or(tool),
                    Some(tool),
                ) {
                    if let Some(blocks) = value["message"]["content"].as_array() {
                        for block in blocks {
                            if block["type"] == "text" {
                                agent.result = text(string(block, "text"), 2000);
                            }
                        }
                    }
                    if kind == "result" {
                        agent.result = text(string(value, "result"), 2000);
                        agent.status = if value["is_error"] == true {
                            Status::Failed
                        } else {
                            Status::Finished
                        };
                    }
                    if kind == "system" && subtype == "permission_denied" {
                        agent.status = Status::Attention;
                        agent.result =
                            "A child tool needs attention in the native conversation.".into();
                    }
                }
                return true;
            }
            false
        }
    }
}

pub(crate) fn depth(agents: &[Subagent], agent: &Subagent) -> usize {
    let mut depth = 0;
    let mut parent = agent.parent_id.as_deref();
    let mut visited = std::collections::BTreeSet::from([agent.id.as_str()]);
    while let Some(ancestor) = parent.and_then(|id| agents.iter().find(|agent| agent.id == id)) {
        if depth >= 8 || !visited.insert(&ancestor.id) {
            break;
        }
        depth += 1;
        parent = ancestor.parent_id.as_deref();
    }
    depth
}

pub(crate) fn ordered(agents: &[Subagent]) -> Vec<&Subagent> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let roots = agents.iter().filter(|agent| {
        !agent
            .parent_id
            .as_ref()
            .is_some_and(|parent| agents.iter().any(|candidate| candidate.id == *parent))
    });
    for root in roots.chain(agents.iter()) {
        let mut pending = vec![root];
        while let Some(agent) = pending.pop() {
            if !seen.insert(&agent.id) {
                continue;
            }
            result.push(agent);
            pending.extend(
                agents
                    .iter()
                    .rev()
                    .filter(|child| child.parent_id.as_deref() == Some(agent.id.as_str())),
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn consume(run: &mut Run, value: Value) {
        backend::consume(run, &value.to_string(), false);
    }

    #[test]
    fn codex_children_are_updated_without_changing_the_parent_result() {
        let mut run = Run::fixture(Provider::Codex);
        run.session_id = Some("parent".into());
        run.result = "Parent result".into();
        for status in ["running", "completed"] {
            consume(
                &mut run,
                json!({"type":"item.completed","item":{"type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"parent","receiver_thread_ids":["child"],"prompt":"Review tests","agents_states":{"child":{"status":status,"message":"Child result"}}}}),
            );
        }
        consume(
            &mut run,
            json!({"type":"item.completed","item":{"type":"collabAgentToolCall","tool":"spawnAgent","senderThreadId":"child","receiverThreadIds":["grandchild"],"prompt":"Check fixtures","agentsStates":{"grandchild":{"status":"pendingInit"}}}}),
        );
        assert_eq!(run.subagents.len(), 2);
        assert_eq!(run.subagents[0].status, Status::Finished);
        assert_eq!(run.subagents[0].result, "Child result");
        assert_eq!(depth(&run.subagents, &run.subagents[1]), 1);
        let reversed = vec![run.subagents[1].clone(), run.subagents[0].clone()];
        assert_eq!(
            ordered(&reversed)
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            vec!["child", "grandchild"]
        );
        assert_eq!(run.result, "Parent result");
        assert_eq!(run.session_id.as_deref(), Some("parent"));
        assert!(!run.completion_seen);
    }

    #[test]
    fn claude_task_identity_and_results_do_not_overwrite_parent_session() {
        let mut run = Run::fixture(Provider::Claude);
        run.session_id = Some("parent".into());
        run.result = "Parent result".into();
        let invocation = json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"tool-1","input":{"description":"Review","prompt":"Review tests"}}]}});
        consume(&mut run, invocation.clone());
        consume(
            &mut run,
            json!({"type":"system","subtype":"task_started","task_id":"task-1","tool_use_id":"tool-1","description":"Review tests"}),
        );
        consume(
            &mut run,
            json!({"type":"assistant","agent_id":"task-1","session_id":"child-session","parent_tool_use_id":"tool-1","message":{"content":[{"type":"text","text":"Found a bug"}]}}),
        );
        consume(
            &mut run,
            json!({"type":"system","subtype":"task_notification","task_id":"task-1","tool_use_id":"tool-1","status":"completed","summary":"Reviewed successfully"}),
        );
        consume(&mut run, invocation);
        assert_eq!(run.subagents.len(), 1);
        assert_eq!(run.subagents[0].id, "task-1");
        assert_eq!(run.subagents[0].status, Status::Finished);
        assert_eq!(run.subagents[0].result, "Reviewed successfully");
        assert_eq!(run.result, "Parent result");
        assert_eq!(run.session_id.as_deref(), Some("parent"));
        assert!(!run.completion_seen);
    }

    #[test]
    fn nested_claude_children_and_permission_failures_keep_their_parent() {
        let mut run = Run::fixture(Provider::Claude);
        run.session_id = Some("root".into());
        consume(
            &mut run,
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"tool-parent","input":{"description":"Parent","prompt":"Full parent task"}}]}}),
        );
        consume(
            &mut run,
            json!({"type":"system","subtype":"task_started","task_id":"parent-task","tool_use_id":"tool-parent","description":"Short summary"}),
        );
        consume(
            &mut run,
            json!({"type":"assistant","parent_tool_use_id":"tool-parent","message":{"content":[{"type":"tool_use","name":"Agent","id":"tool-child","input":{"prompt":"Nested task"}}]}}),
        );
        consume(
            &mut run,
            json!({"type":"system","subtype":"task_started","task_id":"child-task","tool_use_id":"tool-child"}),
        );
        consume(
            &mut run,
            json!({"type":"system","subtype":"permission_denied","agent_id":"child-task","parent_tool_use_id":"tool-child"}),
        );
        assert_eq!(run.subagents.len(), 2);
        assert_eq!(run.subagents[0].prompt, "Full parent task");
        assert_eq!(run.subagents[1].parent_id.as_deref(), Some("parent-task"));
        assert_eq!(run.subagents[1].status, Status::Attention);
        consume(
            &mut run,
            json!({"type":"result","agent_id":"child-task","parent_tool_use_id":"tool-child","session_id":"child-session","is_error":true,"result":"Denied"}),
        );
        assert_eq!(run.subagents[1].status, Status::Failed);
        assert_eq!(run.subagents[1].result, "Denied");
        assert_eq!(run.session_id.as_deref(), Some("root"));
        assert!(!run.completion_seen);
        assert!(run.failure.is_none());
    }

    #[test]
    fn non_agent_tasks_and_invalid_ids_are_not_registered() {
        let mut run = Run::fixture(Provider::Claude);
        consume(
            &mut run,
            json!({"type":"system","subtype":"task_started","task_id":"bash-task","task_type":"local_bash","description":"Run tests"}),
        );
        consume(
            &mut run,
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"../unsafe","input":{"prompt":"Ignore"}}]}}),
        );
        assert!(run.subagents.is_empty());
    }

    #[test]
    fn records_are_bounded_and_old_records_remain_compatible() {
        let mut run = Run::fixture(Provider::Codex);
        for index in 0..100 {
            let id = format!("child-{index}");
            consume(
                &mut run,
                json!({"type":"item.completed","item":{"type":"collab_tool_call","receiver_thread_ids":[id],"prompt":"😀\\\"".repeat(1000),"agents_states":{id:{"status":"running","message":"😀\\\"".repeat(4000)}}}}),
            );
        }
        assert_eq!(run.subagents.len(), LIMIT);
        assert!(
            run.subagents
                .iter()
                .all(|agent| agent.status == Status::Working)
        );
        assert!(run.subagents_omitted);
        assert!(serde_json::to_vec(&run).unwrap().len() < 1024 * 1024);
        let mut old = serde_json::to_value(Run::fixture(Provider::Codex)).unwrap();
        old.as_object_mut().unwrap().remove("subagents");
        old.as_object_mut().unwrap().remove("subagents_omitted");
        assert!(
            serde_json::from_value::<Run>(old)
                .unwrap()
                .subagents
                .is_empty()
        );
        let mut first = run.subagents[0].clone();
        first.parent_id = Some(first.id.clone());
        assert_eq!(depth(std::slice::from_ref(&first), &first), 0);
    }
}
