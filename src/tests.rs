use super::*;
#[test]
fn old_board_survives_local_edits() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::open(dir.path().into())?;
    store.add("Housing task", Some("ScottyLabs/housing"), "context")?;
    store.save()?;
    assert_eq!(store.tasks().len(), 1);
    let id = string(&store.tasks()[0], "id").to_owned();
    let t = store.task_mut(&id)?;
    t["claim"] = json!({"agent":"a"});
    assert!(claim_check(t, 1, "b").is_err());
    assert!(claim_check(t, 2, "a").is_err());
    assert!(claim_check(t, 1, "a").is_ok());
    let inherited_lock = store._lock.try_clone()?;
    drop(store);
    let reopened = Store::open(dir.path().into())?;
    assert_eq!(string(&reopened.tasks()[0], "title"), "Housing task");
    drop(inherited_lock);
    Ok(())
}
#[test]
fn ignores_fenced_todos() {
    assert_eq!(
        todos("- [x] done\n```\n- [ ] example\n```\n- [ ] real").len(),
        2
    )
}
#[test]
fn rejects_argument_injection() {
    assert!(!repo_valid("--config/x"));
    assert!(!repo_valid("a/b/c"));
    assert!(repo_valid("ScottyLabs/housing"))
}
#[test]
fn renderer_handles_common_sizes() -> Result<()> {
    use ratatui::{Terminal, backend::TestBackend};
    let dir = tempfile::tempdir()?;
    let mut store = Store::open(dir.path().into())?;
    store.add("Read real issues", None, "Description")?;
    for (w, h) in [(60, 12), (80, 24), (120, 36)] {
        let mut terminal = Terminal::new(TestBackend::new(w, h))?;
        let ui = crate::ui::Ui::default();
        terminal.draw(|f| ui.draw(f, &store))?;
    }
    Ok(())
}

#[test]
fn sync_keeps_identity_and_archives_local_plans() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::open(dir.path().into())?;
    let issue = json!({"number":4,"title":"Original","body":"- [ ] Check deploy","state":"OPEN","labels":[]});
    import_records(
        &mut store,
        "ScottyLabs/kennel",
        std::slice::from_ref(&issue),
        &[],
    )?;
    let id = string(&store.tasks()[0], "id").to_owned();
    store.task_mut(&id)?["stage"] = json!("ready");
    store.task_mut(&id)?["localStage"] = json!(true);
    let mut changed = issue;
    changed["title"] = json!("Updated upstream");
    import_records(&mut store, "ScottyLabs/kennel", &[changed], &[])?;
    assert_eq!(store.tasks().len(), 1);
    assert_eq!(store.tasks()[0]["id"], id);
    assert_eq!(store.tasks()[0]["stage"], "backlog");
    assert_eq!(store.tasks()[0]["localPlan"]["stage"], "ready");
    assert_eq!(store.tasks()[0]["title"], "Updated upstream");
    assert_eq!(
        store.tasks()[0]["source"]["todos"].as_array().map(Vec::len),
        Some(1)
    );
    let before = store.data.clone();
    assert!(
        import_records(
            &mut store,
            "ScottyLabs/kennel",
            &[json!({"title":"invalid"})],
            &[]
        )
        .is_err()
    );
    assert_eq!(store.data, before);
    Ok(())
}
#[test]
fn accepting_plan_creates_children_only_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::open(dir.path().into())?;
    store.add("Parent", None, "")?;
    let id = string(&store.tasks()[0], "id").to_owned();
    store.task_mut(&id)?["proposals"] =
        json!([{"status":"pending","tasks":[{"title":"Child","body":"Acceptance criteria"}]}]);
    assert_eq!(store.tasks().len(), 1);
    plan(&mut store, &id, false)?;
    assert_eq!(store.tasks().len(), 2);
    assert_eq!(store.tasks()[1]["parentId"], id);
    assert!(plan(&mut store, &id, false).is_err());
    assert_eq!(store.tasks().len(), 2);
    Ok(())
}

