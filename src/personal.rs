use crate::*;
use std::collections::{BTreeMap, BTreeSet};
pub(crate) const ACCESSIBLE: &str = "@accessible";
pub(crate) struct Snapshot {
    pub read_started: String,
    pub login: String,
    pub repositories: Vec<String>,
    pub roles: BTreeMap<String, Vec<String>>,
    pub records: Vec<(String, String, Value)>,
}
pub(crate) fn refresh() -> Result<Snapshot> {
    let read_started = now();
    let user = governance::api("user", &[])?;
    let login = user["login"]
        .as_str()
        .filter(|login| !login.is_empty())
        .context("Collar did not identify the signed-in user")?
        .to_owned();
    let mut repositories = shared::list("user/repos", &[], 5000)?
        .iter()
        .map(|repo| {
            repo["full_name"]
                .as_str()
                .context("Repository has no full name")
                .map(str::to_owned)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut roles: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut sources: BTreeMap<String, (String, String, u64)> = BTreeMap::new();
    for (kind, filter, role) in [
        ("issue", "assigned", "Assigned issue"),
        ("pr", "assigned", "Assigned PR"),
        ("pr", "review_requested", "Review requested"),
        ("pr", "created", "Your PR"),
    ] {
        let records = shared::list(
            "repos/issues/search",
            &[
                ("state", "open"),
                ("type", if kind == "pr" { "pulls" } else { "issues" }),
                (filter, "true"),
                ("sort", "recentupdate"),
            ],
            5000,
        )?;
        for record in records {
            ensure!(
                record["pull_request"].is_object() == (kind == "pr"),
                "Collar returned the wrong source type for My work"
            );
            let repository = record["repository"]["full_name"]
                .as_str()
                .context("Inbox item has no repository")?;
            ensure!(repo_valid(repository), "Invalid inbox repository");
            let number = record["number"]
                .as_u64()
                .context("Inbox item has no number")?;
            let id = format!("gcd:{repository}:{kind}:{number}");
            let entries = roles.entry(id.clone()).or_default();
            if !entries.iter().any(|entry| entry == role) {
                entries.push(role.into());
            }
            sources.insert(id, (repository.into(), kind.into(), number));
            repositories.insert(repository.into());
        }
    }
    let mut records = Vec::new();
    for (_, (repository, kind, number)) in sources {
        let raw = governance::api(
            &format!(
                "repos/{repository}/{}/{number}",
                if kind == "pr" { "pulls" } else { "issues" }
            ),
            &[],
        )?;
        let record = if kind == "pr" {
            normalize_pr(&raw)
        } else {
            shared::normalize(&raw)
        };
        records.push((repository, kind, record));
    }
    Ok(Snapshot {
        read_started,
        login,
        repositories: repositories.into_iter().collect(),
        roles,
        records,
    })
}
pub(crate) fn normalize_pr(raw: &Value) -> Value {
    json!({"number":raw["number"],"title":raw["title"],"body":raw["body"],"state":if raw["merged"]==true{"MERGED".into()}else{string(raw,"state").to_uppercase()},"url":raw["html_url"],"author":{"login":raw["user"]["login"]},"isDraft":raw["draft"],"headRefOid":raw["head"]["sha"],"labels":raw["labels"],"_remote":raw})
}
pub(crate) fn apply(store: &mut Store, snapshot: Snapshot) -> Result<()> {
    let cutoff = snapshot
        .read_started
        .parse::<u128>()
        .context("Invalid personal refresh timestamp")?;
    let mut grouped: BTreeMap<String, (Vec<Value>, Vec<Value>)> = BTreeMap::new();
    for (repository, kind, record) in snapshot.records {
        let id = format!("gcd:{repository}:{kind}:{}", record["number"]);
        if store
            .tasks()
            .iter()
            .find(|task| task["id"] == id)
            .is_some_and(|task| {
                task["source"]["syncedAt"]
                    .as_str()
                    .and_then(|timestamp| timestamp.parse::<u128>().ok())
                    .is_some_and(|timestamp| timestamp > cutoff)
            })
        {
            continue;
        }
        let (issues, prs) = grouped.entry(repository).or_default();
        if kind == "pr" {
            prs.push(record)
        } else {
            issues.push(record)
        }
    }
    for (repository, (issues, prs)) in grouped {
        import_records(store, &repository, &issues, &prs)?;
    }
    store.data["accountRepositories"] = json!(snapshot.repositories);
    store.data["myWork"] =
        json!({"login":snapshot.login,"roles":snapshot.roles,"refreshedAt":now()});
    store.save()?;
    store.reload()
}
pub(crate) fn catalog(
    mut catalog: governance::Catalog,
    data: &Value,
) -> Result<governance::Catalog> {
    let known = catalog
        .workspaces
        .iter()
        .flat_map(|w| w.projects.iter().flat_map(|p| p.repositories.iter()))
        .collect::<BTreeSet<_>>();
    let repositories = data["accountRepositories"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut projects = Vec::new();
    for repository in repositories {
        let name = repository
            .as_str()
            .context("Invalid cached account repository")?;
        ensure!(repo_valid(name), "Invalid cached account repository name");
        if known.iter().any(|entry| entry.as_str() == name) {
            continue;
        }
        projects.push(governance::Project {
            slug: name.into(),
            name: name.into(),
            description: "Repository visible to your Collar account".into(),
            repositories: vec![name.into()],
        });
    }
    if !projects.is_empty() {
        catalog.workspaces.push(governance::Workspace {
            slug: ACCESSIBLE.into(),
            name: "Other accessible repositories".into(),
            description: "Account repositories outside Governance teams".into(),
            leads: vec![],
            members: vec![],
            projects,
        });
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_catalog_adds_only_repositories_outside_governance() -> Result<()> {
        let catalog = governance::from_records(governance::tests::records())?;
        let result = super::catalog(
            catalog,
            &json!({"accountRepositories":["ScottyLabs/infrastructure","Other/project"]}),
        )?;
        let other = result
            .workspaces
            .iter()
            .find(|workspace| workspace.slug == ACCESSIBLE)
            .context("Missing accessible workspace")?;
        assert_eq!(other.projects.len(), 1);
        assert_eq!(other.projects[0].repositories, vec!["Other/project"]);
        Ok(())
    }
}
