use crate::*;
use serde::{Deserialize, Serialize};

pub(crate) const WORKSPACE: &str = "@local";
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Project {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
}
pub(crate) fn projects(data: &Value) -> Result<Vec<Project>> {
    if data["localProjects"].is_null() {
        return Ok(vec![]);
    }
    serde_json::from_value(data["localProjects"].clone()).context("Invalid local project registry")
}
pub(crate) fn catalog(
    mut catalog: governance::Catalog,
    data: &Value,
) -> Result<governance::Catalog> {
    catalog = personal::catalog(catalog, data)?;
    let projects = projects(data)?;
    if !projects.is_empty() {
        catalog.workspaces.push(governance::Workspace {
            slug: WORKSPACE.into(),
            name: "Local projects".into(),
            description: "Directories on this machine; not published to Governance".into(),
            leads: vec![],
            members: vec![],
            projects: projects
                .into_iter()
                .map(|project| governance::Project {
                    slug: project.id,
                    name: project.name,
                    description: project.directory.display().to_string(),
                    repositories: vec![],
                })
                .collect(),
        });
    }
    Ok(catalog)
}
pub(crate) fn add(store: &mut Store, directory: PathBuf, name: Option<String>) -> Result<Project> {
    let directory = if let Ok(suffix) = directory.strip_prefix("~") {
        PathBuf::from(std::env::var_os("HOME").context("HOME is not configured")?).join(suffix)
    } else {
        directory
    };
    let directory = directory
        .canonicalize()
        .context("Choose an existing project directory")?;
    ensure!(directory.is_dir(), "Project path must be a directory");
    let name = name.unwrap_or_else(|| {
        directory
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    ensure!(
        !name.trim().is_empty() && name.len() <= 100,
        "Project name must be 1-100 bytes"
    );
    // Stable FNV-1a identity, including non-UTF-8 path bytes.
    let hash = directory
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    let project = Project {
        id: format!("local-{hash:016x}"),
        name: name.trim().into(),
        directory,
    };
    let mut entries = projects(&store.data)?;
    ensure!(
        !entries
            .iter()
            .any(|entry| entry.id == project.id && entry.directory != project.directory),
        "Project identity collision"
    );
    entries.retain(|entry| entry.id != project.id);
    entries.push(project.clone());
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let previous = store.data.clone();
    store.data["localProjects"] = serde_json::to_value(entries)?;
    if let Err(error) = store.save().and_then(|_| store.reload()) {
        store.data = previous;
        return Err(error);
    }
    Ok(project)
}
pub(crate) fn remove(store: &mut Store, id: &str) -> Result<()> {
    let mut entries = projects(&store.data)?;
    let count = entries.len();
    entries.retain(|entry| entry.id != id);
    ensure!(entries.len() != count, "Local project not found");
    let previous = store.data.clone();
    store.data["localProjects"] = serde_json::to_value(entries)?;
    if let Err(error) = store.save().and_then(|_| store.reload()) {
        store.data = previous;
        return Err(error);
    }
    Ok(())
}
pub(crate) fn task(store: &mut Store, project: &str, title: &str) -> Result<String> {
    let project = projects(&store.data)?
        .into_iter()
        .find(|p| p.id == project)
        .context("Select a local project first")?;
    store.add(title, None, "")?;
    let task = store.data["tasks"]
        .as_array_mut()
        .and_then(|tasks| tasks.last_mut())
        .context("Task missing")?;
    task["localProject"] = json!(project.id);
    task["directory"] = json!(project.directory);
    Ok(string(task, "id").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_directories_survive_governance_and_reregistration() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let directory = temporary.path().join("project");
        fs::create_dir(&directory)?;
        let link = temporary.path().join("alias");
        std::os::unix::fs::symlink(&directory, &link)?;
        let mut store = Store::open(temporary.path().join("cache"))?;
        let first = add(&mut store, directory.clone(), Some("Example".into()))?;
        let second = add(&mut store, link, Some("Renamed".into()))?;
        assert_eq!(first.id, second.id);
        assert_eq!(projects(&store.data)?.len(), 1);
        let id = task(&mut store, &first.id, "Review the prototype")?;
        store.save()?;
        governance::apply(
            &mut store,
            governance::from_records(governance::tests::records())?,
        )?;
        assert!(store.catalog.workspaces.iter().any(|w| w.slug == WORKSPACE));
        assert!(!store.data["governance"].to_string().contains(WORKSPACE));
        remove(&mut store, &first.id)?;
        assert!(directory.exists());
        assert_eq!(store.task_mut(&id)?["title"], "Review the prototype");
        let restored = add(&mut store, directory, None)?;
        assert_eq!(restored.id, first.id);
        assert_eq!(store.task_mut(&id)?["localProject"], restored.id);
        Ok(())
    }
}
