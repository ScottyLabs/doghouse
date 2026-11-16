use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct Catalog {
    pub revision: String,
    pub fetched_at: String,
    pub workspaces: Vec<Workspace>,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Workspace {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub leads: Vec<String>,
    pub members: Vec<String>,
    pub projects: Vec<Project>,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Project {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub repositories: Vec<String>,
}
#[derive(Deserialize)]
struct Team {
    name: String,
    slug: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    leads: Vec<String>,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    projects: Vec<Group>,
    #[serde(default)]
    repos: Vec<Repository>,
}
#[derive(Deserialize)]
struct Group {
    name: String,
    slug: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    repos: Vec<Repository>,
}
#[derive(Deserialize)]
struct Repository {
    name: String,
}
fn project(group: Group) -> Result<Project> {
    ensure!(
        !group.slug.is_empty() && !group.name.is_empty(),
        "Governance project has no identity"
    );
    let repositories = group
        .repos
        .into_iter()
        .map(|repo| {
            let name = if repo.name.contains('/') {
                repo.name
            } else {
                format!("ScottyLabs/{}", repo.name)
            };
            ensure!(repo_valid(&name), "Invalid Governance repository name");
            Ok(name)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Project {
        slug: group.slug,
        name: group.name,
        description: group.description.unwrap_or_default(),
        repositories,
    })
}
pub(crate) fn parse_team(value: &Value) -> Result<Workspace> {
    let team: Team =
        serde_json::from_value(value.clone()).context("Read Collar Governance team")?;
    ensure!(
        !team.slug.is_empty() && !team.name.is_empty(),
        "Governance team has no identity"
    );
    let mut projects = team
        .projects
        .into_iter()
        .map(project)
        .collect::<Result<Vec<_>>>()?;
    if !team.repos.is_empty() {
        projects.insert(
            0,
            project(Group {
                name: format!("{} repositories", team.name),
                slug: team.slug.clone(),
                description: team.description.clone(),
                repos: team.repos,
            })?,
        )
    }
    let mut seen = BTreeSet::new();
    ensure!(
        projects.iter().all(|p| seen.insert(p.slug.clone())),
        "Duplicate Governance project identity"
    );
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Workspace {
        slug: team.slug,
        name: team.name,
        description: team.description.unwrap_or_default(),
        leads: team.leads,
        members: team.members,
        projects,
    })
}
pub(crate) fn api(path: &str, fields: &[(&str, &str)]) -> Result<Value> {
    let mut command = collar_command();
    command.args([
        "--host",
        "https://git.cmu.dev",
        "api",
        path,
        "--method",
        "GET",
    ]);
    for (name, value) in fields {
        command.args(["-f", &format!("{name}={value}")]);
    }
    let output = command
        .output()
        .context("Run Collar; install Collar and sign in with collar auth login")?;
    ensure!(
        output.status.success(),
        "Collar read failed: {}",
        clean(&String::from_utf8_lossy(&output.stderr))
    );
    serde_json::from_slice(&output.stdout).context("Invalid Collar API JSON")
}
pub(crate) fn discover() -> Result<Catalog> {
    let output = collar_command()
        .args([
            "--host",
            "https://git.cmu.dev",
            "governance",
            "team",
            "list",
            "--json",
        ])
        .env_remove("COLLAR_GOVERNANCE")
        .output()
        .context("Run Collar Governance discovery")?;
    ensure!(
        output.status.success(),
        "Governance discovery failed. Update Collar if team list is unavailable: {}",
        clean(&String::from_utf8_lossy(&output.stderr))
    );
    let records: Vec<Value> =
        serde_json::from_slice(&output.stdout).context("Invalid Collar team JSON")?;
    from_records(records)
}
pub(crate) fn from_records(records: Vec<Value>) -> Result<Catalog> {
    ensure!(!records.is_empty(), "Governance returned no teams");
    let revision = string(&records[0]["dataSource"], "revision").to_owned();
    ensure!(
        revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit()),
        "Governance must come from a canonical revision"
    );
    let mut workspaces = Vec::new();
    let mut teams = BTreeSet::new();
    let mut repos = BTreeSet::new();
    for record in records {
        ensure!(
            string(&record["dataSource"], "revision") == revision,
            "Governance team snapshot revisions disagree"
        );
        ensure!(
            record["dataSource"]["kind"] == "snapshot"
                && record["dataSource"]["repository"] == "ScottyLabs/governance"
                && string(&record["dataSource"], "host").trim_end_matches('/')
                    == "https://git.cmu.dev",
            "Governance must come from the canonical GCD snapshot"
        );
        let workspace = parse_team(&record)?;
        ensure!(
            teams.insert(workspace.slug.clone()),
            "Duplicate Governance team identity"
        );
        for project in &workspace.projects {
            for repo in &project.repositories {
                ensure!(
                    repos.insert(repo.to_lowercase()),
                    "Ambiguous Governance repository ownership: {repo}"
                )
            }
        }
        workspaces.push(workspace);
    }
    workspaces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Catalog {
        revision,
        fetched_at: now(),
        workspaces,
    })
}
pub(crate) fn cached(store: &Store) -> Result<Catalog> {
    Ok(store.catalog.clone())
}
pub(crate) fn apply(store: &mut Store, catalog: Catalog) -> Result<()> {
    let mut next = store.data.clone();
    next["repositories"] = json!(
        catalog
            .workspaces
            .iter()
            .flat_map(|w| w.projects.iter().flat_map(|p| p
                .repositories
                .iter()
                .map(|r| json!({"name":r,"team":w.slug,"project":p.slug}))))
            .collect::<Vec<_>>()
    );
    next["governance"] = serde_json::to_value(&catalog)?;
    let previous = std::mem::replace(&mut store.data, next);
    if let Err(error) = store.save() {
        store.data = previous;
        return Err(error);
    }
    store.catalog = catalog;
    Ok(())
}

