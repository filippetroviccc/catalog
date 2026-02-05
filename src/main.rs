mod cli;
mod config;
mod db;
mod indexer;
mod output;
mod roots;
mod search;
mod tags;
mod util;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let cli = cli::Cli::parse();

    let paths = config::Paths::resolve(cli.config.as_deref(), cli.db.as_deref())?;

    match cli.command {
        cli::Commands::Init { preset } => {
            config::init(&paths, preset)?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            if let Ok(cfg) = config::load(&paths.config_path) {
                roots::sync_roots(&conn, &cfg, preset.map(|p| p.to_string()))?;
            }
            println!("Initialized catalog.");
        }
        cli::Commands::Roots => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            roots::print_roots(&conn, &cfg)?;
        }
        cli::Commands::Add { paths: add_paths } => {
            let mut cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let added = roots::add_roots(&mut cfg, &add_paths)?;
            config::save(&paths.config_path, &cfg)?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            roots::sync_roots(&conn, &cfg, None)?;
            println!("Added {} root(s).", added);
        }
        cli::Commands::Rm { paths: rm_paths } => {
            let mut cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let removed = roots::remove_roots(&mut cfg, &rm_paths)?;
            config::save(&paths.config_path, &cfg)?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            roots::sync_roots(&conn, &cfg, None)?;
            println!("Removed {} root(s).", removed);
        }
        cli::Commands::Index {
            full,
            one_filesystem,
        } => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            let stats = indexer::run(&conn, &cfg, full, one_filesystem)?;
            println!(
                "Indexed {} files ({} updated, {} deleted, {} skipped).",
                stats.seen, stats.updated, stats.deleted, stats.skipped
            );
        }
        cli::Commands::Search {
            query,
            ext,
            tag,
            after,
            before,
            min_size,
            max_size,
            root,
            json,
        } => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            let results = search::search(
                &conn,
                &cfg,
                &query,
                ext.as_deref(),
                &tag,
                after.as_deref(),
                before.as_deref(),
                min_size,
                max_size,
                root.as_deref(),
            )?;
            let use_json = json || matches!(cfg.output, config::OutputMode::Json);
            output::print_entries(&results, use_json)?;
        }
        cli::Commands::Recent { days, limit, json } => {
            let cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            let results = search::recent(&conn, &cfg, days, limit)?;
            let use_json = json || matches!(cfg.output, config::OutputMode::Json);
            output::print_entries(&results, use_json)?;
        }
        cli::Commands::Tag { command } => {
            let _cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            match command {
                cli::TagCommands::Add { target, tag } => {
                    tags::add_tag(&conn, &target, &tag)?;
                    println!("Tag added.");
                }
                cli::TagCommands::Rm { target, tag } => {
                    tags::remove_tag(&conn, &target, &tag)?;
                    println!("Tag removed.");
                }
            }
        }
        cli::Commands::Tags => {
            let _cfg = config::load(&paths.config_path)
                .with_context(|| "config not found; run `catalog init`")?;
            let conn = db::connect(&paths.db_path)?;
            db::migrate(&conn)?;
            tags::list_tags(&conn)?;
        }
    }

    Ok(())
}
