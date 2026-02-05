use crate::search::SearchEntry;
use anyhow::Result;
use chrono::{Local, TimeZone};

pub fn print_entries(entries: &[SearchEntry], json: bool) -> Result<()> {
    if json {
        let json = serde_json::to_string_pretty(entries)?;
        println!("{}", json);
        return Ok(());
    }

    for e in entries {
        let dt = Local.timestamp_opt(e.mtime, 0).single();
        let mtime = dt
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string());
        println!("{}  {}  {}  {}", e.id, mtime, e.size, e.path);
    }
    Ok(())
}
