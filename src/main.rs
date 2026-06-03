use anyhow::{Context, Result};
use catalog::analyze;
use catalog::analyze_tui;
use catalog::cli;
use catalog::config;
use catalog::indexer;
use catalog::output;
use catalog::roots;
use catalog::search;
use catalog::store;
use catalog::util;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use tracing_subscriber::EnvFilter;

/// Read-only commands cannot self-heal a damaged index, so warn the user to rebuild.
/// (Mutating commands like `index`/`analyze` recover automatically by reindexing.)
fn warn_if_degraded(store: &store::Store) {
    if store.degraded {
        eprintln!(
            "warning: the index is incomplete or damaged; results may be partial. \
             Run `catalog index` to rebuild."
        );
    }
}

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let filter = if cli.debug {
        EnvFilter::new("catalog=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let paths = config::Paths::resolve(cli.config.as_deref(), cli.store.as_deref())?;

    match cli.command {
        cli::Commands::Init { preset } => {
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let preset_name = preset.as_ref().map(|p| p.to_string());
            config::init(&paths, preset.clone())?;
            let mut store = store::Store::init(&paths.store_path)?;
            if let Ok(cfg) = config::load(&paths.config_path) {
                roots::sync_roots(&mut store.data, &cfg, preset_name)?;
                store.save()?;
            }
            println!("Initialized catalog.");
        }
        cli::Commands::Roots => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let store = store::Store::load(&paths.store_path)?;
            warn_if_degraded(&store);
            roots::print_roots(&store.data, &cfg)?;
        }
        cli::Commands::Add { paths: add_paths } => {
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let mut cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let outcome = roots::add_roots(&mut cfg, &add_paths)?;
            config::save(&paths.config_path, &cfg)?;
            let mut store = store::Store::load(&paths.store_path)?;
            roots::sync_roots(&mut store.data, &cfg, None)?;
            store.save()?;
            println!("Added {} root(s).", outcome.added);
            if !outcome.skipped.is_empty() {
                eprintln!("Skipped {} unresolved path(s):", outcome.skipped.len());
                for s in &outcome.skipped {
                    eprintln!("  {s}");
                }
                std::process::exit(1);
            }
        }
        cli::Commands::Rm {
            paths: rm_paths,
            yes,
        } => {
            if !yes && !util::confirm("Remove the given root(s) and purge their indexed entries?")?
            {
                println!("Aborted (re-run with --yes to skip confirmation).");
                return Ok(());
            }
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let mut cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let removed = roots::remove_roots(&mut cfg, &rm_paths)?;
            config::save(&paths.config_path, &cfg)?;
            let mut store = store::Store::load(&paths.store_path)?;
            roots::sync_roots(&mut store.data, &cfg, None)?;
            store.save()?;
            println!("Removed {} root(s).", removed);
        }
        cli::Commands::Index {
            full,
            one_filesystem,
        } => {
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let mut store = store::Store::load(&paths.store_path)?;
            let stats = indexer::run(&mut store, &cfg, full, one_filesystem)?;
            store.save()?;
            println!(
                "Indexed {} files ({} updated, {} deleted, {} skipped).",
                stats.seen, stats.updated, stats.deleted, stats.skipped
            );
        }
        cli::Commands::Search {
            query,
            ext,
            after,
            before,
            min_size,
            max_size,
            root,
            json,
            long,
        } => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let store = store::Store::load(&paths.store_path)?;
            warn_if_degraded(&store);
            let filters = search::SearchFilters {
                ext: ext.as_deref(),
                after: after.as_deref(),
                before: before.as_deref(),
                min_size,
                max_size,
                root: root.as_deref(),
            };
            let results = search::search(&store, &cfg, &query, &filters)?;
            let use_json = json || matches!(cfg.output, config::OutputMode::Json);
            output::print_entries(&results, use_json, long)?;
        }
        cli::Commands::Recent {
            days,
            limit,
            json,
            long,
        } => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let store = store::Store::load(&paths.store_path)?;
            warn_if_degraded(&store);
            let results = search::recent(&store, &cfg, days, limit)?;
            let use_json = json || matches!(cfg.output, config::OutputMode::Json);
            output::print_entries(&results, use_json, long)?;
        }
        cli::Commands::Watch {
            interval,
            full,
            one_filesystem,
        } => {
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let mut store = store::Store::load(&paths.store_path)?;
            let interval = interval.unwrap_or(30);
            println!(
                "Watching for changes every {}s. Press Ctrl+C to stop.",
                interval
            );
            loop {
                let stats = indexer::run(&mut store, &cfg, full, one_filesystem)?;
                store.save()?;
                println!(
                    "Indexed {} files ({} updated, {} deleted, {} skipped).",
                    stats.seen, stats.updated, stats.deleted, stats.skipped
                );
                std::thread::sleep(std::time::Duration::from_secs(interval));
            }
        }
        cli::Commands::Export { output } => {
            let store = store::Store::load(&paths.store_path)?;
            warn_if_degraded(&store);
            let json = store.export_json()?;
            match output {
                Some(path) => {
                    let out_path = util::normalize_path_allow_missing(&path)?;
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create output dir: {}", parent.display())
                        })?;
                    }
                    std::fs::write(&out_path, json).with_context(|| {
                        format!("failed to write export: {}", out_path.display())
                    })?;
                    println!("Exported store to {}", out_path.display());
                }
                None => {
                    println!("{}", json);
                }
            }
        }
        cli::Commands::Prune { yes } => {
            if !yes
                && !util::confirm(
                    "This permanently removes all indexed data (config is kept). Continue?",
                )?
            {
                println!("Aborted (re-run with --yes to skip confirmation).");
                return Ok(());
            }
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let removed = store::prune_store(&paths.store_path)?;
            if removed == 0 {
                println!("No store found to remove.");
            } else {
                println!("Pruned {} store file(s).", removed);
            }
        }
        cli::Commands::Analyze {
            path,
            top,
            files,
            json,
            raw,
            tui,
        } => {
            // Analyze re-indexes (and saves) when the index is stale, so it holds the
            // write lock for the duration — including the interactive TUI session.
            let _lock = store::StoreLock::acquire(&paths.store_path)?;
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let mut store = store::Store::load(&paths.store_path)?;
            let filter = match path {
                Some(p) => Some(util::normalize_path_allow_missing(&p)?),
                None => None,
            };
            let stale =
                store::index_is_stale(&store.data, filter.as_deref(), chrono::Duration::days(1));
            let use_tui = tui || (!json && !raw);
            if use_tui {
                let browse_index = if stale {
                    let roots = store
                        .data
                        .roots
                        .iter()
                        .map(|root| std::path::PathBuf::from(&root.path))
                        .collect::<Vec<_>>();
                    let mut builder = analyze::BrowseIndexBuilder::new(filter.clone(), roots);
                    let _stats =
                        indexer::run_with_observer(&mut store, &cfg, false, false, &mut builder)?;
                    store.save()?;
                    builder.finalize()
                } else {
                    let pb = ProgressBar::new_spinner();
                    let style = ProgressStyle::with_template("{spinner:.green} {msg}")
                        .unwrap_or_else(|_| ProgressStyle::default_spinner());
                    pb.set_style(style);
                    pb.set_message("Analyzing existing index...");
                    pb.enable_steady_tick(std::time::Duration::from_millis(120));
                    let mut last_k = 0usize;
                    let mut progress = |processed: usize| {
                        let k = processed / 1000;
                        if k != last_k {
                            last_k = k;
                            pb.set_message(format!("Analyzing {}k files...", k));
                        }
                    };
                    let report = analyze::browse_index_from_store_with_progress(
                        &store,
                        filter.clone(),
                        Some(&mut progress),
                    );
                    pb.finish_and_clear();
                    report
                };

                let start_path = filter.and_then(|p| {
                    if browse_index.has_dir(&p) {
                        Some(p)
                    } else if browse_index.has_file(&p) {
                        p.parent().map(|parent| parent.to_path_buf())
                    } else {
                        None
                    }
                });
                analyze_tui::run_browse_tui(&browse_index, start_path)?;
            } else {
                let report = if stale {
                    let mut analyzer =
                        analyze::Analyzer::new(filter, top.unwrap_or(20), files.unwrap_or(20));
                    let stats =
                        indexer::run_with_observer(&mut store, &cfg, false, false, &mut analyzer)?;
                    store.save()?;
                    let report = analyzer.finalize();
                    if !json {
                        println!(
                            "\nIndexed {} files ({} updated, {} deleted, {} skipped).",
                            stats.seen, stats.updated, stats.deleted, stats.skipped
                        );
                    }
                    report
                } else {
                    let pb = ProgressBar::new_spinner();
                    let style = ProgressStyle::with_template("{spinner:.green} {msg}")
                        .unwrap_or_else(|_| ProgressStyle::default_spinner());
                    pb.set_style(style);
                    pb.set_message("Analyzing existing index...");
                    pb.enable_steady_tick(std::time::Duration::from_millis(120));
                    let mut last_k = 0usize;
                    let mut progress = |processed: usize| {
                        let k = processed / 1000;
                        if k != last_k {
                            last_k = k;
                            pb.set_message(format!("Analyzing {}k files...", k));
                        }
                    };
                    let report = analyze::analyze_store_with_progress(
                        &store,
                        filter,
                        top.unwrap_or(20),
                        files.unwrap_or(20),
                        Some(&mut progress),
                    );
                    pb.finish_and_clear();
                    report
                };
                analyze::print_report(&report, json)?;
            }
        }
    }

    Ok(())
}
