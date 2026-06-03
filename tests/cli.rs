//! End-to-end tests driving the built `catalog` binary.
//!
//! Config/store paths are redirected into a tempdir via `CATALOG_CONFIG` /
//! `CATALOG_STORE`, and the config is seeded through the library so `init` never
//! falls back to the `macos-full` preset (which would index `/`).

use assert_cmd::Command;
use catalog::config::{Config, OutputMode};
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    config_path: PathBuf,
    store_path: PathBuf,
}

impl Fixture {
    /// Tempdir with a seeded config pointing at a small data dir (two files).
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        let store_path = tmp.path().join("catalog.bin");
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("alpha.txt"), "a").unwrap();
        fs::write(data_dir.join("beta.rs"), "b").unwrap();
        let data_canon = fs::canonicalize(&data_dir).unwrap();

        let cfg = Config {
            version: 1,
            output: OutputMode::Plain,
            include_hidden: false,
            one_filesystem: true,
            roots: vec![data_canon.to_string_lossy().to_string()],
            excludes: vec![],
        };
        catalog::config::save(&config_path, &cfg).unwrap();

        Fixture {
            _tmp: tmp,
            config_path,
            store_path,
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("catalog").unwrap();
        c.env("CATALOG_CONFIG", &self.config_path)
            .env("CATALOG_STORE", &self.store_path);
        c
    }
}

#[test]
fn full_lifecycle_index_search_recent_prune() {
    let fx = Fixture::new();

    // init is a no-op here (config already exists) but must still succeed.
    fx.cmd().arg("init").assert().success();

    fx.cmd()
        .arg("index")
        .assert()
        .success()
        .stdout(predicate::str::contains("Indexed"));

    fx.cmd()
        .args(["search", "alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha.txt"));

    // extension filter narrows to beta.rs and excludes alpha.txt
    fx.cmd()
        .args(["search", "beta", "--ext", "rs"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("beta.rs").and(predicate::str::contains("alpha.txt").not()),
        );

    fx.cmd()
        .args(["recent", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"path\""));

    // prune without --yes on a non-TTY declines and leaves the store in place.
    fx.cmd()
        .arg("prune")
        .assert()
        .success()
        .stdout(predicate::str::contains("Aborted"));
    assert!(fx.store_path.exists(), "store kept when prune declined");

    fx.cmd()
        .args(["prune", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pruned"));
    assert!(!fx.store_path.exists(), "store removed after prune --yes");
}

#[test]
fn missing_subcommand_is_usage_error() {
    // clap exits 2 on usage errors.
    Command::cargo_bin("catalog")
        .unwrap()
        .assert()
        .failure()
        .code(2);
}

#[test]
fn search_without_config_fails() {
    let tmp = tempfile::tempdir().unwrap();
    Command::cargo_bin("catalog")
        .unwrap()
        .env("CATALOG_CONFIG", tmp.path().join("missing.toml"))
        .env("CATALOG_STORE", tmp.path().join("catalog.bin"))
        .args(["search", "anything"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn add_missing_path_exits_nonzero() {
    let fx = Fixture::new();
    let missing = Path::new("/definitely/not/here/xyz123");
    fx.cmd()
        .args(["add", missing.to_str().unwrap()])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Skipped"));
}