#[test]
fn complete_snapshots_retire_and_restore_sources_without_losing_history() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let mut store = Store::open(temporary.path().into())?;
    let issue = json!({"number":1,"title":"Issue","state":"OPEN","labels":[]});
    import_snapshot(
        &mut store,
        "ScottyLabs/example",
        std::slice::from_ref(&issue),
        &[],
        &now(),
    )?;
    let id = store.tasks()[0]["id"]
        .as_str()
        .context("Missing ID")?
        .to_owned();
    store.task_mut(&id)?["localNote"] = json!("Keep my notes");
    store.add("Local draft", Some("ScottyLabs/example"), "")?;
    import_records(&mut store, "ScottyLabs/example", &[], &[])?;
    assert_eq!(store.tasks().len(), 2);
    import_snapshot(&mut store, "ScottyLabs/example", &[], &[], &now())?;
    assert_eq!(store.tasks().len(), 1);
    assert!(store.tasks()[0]["source"].is_null());
    assert_eq!(store.data["retiredTasks"][0]["localNote"], "Keep my notes");
    import_snapshot(
        &mut store,
        "ScottyLabs/example",
        std::slice::from_ref(&issue),
        &[],
        &now(),
    )?;
    assert_eq!(store.task_mut(&id)?["localNote"], "Keep my notes");
    assert!(
        store.data["retiredTasks"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    let before = store.data.clone();
    assert!(
        import_snapshot(
            &mut store,
            "ScottyLabs/example",
            &[json!({"title":"invalid"})],
            &[],
            &now()
        )
        .is_err()
    );
    assert_eq!(store.data, before);
    Ok(())
}
#[test]
fn unchanged_derived_review_does_not_churn_revision_or_agent_context() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let mut store = Store::open(temporary.path().into())?;
    let issue = json!({"number":1,"title":"Issue","state":"OPEN","labels":[]});
    let pr = json!({"number":2,"title":"Fix","body":"Fixes #1","state":"OPEN","isDraft":false});
    import_snapshot(
        &mut store,
        "ScottyLabs/example",
        std::slice::from_ref(&issue),
        std::slice::from_ref(&pr),
        &now(),
    )?;
    let previous = store.tasks()[0].clone();
    assert_eq!(previous["stage"], "review");
    import_snapshot(
        &mut store,
        "ScottyLabs/example",
        std::slice::from_ref(&issue),
        std::slice::from_ref(&pr),
        &now(),
    )?;
    assert_eq!(store.tasks()[0]["revision"], previous["revision"]);
    assert_eq!(
        agents::task_context(&store.tasks()[0]),
        agents::task_context(&previous)
    );
    import_snapshot(&mut store, "ScottyLabs/example", &[issue], &[], &now())?;
    assert_eq!(store.tasks()[0]["stage"], "backlog");
    Ok(())
}

#[test]
fn snapshot_started_before_creation_keeps_the_newer_task() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let mut store = Store::open(temporary.path().into())?;
    let started = now();
    let issue = json!({"number":9,"title":"Created during refresh","state":"OPEN","labels":[]});
    import_records(&mut store, "ScottyLabs/example", &[issue], &[])?;
    import_snapshot(&mut store, "ScottyLabs/example", &[], &[], &started)?;
    assert_eq!(store.tasks().len(), 1);
    import_snapshot(&mut store, "ScottyLabs/example", &[], &[], &now())?;
    assert!(store.tasks().is_empty());
    Ok(())
}

#[test]
fn an_older_snapshot_cannot_undo_a_local_pr_state_refresh() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let mut store = Store::open(temporary.path().into())?;
    let open = json!({"number":1,"title":"Change","state":"OPEN","headRefOid":"abc"});
    import_records(
        &mut store,
        "ScottyLabs/example",
        &[],
        std::slice::from_ref(&open),
    )?;
    let started = now();
    let mut closed = open.clone();
    closed["state"] = json!("CLOSED");
    import_records(&mut store, "ScottyLabs/example", &[], &[closed])?;
    import_snapshot(&mut store, "ScottyLabs/example", &[], &[open], &started)?;
    assert_eq!(store.tasks()[0]["source"]["state"], "CLOSED");
    Ok(())
}

#[test]
fn unchanged_reload_skips_parsing_but_atomic_replacements_are_seen() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path().into())?;
    store.add("first", None, "")?;
    store.save()?;
    store.reload_if_changed()?;
    let pointer = store.tasks().as_ptr();
    store.reload_if_changed()?;
    assert_eq!(pointer, store.tasks().as_ptr());
    let mut replacement = store.data.clone();
    replacement["tasks"][0]["title"] = json!("other");
    let mut file = tempfile::NamedTempFile::new_in(directory.path())?;
    serde_json::to_writer(&mut file, &replacement)?;
    file.persist(directory.path().join("board.json"))?;
    store.reload_if_changed()?;
    assert_eq!(store.tasks()[0]["title"], "other");
    store.data["tasks"][0]["title"] = json!("unsaved");
    store.reload()?;
    assert_eq!(store.tasks()[0]["title"], "other");
    Ok(())
}

#[test]
fn invalid_atomic_replacement_is_not_marked_as_loaded() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path().into())?;
    store.add("retained", None, "")?;
    store.save()?;
    fs::write(directory.path().join("board.json"), b"invalid")?;
    assert!(store.reload_if_changed().is_err());
    assert!(store.reload_if_changed().is_err());
    assert_eq!(store.tasks()[0]["title"], "retained");
    Ok(())
}

#[test]
fn failed_save_invalidates_the_idle_reload_stamp() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path().into())?;
    store.add("saved", None, "")?;
    store.save()?;
    store.reload_if_changed()?;
    store.data["tasks"][0]["title"] = json!("unsaved");
    store.dir = directory.path().join("missing-parent");
    assert!(store.save().is_err());
    store.dir = directory.path().into();
    store.reload_if_changed()?;
    assert_eq!(store.tasks()[0]["title"], "saved");
    Ok(())
}
