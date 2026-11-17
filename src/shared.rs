use crate::*;
use std::process::Stdio;
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
        let prs = collar(repo, "pr");
        let issues = issues
            .join()
            .map_err(|_| anyhow::anyhow!("Issue reader panicked"))??;
        Ok((issues, prs?))
    })
}

fn collect_pages(
    mut read: impl FnMut(usize) -> Result<Vec<Value>>,
    maximum: usize,
) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for page in 1..=(maximum / 50 + 1) {
        let entries = read(page)?;
        let count = entries.len();
        records.extend(entries);
        ensure!(
            records.len() <= maximum,
            "Listing exceeds {maximum} records; refusing a partial snapshot"
        );
        if count < 50 {
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
    ensure!(
        task["source"]["kind"] == "issue",
        "Select a GCD issue; PRs are changed on GCD"
    );
    let repo = string(task, "repo");
    ensure!(repo_valid(repo), "Task has no valid repository");
    let number = task["source"]["number"]
        .as_u64()
        .context("Issue has no number")?;
    Ok(format!("repos/{repo}/issues/{number}"))
}
fn write(path: &str, method: &str, body: &Value) -> Result<Value> {
    let mut child = collar_command()
        .args([
            "--host",
            "https://git.cmu.dev",
            "api",
            path,
            "--method",
            method,
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Run Collar")?;
    let stdin = child.stdin.take().context("Collar stdin is unavailable")?;
    serde_json::to_writer(stdin, body)?;
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "GCD write failed; inspect the issue before retrying: {}",
        clean(&String::from_utf8_lossy(&output.stderr))
    );
    serde_json::from_slice(&output.stdout)
        .context("GCD write may have completed; refresh before retrying")
}
pub(crate) fn create(repo: &str, title: &str) -> Result<Value> {
    ensure!(repo_valid(repo), "Choose an explicit project repository");
    ensure!(
        !title.trim().is_empty() && title.len() <= 300,
        "Title must be 1-300 bytes"
    );
    write(
        &format!("repos/{repo}/issues"),
        "POST",
        &json!({"title":title.trim(),"body":""}),
    )
}
fn verify(expected: &Value, current: &Value) -> Result<()> {
    ensure!(expected.is_object(), "Refresh this issue before editing it");
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
            "Issue changed on GCD; refresh and review it before writing"
        );
    }
    Ok(())
}
pub(crate) fn update(task: &Value, field: &str, value: &str) -> Result<Value> {
    let path = endpoint(task)?;
    let current = governance::api(&path, &[])?;
    verify(&task["source"]["remote"], &current)?;
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
            let repo = string(task, "repo");
            if field == "stage" && value == "done" {
                ensure!(task["source"]["state"] == "OPEN", "Issue is already closed");
                return change_state(task);
            }
            let local = list(&format!("repos/{repo}/labels"), &[], 1000)?;
            let org = list("orgs/ScottyLabs/labels", &[], 1000)?;
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
                matching.is_some() || (field == "stage" && value == "backlog"),
                "This repository has no shared label for that value. Configure its workflow on GCD; Shepherd will not create labels."
            );
            if field == "stage" && string(&current, "state") == "closed" {
                let normalized = normalize(&current);
                let mut opened = normalized;
                opened["state"] = json!("OPEN");
                ensure!(
                    stage(&opened, "issue") == value,
                    "Reopen the issue on GCD before changing its stage"
                );
                return write(&path, "PATCH", &json!({"state":"open"}));
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
            let latest = governance::api(&path, &[])?;
            verify(&current, &latest)?;
            write(
                &format!("{path}/labels"),
                "PUT",
                &json!({"labels":retained}),
            )?;
            return governance::api(&path, &[]).context(
                "Labels were updated on GCD but the refresh failed; inspect before retrying",
            );
        }
        _ => bail!("Unsupported shared issue field"),
    };
    write(&path, "PATCH", &payload)
}

pub(crate) fn publish_run(repository: &str, number: u64, body: &str) -> Result<Value> {
    ensure!(repo_valid(repository), "Invalid GCD repository");
    ensure!(
        !body.trim().is_empty() && body.len() <= 100000,
        "Invalid agent report length"
    );
    write(
        &format!("repos/{repository}/issues/{number}/comments"),
        "POST",
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
fn native_write(task: &Value, operation: &str, body: Option<&str>) -> Result<Value> {
    let (kind, repo, number) = discussion(task)?;
    ensure!(
        matches!(operation, "comment" | "close" | "reopen"),
        "Unsupported Collar action"
    );
    let mut command = collar_command();
    command.args([
        "--host",
        "https://git.cmu.dev",
        "-R",
        repo,
        kind,
        operation,
        &number.to_string(),
        "--json",
    ]);
    if body.is_some() {
        command.args(["--body-file", "-"]);
    }
    let mut child = command
        .stdin(if body.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Start Collar")?;
    if let Some(body) = body {
        let delivery = child
            .stdin
            .take()
            .context("Collar stdin is unavailable")?
            .write_all(body.as_bytes());
        if let Err(error) = delivery {
            let _ = child.wait();
            return Err(error)
                .context("Comment submission is uncertain; inspect GCD before sending again");
        }
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "Collar action failed; inspect GCD before trying again: {}",
        clean(&String::from_utf8_lossy(&output.stderr))
    );
    serde_json::from_slice(&output.stdout)
        .context("Collar action may have completed; refresh GCD before trying again")
}
pub(crate) fn comment(task: &Value, body: &str) -> Result<Value> {
    ensure!(
        !body.trim().is_empty() && body.len() <= 20000,
        "Comment must be 1-20000 bytes"
    );
    native_write(task, "comment", Some(body))
}
pub(crate) fn change_state(task: &Value) -> Result<Value> {
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
    let current = governance::api(&path, &[])?;
    if kind == "issue" {
        verify(&task["source"]["remote"], &current)?;
    } else {
        ensure!(
            current["merged"] != true
                && string(&current, "state").to_uppercase() == state
                && current["title"] == task["title"]
                && current["body"].as_str().unwrap_or("") == string(task, "body")
                && current["head"]["sha"] == task["source"]["head"],
            "PR changed on GCD; refresh and review it before changing its state"
        );
    }
    let result = native_write(task, if state == "OPEN" { "close" } else { "reopen" }, None)?;
    if kind == "issue" {
        return governance::api(&path, &[]).map(|raw| normalize(&raw)).context("Issue state changed through Collar, but refreshing it failed. Inspect GCD before trying again");
    }
    Ok(result)
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
        for total in [0, 49, 50, 51, 100, 102, 5000] {
            let mut pages = Vec::new();
            let records = collect_pages(
                |page| {
                    pages.push(page);
                    let start = (page - 1) * 50;
                    Ok((start..(start + 50).min(total))
                        .map(|number| json!({"number":number}))
                        .collect())
                },
                5000,
            )?;
            assert_eq!(records.len(), total);
            assert_eq!(pages.len(), total / 50 + 1);
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
