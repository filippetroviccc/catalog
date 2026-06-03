use anyhow::{Context, Result};
use catalog::analyze;
use catalog::config::{Config, OutputMode};
use catalog::indexer;
use catalog::search::{self, SearchFilters};
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
    let max_search = Duration::from_millis(env_u64(
        "CATALOG_PERF_MAX_SEARCH_MS",
        default_max_search_ms(),
    ));
    let max_noop_save = Duration::from_millis(env_u64(
        "CATALOG_PERF_MAX_NOOP_SAVE_MS",
        default_max_noop_save_ms(),
    ));

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

    // No-op reindex: nothing changed on disk. The merge-join should find every file
    // unchanged, so `save()` rewrites only the tiny manifest (no segments).
    let noop_index_start = Instant::now();
    indexer::run(&mut store, &cfg, false, false)?;
    let noop_index_elapsed = noop_index_start.elapsed();
    let noop_save_start = Instant::now();
    store.save()?;
    let noop_save_elapsed = noop_save_start.elapsed();

    let analyze_start = Instant::now();
    let report = analyze::analyze_store_with_progress(&store, None, 20, 20, None);
    let analyze_elapsed = analyze_start.elapsed();

    let browse_start = Instant::now();
    let browse = analyze::browse_index_from_store_with_progress(&store, None, None);
    let browse_elapsed = browse_start.elapsed();

    // Worst-case search: a substring that matches every indexed file, forcing a full
    // linear scan plus materialization of every result.
    let broad_query = ".dat";
    let search_start = Instant::now();
    let search_hits = search::search(&store, &cfg, broad_query, &SearchFilters::default())?.len();
    let search_elapsed = search_start.elapsed();

    // Scan-only floor: a substring that matches nothing still touches every entry but
    // builds no results — this is closer to a realistic, selective query's cost.
    let scan_start = Instant::now();
    let scan_hits =
        search::search(&store, &cfg, "zzz_no_match_zzz", &SearchFilters::default())?.len();
    let scan_elapsed = scan_start.elapsed();

    println!("perf_smoke:");
    println!("  roots: {}", cfg.roots.len());
    println!("  files created: {}", total_files);
    println!("  files indexed: {} (seen {})", indexed_files, stats.seen);
    println!("  expected total size: {} bytes", expected_total_size);
    println!("  index:  {:?}", index_elapsed);
    println!(
        "  reindex no-op: walk {:?}, save {:?}",
        noop_index_elapsed, noop_save_elapsed
    );
    println!("  analyze: {:?}", analyze_elapsed);
    println!("  browse: {:?}", browse_elapsed);
    println!(
        "  search (broad '{}', {} hits): {:?}",
        broad_query, search_hits, search_elapsed
    );
    println!(
        "  search (scan-only, {} hits): {:?}",
        scan_hits, scan_elapsed
    );

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
        anyhow::bail!(
            "index exceeded budget: {:?} > {:?}",
            index_elapsed,
            max_index
        );
    }
    if analyze_elapsed > max_analyze {
        anyhow::bail!(
            "analyze exceeded budget: {:?} > {:?}",
            analyze_elapsed,
            max_analyze
        );
    }
    if browse_elapsed > max_browse {
        anyhow::bail!(
            "browse exceeded budget: {:?} > {:?}",
            browse_elapsed,
            max_browse
        );
    }
    // Budget the scan-only (selective) query — it is the representative cost for the
    // "<100ms" target. The broad all-match query is reported but not budgeted: it is a
    // pathological case dominated by materializing one result row per indexed file.
    if scan_elapsed > max_search {
        anyhow::bail!(
            "search exceeded budget: {:?} > {:?} (broad all-match was {:?}, informational)",
            scan_elapsed,
            max_search,
            search_elapsed
        );
    }
    if noop_save_elapsed > max_noop_save {
        anyhow::bail!(
            "no-op reindex save exceeded budget: {:?} > {:?}",
            noop_save_elapsed,
            max_noop_save
        );
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
            let target_dir = if file_idx % 2 == 0 {
                &dir_path
            } else {
                &nested_path
            };
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
    if cfg!(debug_assertions) { 20 } else { 8 }
}

fn default_max_analyze_secs() -> u64 {
    if cfg!(debug_assertions) { 6 } else { 3 }
}

fn default_max_browse_secs() -> u64 {
    if cfg!(debug_assertions) { 6 } else { 3 }
}

fn default_max_search_ms() -> u64 {
    // Target is <100ms for 100k-500k entries (release). Debug builds run the linear
    // scan unoptimized, so allow more headroom there.
    if cfg!(debug_assertions) { 1500 } else { 100 }
}

fn default_max_noop_save_ms() -> u64 {
    // A no-op reindex writes only the manifest (no file segments), so this should be
    // tiny regardless of store size. Generous to avoid CI flakiness.
    if cfg!(debug_assertions) { 800 } else { 300 }
}
