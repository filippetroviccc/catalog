use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "catalog", version, about = "Local index + search CLI")]
pub struct Cli {
    /// Override config path
    #[arg(long)]
    pub config: Option<String>,
    /// Override store path (binary)
    #[arg(long)]
    pub store: Option<String>,
    /// Enable debug logging
    #[arg(long, global = true)]
    pub debug: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize config and database
    Init {
        #[arg(long)]
        preset: Option<Preset>,
    },
    /// Show configured roots
    Roots,
    /// Add roots
    Add {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Remove roots
    Rm {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Index configured roots
    Index {
        #[arg(long)]
        full: bool,
        #[arg(long)]
        one_filesystem: bool,
    },
    /// Search indexed files
    Search {
        query: String,
        #[arg(long)]
        ext: Option<String>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        before: Option<String>,
        #[arg(long)]
        min_size: Option<u64>,
        #[arg(long)]
        max_size: Option<u64>,
        #[arg(long)]
        root: Option<String>,
        #[arg(long)]
        json: bool,
        /// Show more metadata
        #[arg(long, alias = "details")]
        long: bool,
    },
    /// List recently modified files
    Recent {
        #[arg(long)]
        days: Option<u32>,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        json: bool,
        /// Show more metadata
        #[arg(long, alias = "details")]
        long: bool,
    },
    /// Watch for changes (polling)
    Watch {
        /// Poll interval in seconds
        #[arg(long)]
        interval: Option<u64>,
        /// Force full rescan each interval
        #[arg(long)]
        full: bool,
        /// Override one-filesystem for this run
        #[arg(long)]
        one_filesystem: bool,
    },
    /// Export store as JSON
    Export {
        /// Write JSON to a file instead of stdout
        #[arg(long)]
        output: Option<String>,
    },
    /// Remove all stored index state
    Prune,
    /// Analyze disk usage
    Analyze {
        /// Path to analyze (defaults to configured roots)
        path: Option<String>,
        /// Top N folders to show
        #[arg(long)]
        top: Option<usize>,
        /// Top N files to show
        #[arg(long)]
        files: Option<usize>,
        /// JSON output
        #[arg(long)]
        json: bool,
        /// Raw text report output
        #[arg(long, conflicts_with_all = ["json", "tui"])]
        raw: bool,
        /// Interactive TUI browser (default)
        #[arg(long, conflicts_with_all = ["json", "raw"])]
        tui: bool,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Preset {
    #[value(name = "macos-user-additions")]
    MacosUserAdditions,
    #[value(name = "macos-deep")]
    MacosDeep,
    #[value(name = "macos-full")]
    MacosFull,
}

impl Preset {
    pub fn to_string(&self) -> String {
        match self {
            Preset::MacosUserAdditions => "macos-user-additions".to_string(),
            Preset::MacosDeep => "macos-deep".to_string(),
            Preset::MacosFull => "macos-full".to_string(),
        }
    }
}
