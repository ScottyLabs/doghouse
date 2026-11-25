use std::{fs, process::Command};

#[test]
fn imports_checkout_board_into_renamed_cache_once() {
    let temporary = tempfile::tempdir().unwrap();
    let checkout = temporary.path();
    let cache = checkout.join("cache");
    let legacy = checkout.join(".shepherd");
    fs::create_dir(&legacy).unwrap();
    let board = br#"{"version":1,"repositories":[],"tasks":[]}"#;
    fs::write(legacy.join("board.json"), board).unwrap();

    let read_board = || {
        let output = Command::new(env!("CARGO_BIN_EXE_doghouse"))
            .current_dir(checkout)
            .env("XDG_CACHE_HOME", &cache)
            .env_remove("DOGHOUSE_DATA_DIR")
            .arg("board")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    read_board();
    let migrated = cache.join("doghouse/board.json");
    assert_eq!(fs::read(&migrated).unwrap(), board);
    fs::write(legacy.join("board.json"), b"{}").unwrap();
    read_board();
    assert_eq!(fs::read(migrated).unwrap(), board);
}
