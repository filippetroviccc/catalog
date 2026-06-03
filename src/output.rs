use crate::search::SearchEntry;
use crate::util::human_size;
use anyhow::Result;
use chrono::{Local, TimeZone};

pub fn print_entries(entries: &[SearchEntry], json: bool, long: bool) -> Result<()> {
    if json {
        let json = serde_json::to_string_pretty(entries)?;
        println!("{}", json);
        return Ok(());
    }

    for e in entries {
        if long {
            let dt = Local.timestamp_opt(e.mtime, 0).single();
            let mtime = dt
                .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "-".to_string());
            let kind = if e.is_dir {
                "dir"
            } else if e.is_symlink {
                "symlink"
            } else {
                "file"
            };
            let ext = e.ext.as_deref().unwrap_or("-");
            println!(
                "{}  {}  {}  {}  {}  {}  {}  {}",
                e.id, mtime, e.size, kind, ext, e.status, e.root, e.path
            );
        } else {
            let dt = Local.timestamp_opt(e.mtime, 0).single();
            let mtime = dt
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{}  {}  {}",
                e.path,
                human_size(e.size.max(0) as u64),
                mtime
            );
        }
    }
    Ok(())
}
