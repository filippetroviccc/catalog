use crate::search::SearchEntry;
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
            println!("{}  {}  {}", e.path, human_size(e.size), mtime);
        }
    }
    Ok(())
}

fn human_size(bytes: i64) -> String {
    let size = if bytes < 0 { 0.0 } else { bytes as f64 };
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size;
    let mut idx = 0;
    while value >= 1024.0 && idx < units.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{}{}", bytes.max(0), units[idx])
    } else {
        format!("{:.1}{}", value, units[idx])
    }
}