pub(crate) fn repositories(
    catalog: &Catalog,
    workspace: Option<&str>,
    project: Option<&str>,
) -> Result<Vec<String>> {
    ensure!(
        workspace.is_none_or(|slug| catalog.workspaces.iter().any(|w| w.slug == slug)),
        "Unknown Governance workspace"
    );
    let teams = catalog
        .workspaces
        .iter()
        .filter(|w| workspace.is_none_or(|s| w.slug == s));
    let mut repos = Vec::new();
    let mut found = project.is_none();
    for team in teams {
        for item in &team.projects {
            if project.is_none_or(|s| item.slug == s) {
                found = true;
                repos.extend(item.repositories.clone())
            }
        }
    }
    ensure!(found, "Unknown Governance project in this workspace");
    Ok(repos)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn source() -> Value {
        json!({"kind":"snapshot","host":"https://git.cmu.dev/","repository":"ScottyLabs/governance","revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})
    }
    pub(crate) fn records() -> Vec<Value> {
        vec![
            json!({"slug":"devops","name":"DevOps","description":null,"leads":["lead"],"members":[],"repos":[],"projects":[{"slug":"infrastructure","name":"Infrastructure","description":"Shared infrastructure","repos":[{"name":"infrastructure"},{"name":"infrastructure-vendor"}]},{"slug":"changelog","name":"Changelog","description":null,"repos":[]}],"dataSource":source()}),
            json!({"slug":"cmu-housing","name":"CMU Housing","description":null,"repos":[],"projects":[{"slug":"housing","name":"Housing","repos":[{"name":"housing"}]}],"dataSource":source()}),
            json!({"slug":"empty-team","name":"Empty team","projects":[],"repos":[],"dataSource":source()}),
        ]
    }
    #[test]
    fn full_canonical_hierarchy_includes_empty_groups_and_multi_repo_projects() -> Result<()> {
        let catalog = from_records(records())?;
        assert_eq!(catalog.workspaces.len(), 3);
        let selected_repos = repositories(&catalog, Some("devops"), Some("infrastructure"))?;
        assert_eq!(
            selected_repos,
            vec![
                "ScottyLabs/infrastructure",
                "ScottyLabs/infrastructure-vendor"
            ]
        );
        assert!(repositories(&catalog, Some("devops"), Some("changelog"))?.is_empty());
        assert!(repositories(&catalog, Some("devops"), Some("housing")).is_err());
        Ok(())
    }
    #[test]
    fn checkout_and_mixed_revision_catalogs_are_rejected() {
        let mut input = records();
        input[0]["dataSource"]["kind"] = json!("checkout");
        assert!(from_records(input).is_err());
        let mut input = records();
        input[1]["dataSource"]["revision"] = json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert!(from_records(input).is_err());
    }
}
