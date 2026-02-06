mod cli;
mod config;
mod indexer;
mod output;
mod roots;
mod search;
mod store;
mod util;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

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
            roots::print_roots(&store.data, &cfg)?;
        }
        cli::Commands::Add { paths: add_paths } => {
            let mut cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let added = roots::add_roots(&mut cfg, &add_paths)?;
            config::save(&paths.config_path, &cfg)?;
            let mut store = store::Store::load(&paths.store_path)?;
            roots::sync_roots(&mut store.data, &cfg, None)?;
            store.save()?;
            println!("Added {} root(s).", added);
        }
        cli::Commands::Rm { paths: rm_paths } => {
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
            let results = search::search(
                &store,
                &cfg,
                &query,
                ext.as_deref(),
                after.as_deref(),
                before.as_deref(),
                min_size,
                max_size,
                root.as_deref(),
            )?;
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
            let results = search::recent(&store, &cfg, days, limit)?;
            let use_json = json || matches!(cfg.output, config::OutputMode::Json);
            output::print_entries(&results, use_json, long)?;
        }
        cli::Commands::Watch {
            interval,
            full,
            one_filesystem,
        } => {
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
        cli::Commands::Prune => {
            let removed = store::prune_store(&paths.store_path)?;
            if removed == 0 {
                println!("No store found to remove.");
            } else {
                println!("Pruned {} store file(s).", removed);
            }
        }
    }

    Ok(())
}
