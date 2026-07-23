use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn quotes_command(database: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_quotes"));
    command.arg("--database").arg(database);
    command
}

fn output_text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn assert_success(output: &Output) {
    let (stdout, stderr) = output_text(output);
    assert!(
        output.status.success(),
        "command failed\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn crud_commands_keep_data_on_stdout_and_diagnostics_on_stderr() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("quotes.db");

    let added = quotes_command(&database)
        .args(["add", "  Alpha  ", "--attribution", "  Author  "])
        .output()
        .unwrap();
    assert_success(&added);
    let (stdout, stderr) = output_text(&added);
    assert_eq!(stdout, "Added quote 1.\n");
    assert!(stderr.is_empty());

    let listed = quotes_command(&database)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert_success(&listed);
    let quotes: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(quotes[0]["id"], 1);
    assert_eq!(quotes[0]["text"], "Alpha");
    assert_eq!(quotes[0]["attribution"], "Author");
    assert_eq!(quotes[0]["display_width"], 14);
    assert!(listed.stderr.is_empty());

    let selected = quotes_command(&database)
        .args(["get", "--min-width", "14", "--max-width", "14"])
        .output()
        .unwrap();
    assert_success(&selected);
    assert_eq!(selected.stdout, b"Alpha - Author\n");
    assert!(selected.stderr.is_empty());

    let edited = quotes_command(&database)
        .args(["edit", "1", "--text", "Beta", "--clear-attribution"])
        .output()
        .unwrap();
    assert_success(&edited);
    let human = quotes_command(&database).arg("list").output().unwrap();
    assert_success(&human);
    let (stdout, stderr) = output_text(&human);
    assert!(stdout.contains("[  4] Beta"));
    assert!(stderr.is_empty());

    let unconfirmed = quotes_command(&database)
        .args(["remove", "1"])
        .output()
        .unwrap();
    assert_eq!(unconfirmed.status.code(), Some(2));
    assert!(unconfirmed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unconfirmed.stderr).contains("--yes"));

    let removed = quotes_command(&database)
        .args(["remove", "1", "--yes"])
        .output()
        .unwrap();
    assert_success(&removed);
    assert_eq!(removed.stdout, b"Removed quote 1.\n");
}

#[test]
fn no_matches_and_invalid_widths_fail_without_stdout() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("quotes.db");

    let empty = quotes_command(&database).arg("list").output().unwrap();
    assert_eq!(empty.status.code(), Some(1));
    assert!(empty.stdout.is_empty());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("no quotes matched"));

    assert_success(
        &quotes_command(&database)
            .args(["add", "cat"])
            .output()
            .unwrap(),
    );
    let no_width_match = quotes_command(&database)
        .args(["get", "--max-width", "2"])
        .output()
        .unwrap();
    assert_eq!(no_width_match.status.code(), Some(1));
    assert!(no_width_match.stdout.is_empty());
    assert!(String::from_utf8_lossy(&no_width_match.stderr).contains("no quotes matched"));

    let reversed = quotes_command(&database)
        .args(["list", "--min-width", "9", "--max-width", "3"])
        .output()
        .unwrap();
    assert_eq!(reversed.status.code(), Some(1));
    assert!(reversed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&reversed.stderr).contains("cannot exceed"));

    let negative = quotes_command(&database)
        .args(["list", "--min-width", "-1"])
        .output()
        .unwrap();
    assert_eq!(negative.status.code(), Some(2));
    assert!(negative.stdout.is_empty());
}

