use crate::*;
use collar::Method;
use std::sync::{Condvar, Mutex};

const READ_WORKERS: usize = 4;
static READS: (Mutex<(usize, usize)>, Condvar) = (Mutex::new((0, 0)), Condvar::new());
pub(crate) struct ReadPermit {
    background: bool,
}
pub(crate) fn read_permit() -> Result<ReadPermit> {
    permit(false)
}
pub(crate) fn background_read_permit() -> Result<ReadPermit> {
    permit(true)
}
fn permit(background: bool) -> Result<ReadPermit> {
    let mut count = READS
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("Read scheduler unavailable"))?;
    while count.0 >= READ_WORKERS || (background && count.1 >= READ_WORKERS - 1) {
        count = READS
            .1
            .wait(count)
            .map_err(|_| anyhow::anyhow!("Read scheduler unavailable"))?;
    }
    count.0 += 1;
    if background {
        count.1 += 1;
    }
    Ok(ReadPermit { background })
}
impl Drop for ReadPermit {
    fn drop(&mut self) {
        if let Ok(mut count) = READS.0.lock() {
            count.0 -= 1;
            if self.background {
                count.1 -= 1;
            }
            READS.1.notify_all();
        }
    }
}

pub(crate) fn parallel_reads<T: Send, R: Send>(
    items: Vec<T>,
    read: impl Fn(T) -> R + Sync,
) -> Result<Vec<R>> {
    let workers = items.len().min(READ_WORKERS);
    let queue = Mutex::new(items.into_iter().enumerate());
    std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|_| {
                scope.spawn(|| -> Result<Vec<_>> {
                    let mut results = Vec::new();
                    loop {
                        let item = queue
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Read queue unavailable"))?
                            .next();
                        let Some((index, item)) = item else { break };
                        results.push((index, read(item)));
                    }
                    Ok(results)
                })
            })
            .collect::<Vec<_>>();
        let mut results = Vec::new();
        for handle in handles {
            results.extend(
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("Read worker panicked"))??,
            );
        }
        results.sort_by_key(|(index, _)| *index);
        Ok(results.into_iter().map(|(_, result)| result).collect())
    })
}

pub(crate) fn snapshot(repo: &str) -> Result<(Vec<Value>, Vec<Value>)> {
    std::thread::scope(|scope| {
        let issues = scope.spawn(|| issues(repo));
        let prs = pulls(repo);
        let issues = issues
            .join()
            .map_err(|_| anyhow::anyhow!("Issue reader panicked"))??;
        Ok((issues, prs?))
    })
}

