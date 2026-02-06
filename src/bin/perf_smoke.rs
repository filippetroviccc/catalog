use anyhow::{Context, Result};
use catalog::analyze;
use catalog::config::{Config, OutputMode};
use catalog::indexer;
use catalog::store::Store;
use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() -> Result<()> {
    let dirs = env_usize("CATALOG_PERF_DIRS", 40);
    let files_per_dir = env_usize("CATALOG_PERF_FILES_PER_DIR", 200);
    let file_size = env_u64("CATALOG_PERF_FILE_SIZE", 16 * 1024);

    let max_index = env_duration("CATALOG_PERF_MAX_INDEX_SECS", default_max_index_secs());
    let max_analyze = env_duration("CATALOG_PERF_MAX_ANALYZE_SECS", default_max_analyze_secs());
    let max_browse = env_duration("CATALOG_PERF_MAX_BROWSE_SECS", default_max_browse_secs());

    let base = temp_dir("perf_smoke");
    let root = base.join("root");
    fs::create_dir_all(&root)?;

    let total_files = populate_tree(&root, dirs, files_per_dir, file_size)?;
    let expected_total_size = total_files as u64 * file_size;

    let cfg = Config {
        version: 1,
        output: OutputMode::Plain,
        include_hidden: true,
        one_filesystem: true,
        roots: vec![root.to_string_lossy().to_string()],
        excludes: Vec::new(),
    };

    let store_path = base.join("store.bin");
    let mut store = Store::init(&store_path)?;

    let start = Instant::now();
    let stats = indexer::run(&mut store, &cfg, false, false)?;
    store.save()?;
    let index_elapsed = start.elapsed();

    let indexed_files = store
        .data
        .files
        .iter()
        .filter(|f| !f.is_dir && f.status == "active")
        .count();

    let analyze_start = Instant::now();
    let report = analyze::analyze_store_with_progress(&store, None, 20, 20, None);
    let analyze_elapsed = analyze_start.elapsed();

    let browse_start = Instant::now();
    let browse = analyze::browse_index_from_store_with_progress(&store, None, None);
    let browse_elapsed = browse_start.elapsed();

    println!("perf_smoke:");
    println!("  roots: {}", cfg.roots.len());
    println!("  files created: {}", total_files);
    println!("  files indexed: {} (seen {})", indexed_files, stats.seen);
    println!("  expected total size: {} bytes", expected_total_size);
    println!("  index:  {:?}", index_elapsed);
    println!("  analyze: {:?}", analyze_elapsed);
    println!("  browse: {:?}", browse_elapsed);

    if indexed_files != total_files {
        anyhow::bail!(
            "indexed file count mismatch: expected {}, got {}",
            total_files,
            indexed_files
        );
    }

    if report.total_scanned != expected_total_size {
        anyhow::bail!(
            "analyze total mismatch: expected {} bytes, got {} bytes",
            expected_total_size,
            report.total_scanned
        );
    }

    if browse.total_scanned != expected_total_size {
        anyhow::bail!(
            "browse total mismatch: expected {} bytes, got {} bytes",
            expected_total_size,
            browse.total_scanned
        );
    }

    if index_elapsed > max_index {
        anyhow::bail!("index exceeded budget: {:?} > {:?}", index_elapsed, max_index);
    }
    if analyze_elapsed > max_analyze {
        anyhow::bail!(
            "analyze exceeded budget: {:?} > {:?}",
            analyze_elapsed,
            max_analyze
        );
    }
    if browse_elapsed > max_browse {
        anyhow::bail!("browse exceeded budget: {:?} > {:?}", browse_elapsed, max_browse);
    }

    if env::var("CATALOG_PERF_KEEP").is_err() {
        let _ = fs::remove_dir_all(&base);
    } else {
        println!("  kept temp dir: {}", base.display());
    }

    Ok(())
}

fn populate_tree(root: &Path, dirs: usize, files_per_dir: usize, file_size: u64) -> Result<usize> {
    let mut total_files = 0;
    for dir_idx in 0..dirs {
        let dir_path = root.join(format!("dir_{:03}", dir_idx));
        let nested_path = dir_path.join("nested");
        fs::create_dir_all(&nested_path)
            .with_context(|| format!("failed to create dir: {}", nested_path.display()))?;

        for file_idx in 0..files_per_dir {
            let target_dir = if file_idx % 2 == 0 { &dir_path } else { &nested_path };
            let file_path = target_dir.join(format!("file_{:04}.dat", file_idx));
            let file = File::create(&file_path)
                .with_context(|| format!("failed to create file: {}", file_path.display()))?;
            file.set_len(file_size)
                .with_context(|| format!("failed to set size for: {}", file_path.display()))?;
            total_files += 1;
        }
    }
    Ok(total_files)
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_nanos(0))
        .as_nanos();
    std::env::temp_dir().join(format!(
        "catalog_{}_{}_{}",
        prefix,
        std::process::id(),
        nanos
    ))
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_duration(name: &str, default_secs: u64) -> Duration {
    Duration::from_secs(env_u64(name, default_secs))
}

fn default_max_index_secs() -> u64 {
    if cfg!(debug_assertions) {
        20
    } else {
        8
    }
}

fn default_max_analyze_secs() -> u64 {
    if cfg!(debug_assertions) {
        6
    } else {
        3
    }
}

fn default_max_browse_secs() -> u64 {
    if cfg!(debug_assertions) {
        6
    } else {
        3
    }
}