#[test]
fn legacy_and_json_transfers_are_repeatable_and_atomic() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("quotes.db");
    let legacy = directory.path().join("legacy.txt");
    fs::write(
        &legacy,
        "A thought - with an internal separator - Writer\nA thought without attribution\n",
    )
    .unwrap();

    let imported = quotes_command(&database)
        .arg("import")
        .arg(&legacy)
        .output()
        .unwrap();
    assert_success(&imported);
    assert_eq!(
        imported.stdout,
        b"Imported 2 quote(s); skipped 0 duplicate(s).\n"
    );
    let repeated = quotes_command(&database)
        .args(["import", legacy.to_str().unwrap(), "--format", "legacy"])
        .output()
        .unwrap();
    assert_success(&repeated);
    assert_eq!(
        repeated.stdout,
        b"Imported 0 quote(s); skipped 2 duplicate(s).\n"
    );

    let exported = quotes_command(&database)
        .args(["export", "-"])
        .output()
        .unwrap();
    assert_success(&exported);
    assert!(exported.stderr.is_empty());
    let document: Value = serde_json::from_slice(&exported.stdout).unwrap();
    assert_eq!(document["version"], 1);
    assert_eq!(document["quotes"].as_array().unwrap().len(), 2);
    assert_eq!(
        document["quotes"][0]["text"],
        "A thought - with an internal separator"
    );
    assert_eq!(document["quotes"][0]["attribution"], "Writer");

    let first_path = directory.path().join("first.json");
    let second_path = directory.path().join("second.json");
    for path in [&first_path, &second_path] {
        let output = quotes_command(&database)
            .arg("export")
            .arg(path)
            .output()
            .unwrap();
        assert_success(&output);
    }
    assert_eq!(
        fs::read(&first_path).unwrap(),
        fs::read(&second_path).unwrap()
    );

    let malformed = directory.path().join("malformed.json");
    fs::write(
        &malformed,
        r#"{"version":1,"quotes":[{"text":"new"},{"text":"bad\nline"}]}"#,
    )
    .unwrap();
    let failed = quotes_command(&database)
        .arg("import")
        .arg(&malformed)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(failed.stdout.is_empty());

    let listed = quotes_command(&database)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let quotes: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(quotes.as_array().unwrap().len(), 2);
}

#[test]
fn database_path_precedence_is_cli_then_environment_then_default() {
    let directory = tempdir().unwrap();
    let cli_path = directory.path().join("cli.db");
    let environment_path = directory.path().join("environment.db");
    let data_home = directory.path().join("data");
    let home = directory.path().join("home");

    let cli = Command::new(env!("CARGO_BIN_EXE_quotes"))
        .env("QUOTES_DATABASE", &environment_path)
        .env("XDG_DATA_HOME", &data_home)
        .arg("--database")
        .arg(&cli_path)
        .arg("path")
        .output()
        .unwrap();
    assert_success(&cli);
    assert_eq!(
        String::from_utf8_lossy(&cli.stdout).trim(),
        cli_path.to_string_lossy()
    );

    let environment = Command::new(env!("CARGO_BIN_EXE_quotes"))
        .env("QUOTES_DATABASE", &environment_path)
        .env("XDG_DATA_HOME", &data_home)
        .arg("path")
        .output()
        .unwrap();
    assert_success(&environment);
    assert_eq!(
        String::from_utf8_lossy(&environment.stdout).trim(),
        environment_path.to_string_lossy()
    );

    let xdg = Command::new(env!("CARGO_BIN_EXE_quotes"))
        .env_remove("QUOTES_DATABASE")
        .env("XDG_DATA_HOME", &data_home)
        .env("HOME", &home)
        .arg("path")
        .output()
        .unwrap();
    assert_success(&xdg);
    assert_eq!(
        String::from_utf8_lossy(&xdg.stdout).trim(),
        data_home.join("quotes/quotes.db").to_string_lossy()
    );

    let fallback = Command::new(env!("CARGO_BIN_EXE_quotes"))
        .env_remove("QUOTES_DATABASE")
        .env_remove("XDG_DATA_HOME")
        .env("HOME", &home)
        .arg("path")
        .output()
        .unwrap();
    assert_success(&fallback);
    assert_eq!(
        String::from_utf8_lossy(&fallback.stdout).trim(),
        home.join(".local/share/quotes/quotes.db").to_string_lossy()
    );
}