pub(crate) fn collect_pages(
    mut read: impl FnMut(usize) -> Result<Vec<Value>>,
    maximum: usize,
) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for page in 1..=maximum.saturating_add(1) {
        let entries = read(page)?;
        let count = entries.len();
        records.extend(entries);
        ensure!(
            records.len() <= maximum,
            "Listing exceeds {maximum} records; refusing a partial snapshot"
        );
        if count == 0 {
            return Ok(records);
        }
    }
    bail!("Pagination did not terminate")
}
pub(crate) fn list(path: &str, fields: &[(&str, &str)], maximum: usize) -> Result<Vec<Value>> {
    list_read(path, fields, maximum, false)
}
pub(crate) fn background_list(
    path: &str,
    fields: &[(&str, &str)],
    maximum: usize,
) -> Result<Vec<Value>> {
    list_read(path, fields, maximum, true)
}
fn list_read(
    path: &str,
    fields: &[(&str, &str)],
    maximum: usize,
    background: bool,
) -> Result<Vec<Value>> {
    collect_pages(
        |page| {
            let page = page.to_string();
            let mut query = fields.to_vec();
            query.extend([("page", page.as_str()), ("limit", "50")]);
            let response = if background {
                governance::background_api(path, &query)
            } else {
                governance::api(path, &query)
            }?;
            response
                .as_array()
                .cloned()
                .context("Unexpected GCD listing")
        },
        maximum,
    )
}
pub(crate) fn issues(repo: &str) -> Result<Vec<Value>> {
    ensure!(repo_valid(repo), "Use owner/repository");
    let records = background_list(
        &format!("repos/{repo}/issues"),
        &[("state", "all"), ("type", "issues"), ("sort", "oldest")],
        5000,
    )?;
    let mut seen = std::collections::BTreeSet::new();
    records
        .into_iter()
        .filter(|raw| !raw["pull_request"].is_object())
        .map(|raw| {
            let number = raw["number"].as_u64().context("Issue has no number")?;
            ensure!(
                seen.insert(number),
                "Issue listing changed during pagination; refresh again"
            );
            Ok(normalize(&raw))
        })
        .collect()
}
pub(crate) fn pulls(repo: &str) -> Result<Vec<Value>> {
    ensure!(repo_valid(repo), "Use owner/repository");
    let records = background_list(
        &format!("repos/{repo}/pulls"),
        &[("state", "all"), ("sort", "oldest")],
        5000,
    )?;
    let mut seen = std::collections::BTreeSet::new();
    records
        .into_iter()
        .map(|raw| {
            let number = raw["number"].as_u64().context("PR has no number")?;
            ensure!(
                seen.insert(number),
                "PR listing changed during pagination; refresh again"
            );
            Ok(personal::normalize_pr(&raw))
        })
        .collect()
}
pub(crate) fn closing_references(body: &str) -> Vec<u64> {
    let mut result = Vec::new();
    let mut closing = false;
    for word in body.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, ',' | '.' | ':' | ';'));
        if let Some(reference) = word.strip_prefix('#') {
            if closing
                && let Ok(number) = reference
                    .trim_end_matches(|c: char| !c.is_ascii_digit())
                    .parse::<u64>()
            {
                result.push(number)
            }
        } else {
            closing = matches!(
                word.to_ascii_lowercase().as_str(),
                "fix"
                    | "fixes"
                    | "fixed"
                    | "close"
                    | "closes"
                    | "closed"
                    | "resolve"
                    | "resolves"
                    | "resolved"
            );
        }
    }
    result
}
pub(crate) fn normalize(raw: &Value) -> Value {
    json!({"number":raw["number"],"title":raw["title"],"body":raw["body"],"state":string(raw,"state").to_uppercase(),"url":raw["html_url"],"labels":raw["labels"],"author":{"login":raw["user"]["login"]},"_remote":raw})
}
pub(crate) fn stage(record: &Value, kind: &str) -> &'static str {
    if matches!(string(record, "state"), "CLOSED" | "MERGED") {
        return "done";
    }
    if kind == "pr" {
        return if record["isDraft"] == true {
            "in-progress"
        } else {
            "review"
        };
    }
    if let Some(labels) = record["labels"].as_array() {
        for label in labels {
            match string(label, "name") {
                "stage/backlog" | "stage/todo" => return "backlog",
                "stage/ready" => return "ready",
                "stage/in progress" | "stage/in-progress" => return "in-progress",
                "stage/review" | "stage/in-review" => return "review",
                "stage/done" => return "done",
                _ => {}
            }
        }
    }
    if record["_remote"]["assignees"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        "in-progress"
    } else {
        "backlog"
    }
}
pub(crate) fn priority(record: &Value) -> &'static str {
    let labels = record["labels"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if labels.iter().any(|l| {
        matches!(
            string(l, "name"),
            "priority/high" | "urgency/high" | "urgency/immediate"
        )
    }) {
        "high"
    } else if labels
        .iter()
        .any(|l| matches!(string(l, "name"), "priority/low" | "urgency/low"))
    {
        "low"
    } else {
        "normal"
    }
}
fn endpoint(task: &Value) -> Result<String> {
    let (kind, repo, number) = discussion(task)?;
    Ok(format!(
        "repos/{repo}/{}/{number}",
        if kind == "pr" { "pulls" } else { "issues" }
    ))
}
fn write(path: &str, method: Method, body: &Value) -> Result<Value> {
    forge::client()?.write(path, method, body)
}
pub(crate) fn create(repo: &str, title: &str) -> Result<Value> {
    ensure!(repo_valid(repo), "Choose an explicit project repository");
    ensure!(
        !title.trim().is_empty() && title.len() <= 300,
        "Title must be 1-300 bytes"
    );
    write(
        &format!("repos/{repo}/issues"),
        Method::POST,
        &json!({"title":title.trim(),"body":""}),
    )
}
fn verify(expected: &Value, current: &Value) -> Result<()> {
    ensure!(expected.is_object(), "Refresh this item before editing it");
    for field in [
        "updated_at",
        "title",
        "body",
        "state",
        "labels",
        "assignees",
    ] {
        ensure!(
            expected[field] == current[field],
            "Item changed on GCD; refresh and review it before writing"
        );
    }
    Ok(())
}
pub(crate) fn update(task: &Value, field: &str, value: &str) -> Result<Value> {
    update_with(forge::client()?.as_ref(), task, field, value)
}
fn update_with(client: &forge::Client, task: &Value, field: &str, value: &str) -> Result<Value> {
    let path = endpoint(task)?;
    let current = client.read(&path, &[])?;
    verify_task(task, &current)?;
    let (_, repo, number) = discussion(task)?;
    let issue_path = format!("repos/{repo}/issues/{number}");
    let payload = match field {
        "title" => {
            ensure!(
                !value.trim().is_empty() && value.len() <= 300,
                "Title must be 1-300 bytes"
            );
            json!({"title":value})
        }
        "body" => {
            ensure!(value.len() <= 20000, "Description too long");
            json!({"body":value})
        }
        "owner" => {
            json!({"assignees":value.split(',').map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>()})
        }
        "stage" | "priority" => {
            ensure!(
                field != "stage" || task["source"]["kind"] == "issue",
                "PR stage follows its draft and merge state; use close/reopen for its state"
            );
            let repo = string(task, "repo");
            if field == "stage" && value == "done" {
                ensure!(task["source"]["state"] == "OPEN", "Issue is already closed");
                return change_state_with(client, task);
            }
            let local = client.list(&format!("repos/{repo}/labels"), &[], 1000)?;
            let (owner, _) = repo.split_once('/').context("Invalid repository")?;
            let org = match client.list(&format!("orgs/{owner}/labels"), &[], 1000) {
                Ok(labels) => labels,
                Err(error) if collar::HttpStatus::is(&error, 404) => Vec::new(),
                Err(error) => return Err(error),
            };
            let available = local.iter().chain(org.iter());
            let names: &[&str] = match (field, value) {
                ("stage", "backlog") => &["stage/backlog", "stage/todo"],
                ("stage", "ready") => &["stage/ready"],
                ("stage", "in-progress") => &["stage/in-progress", "stage/in progress"],
                ("stage", "review") => &["stage/review", "stage/in-review"],
                ("priority", "high") => &["priority/high", "urgency/high"],
                ("priority", "normal") => &["priority/medium", "urgency/medium"],
                ("priority", "low") => &["priority/low", "urgency/low"],
                _ => bail!("Unknown task value"),
            };
            let matching = available
                .filter(|l| names.contains(&string(l, "name")))
                .collect::<Vec<_>>();
            let matching = names.iter().find_map(|name| {
                matching
                    .iter()
                    .find(|l| string(l, "name") == *name)
                    .copied()
            });
            ensure!(
                matching.is_some()
                    || (field == "stage" && value == "backlog")
                    || (field == "priority" && value == "normal"),
                "This repository has no shared label for that value. Configure its workflow on GCD; Doghouse will not create labels."
            );
            if field == "stage" && string(&current, "state") == "closed" {
                let normalized = normalize(&current);
                let mut opened = normalized;
                opened["state"] = json!("OPEN");
                ensure!(
                    stage(&opened, "issue") == value,
                    "Reopen the issue on GCD before changing its stage"
                );
                return client.write(&path, Method::PATCH, &json!({"state":"open"}));
            }
            let labels = current["labels"]
                .as_array()
                .context("Issue labels missing")?;
            let mut retained = labels
                .iter()
                .filter(|label| {
                    let name = string(label, "name");
                    if field == "stage" {
                        !name.starts_with("stage/")
                    } else {
                        !name.starts_with("priority/") && !name.starts_with("urgency/")
                    }
                })
                .map(|l| l["id"].as_u64().context("Label ID missing"))
                .collect::<Result<Vec<_>>>()?;
            if let Some(label) = matching {
                retained.push(label["id"].as_u64().context("Label ID missing")?)
            }
            if field == "stage" && value == "backlog" && matching.is_none() {
                let mut unlabelled = current.clone();
                unlabelled["labels"] = json!(
                    current["labels"]
                        .as_array()
                        .context("Labels missing")?
                        .iter()
                        .filter(|l| !string(l, "name").starts_with("stage/"))
                        .cloned()
                        .collect::<Vec<_>>()
                );
                ensure!(
                    stage(&normalize(&unlabelled), "issue") == "backlog"
                        && task["source"]["derivedReview"] != true,
                    "This issue needs a shared stage/backlog label to override assignment or PR review. No changes were sent."
                );
            }
            let latest = client.read(&path, &[])?;
            verify(&current, &latest)?;
            verify_task(task, &latest)?;
            client.write(
                &format!("{issue_path}/labels"),
                Method::PUT,
                &json!({"labels":retained}),
            )?;
            return client.read(&path, &[]).context(
                "Labels were updated on GCD but the refresh failed; inspect before retrying",
            );
        }
        _ => bail!("Unsupported shared issue field"),
    };
    client.write(
        if field == "owner" { &issue_path } else { &path },
        Method::PATCH,
        &payload,
    )?;
    client
        .read(&path, &[])
        .context("Saved on GCD but refreshing failed; inspect before retrying")
}

pub(crate) fn publish_run(repository: &str, number: u64, body: &str) -> Result<Value> {
    ensure!(repo_valid(repository), "Invalid GCD repository");
    ensure!(
        !body.trim().is_empty() && body.len() <= 100000,
        "Invalid agent report length"
    );
    write(
        &format!("repos/{repository}/issues/{number}/comments"),
        Method::POST,
        &json!({"body":body}),
    )
}
fn discussion(task: &Value) -> Result<(&str, &str, u64)> {
    let kind = string(&task["source"], "kind");
    ensure!(
        matches!(kind, "issue" | "pr"),
        "Choose a shared issue or PR; local tasks use m to change stage"
    );
    let repo = string(task, "repo");
    ensure!(repo_valid(repo), "Invalid repository");
    let number = task["source"]["number"]
        .as_u64()
        .filter(|number| *number > 0)
        .context("Source has no number")?;
    Ok((kind, repo, number))
}
fn verify_task(task: &Value, current: &Value) -> Result<()> {
    verify(&task["source"]["remote"], current)?;
    if task["source"]["kind"] == "pr" {
        ensure!(
            current["head"]["sha"] == task["source"]["head"]
                && current["merged"] == task["source"]["remote"]["merged"],
            "PR changed on GCD; refresh before editing"
        );
    }
    Ok(())
}
pub(crate) fn comment(task: &Value, body: &str) -> Result<Value> {
    ensure!(
        !body.trim().is_empty() && body.len() <= 20000,
        "Comment must be 1-20000 bytes"
    );
    let (_, repo, number) = discussion(task)?;
    write(
        &format!("repos/{repo}/issues/{number}/comments"),
        Method::POST,
        &json!({"body":body}),
    )
}
pub(crate) fn change_state(task: &Value) -> Result<Value> {
    let raw = change_state_with(forge::client()?.as_ref(), task)?;
    Ok(if task["source"]["kind"] == "pr" {
        personal::normalize_pr(&raw)
    } else {
        normalize(&raw)
    })
}
fn change_state_with(client: &forge::Client, task: &Value) -> Result<Value> {
    let (kind, repo, number) = discussion(task)?;
    let state = string(&task["source"], "state");
    ensure!(
        matches!(state, "OPEN" | "CLOSED"),
        "Merged PRs cannot be closed or reopened"
    );
    let path = format!(
        "repos/{repo}/{}/{number}",
        if kind == "pr" { "pulls" } else { "issues" }
    );
    let current = client.read(&path, &[])?;
    verify_task(task, &current)?;
    ensure!(current["merged"] != true, "Merged PRs cannot change state");
    client.write(
        &path,
        Method::PATCH,
        &json!({"state":if state == "OPEN" {"closed"} else {"open"}}),
    )?;
    let raw = client
        .read(&path, &[])
        .context("State saved on GCD but refreshing failed; inspect before retrying")?;
    Ok(raw)
}

pub(crate) fn comments(task: &Value) -> Result<Vec<Value>> {
    let (_, repository, number) = discussion(task)?;
    let comments = list(
        &format!("repos/{repository}/issues/{number}/comments"),
        &[],
        1000,
    )?;
    let start = comments.len().saturating_sub(30);
    Ok(comments.into_iter().skip(start).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    use tiny_http::{Response, Server};

    fn fixture(
        steps: Vec<(&'static str, String, Option<Value>, Value)>,
    ) -> (forge::Client, std::thread::JoinHandle<()>) {
        let server = Server::http("127.0.0.1:0").unwrap();
        let cache = tempfile::tempdir().unwrap();
        let api = collar::Api::new(
            collar::host_url(&format!("http://{}", server.server_addr())).unwrap(),
            Some("fixture-token".into()),
        )
        .unwrap()
        .with_read_cache(collar::ReadCache {
            directory: Some(cache.path().into()),
            refresh: false,
        });
        let worker = std::thread::spawn(move || {
            let _cache = cache;
            for (method, path, body, response) in steps {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.method().as_str(), method);
                assert_eq!(request.url(), format!("/api/v1/{path}"));
                if let Some(body) = body {
                    let received: Value = serde_json::from_reader(request.as_reader()).unwrap();
                    assert_eq!(received, body);
                }
                request
                    .respond(Response::from_string(response.to_string()))
                    .unwrap();
            }
            assert!(
                server
                    .recv_timeout(Duration::from_millis(50))
                    .unwrap()
                    .is_none(),
                "Unexpected retry or extra write"
            );
        });
        (forge::Client { api }, worker)
    }

    fn remote_task(kind: &str) -> Value {
        let raw = json!({"number":7,"title":"Before","body":"Body","state":"open","merged":false,"draft":false,"updated_at":"v1","labels":[{"id":1,"name":"type/bug"},{"id":2,"name":"priority/low"}],"assignees":[{"login":"old"}],"head":{"sha":"head"}});
        json!({"repo":"ScottyLabs/example","title":"Before","body":"Body","source":{"kind":kind,"number":7,"state":"OPEN","head":"head","remote":raw}})
    }

    #[test]
    fn issue_and_pr_edits_use_the_correct_endpoint_and_refresh_metadata() {
        for kind in ["issue", "pr"] {
            for (field, value, payload) in [
                ("title", "After", json!({"title":"After"})),
                ("body", "New\nbody", json!({"body":"New\nbody"})),
                ("owner", "alice, bob", json!({"assignees":["alice","bob"]})),
            ] {
                let task = remote_task(kind);
                let path = endpoint(&task).unwrap();
                let write_path = if field == "owner" {
                    "repos/ScottyLabs/example/issues/7".into()
                } else {
                    path.clone()
                };
                let mut updated = task["source"]["remote"].clone();
                updated["updated_at"] = json!("v2");
                let (client, worker) = fixture(vec![
                    ("GET", path.clone(), None, task["source"]["remote"].clone()),
                    ("PATCH", write_path, Some(payload), updated.clone()),
                    ("GET", path, None, updated.clone()),
                ]);
                assert_eq!(update_with(&client, &task, field, value).unwrap(), updated);
                worker.join().unwrap();
            }
        }
    }

    #[test]
    fn pr_priority_preserves_unrelated_labels_and_uses_issue_label_endpoint() {
        let task = remote_task("pr");
        let path = endpoint(&task).unwrap();
        let current = task["source"]["remote"].clone();
        let mut updated = current.clone();
        updated["labels"] = json!([{"id":1,"name":"type/bug"},{"id":3,"name":"priority/high"}]);
        let (client, worker) = fixture(vec![
            ("GET", path.clone(), None, current.clone()),
            (
                "GET",
                "repos/ScottyLabs/example/labels?page=1&limit=50".into(),
                None,
                json!([{"id":3,"name":"priority/high"}]),
            ),
            (
                "GET",
                "repos/ScottyLabs/example/labels?page=2&limit=50".into(),
                None,
                json!([]),
            ),
            (
                "GET",
                "orgs/ScottyLabs/labels?page=1&limit=50".into(),
                None,
                json!([]),
            ),
            ("GET", path.clone(), None, current),
            (
                "PUT",
                "repos/ScottyLabs/example/issues/7/labels".into(),
                Some(json!({"labels":[1,3]})),
                updated["labels"].clone(),
            ),
            ("GET", path, None, updated.clone()),
        ]);
        assert_eq!(
            update_with(&client, &task, "priority", "high").unwrap(),
            updated
        );
        worker.join().unwrap();
    }

    #[test]
    fn normal_priority_can_clear_priority_labels_without_a_medium_label() {
        let task = remote_task("issue");
        let path = endpoint(&task).unwrap();
        let current = task["source"]["remote"].clone();
        let mut updated = current.clone();
        updated["labels"] = json!([{"id":1,"name":"type/bug"}]);
        let (client, worker) = fixture(vec![
            ("GET", path.clone(), None, current.clone()),
            (
                "GET",
                "repos/ScottyLabs/example/labels?page=1&limit=50".into(),
                None,
                json!([]),
            ),
            (
                "GET",
                "orgs/ScottyLabs/labels?page=1&limit=50".into(),
                None,
                json!([]),
            ),
            ("GET", path.clone(), None, current),
            (
                "PUT",
                "repos/ScottyLabs/example/issues/7/labels".into(),
                Some(json!({"labels":[1]})),
                updated["labels"].clone(),
            ),
            ("GET", path, None, updated.clone()),
        ]);
        assert_eq!(
            update_with(&client, &task, "priority", "normal").unwrap(),
            updated
        );
        worker.join().unwrap();
    }

    #[test]
    fn state_changes_preserve_raw_metadata_for_issues_and_unmerged_prs() {
        for (kind, state) in [
            ("issue", "OPEN"),
            ("pr", "OPEN"),
            ("issue", "CLOSED"),
            ("pr", "CLOSED"),
        ] {
            let mut task = remote_task(kind);
            task["source"]["state"] = json!(state);
            task["source"]["remote"]["state"] = json!(state.to_lowercase());
            let path = endpoint(&task).unwrap();
            let mut closed = task["source"]["remote"].clone();
            closed["state"] = json!(if state == "OPEN" { "closed" } else { "open" });
            let (client, worker) = fixture(vec![
                ("GET", path.clone(), None, task["source"]["remote"].clone()),
                (
                    "PATCH",
                    path.clone(),
                    Some(json!({"state":closed["state"]})),
                    closed.clone(),
                ),
                ("GET", path, None, closed.clone()),
            ]);
            assert_eq!(change_state_with(&client, &task).unwrap(), closed);
            assert_eq!(closed["merged"], false);
            assert_eq!(closed["labels"][0]["name"], "type/bug");
            worker.join().unwrap();
        }
    }

    #[test]
    fn changed_pr_heads_and_stale_issue_metadata_prevent_writes() {
        for kind in ["issue", "pr"] {
            let task = remote_task(kind);
            let mut current = task["source"]["remote"].clone();
            if kind == "pr" {
                current["head"]["sha"] = json!("new-head");
            } else {
                current["updated_at"] = json!("v2");
            }
            let (client, worker) = fixture(vec![("GET", endpoint(&task).unwrap(), None, current)]);
            assert!(update_with(&client, &task, "title", "After").is_err());
            worker.join().unwrap();
        }
    }
    #[test]
    fn existing_stage_aliases_and_priorities_are_shared() {
        assert_eq!(
            stage(&json!({"labels":[{"name":"stage/in progress"}]}), "issue"),
            "in-progress"
        );
        assert_eq!(
            priority(&json!({"labels":[{"name":"urgency/immediate"}]})),
            "high"
        );
    }
    #[test]
    fn pagination_handles_fifty_item_server_cap_and_exact_boundaries() -> Result<()> {
        for capacity in [20, 50] {
            for total in [0, 49, 50, 51, 100, 102, 5000] {
                let mut pages = Vec::new();
                let records = collect_pages(
                    |page| {
                        pages.push(page);
                        let start = (page - 1) * capacity;
                        Ok((start..(start + capacity).min(total))
                            .map(|number| json!({"number":number}))
                            .collect())
                    },
                    5000,
                )?;
                assert_eq!(records.len(), total);
                assert_eq!(pages.len(), total.div_ceil(capacity) + 1);
            }
        }
        assert!(collect_pages(|_| Ok(vec![json!({}); 50]), 100).is_err());
        Ok(())
    }
    #[test]
    fn stale_remote_edits_are_rejected() {
        let initial = json!({"title":"Original","updated_at":"a"});
        let changed = json!({"title":"Changed","updated_at":"b"});
        assert!(verify(&initial, &changed).is_err());
        assert!(verify(&initial, &initial).is_ok());
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    #[test]
    fn reads_overlap_with_a_bounded_shared_budget_and_keep_result_order() -> Result<()> {
        let barrier = Arc::new(Barrier::new(READ_WORKERS));
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let values = parallel_reads((0..8).collect(), |index| -> Result<_> {
            let _permit = read_permit()?;
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            barrier.wait();
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(index)
        })?;
        assert_eq!(
            values.into_iter().collect::<Result<Vec<_>>>()?,
            (0..8).collect::<Vec<_>>()
        );
        assert_eq!(peak.load(Ordering::SeqCst), READ_WORKERS);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(parallel_reads(Vec::<()>::new(), |_| ())?.is_empty());
        let background = (0..READ_WORKERS - 1)
            .map(|_| background_read_permit())
            .collect::<Result<Vec<_>>>()?;
        // Refresh workers leave a slot available for a discussion or write precondition read.
        let foreground = read_permit()?;
        drop(foreground);
        drop(background);

        Ok(())
    }
}
