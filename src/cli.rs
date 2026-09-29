use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Auto,
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "onenote", version, about = "Your OneNote notebooks, from the terminal",
    styles = styles(),
    after_help = "Get started:\n  onenote init\n  onenote init --profile kiosk --backend ssh --host kiosk\n  onenote tui\n  onenote profile list\n  onenote doctor\n  onenote notebooks list\n  onenote sections list --notebook ID\n  onenote pages list --section ID\n  onenote search 'quarterly review'\n  onenote pages read ID\n  onenote pages create --section ID --title Notes --file notes.txt --dry-run\n  onenote pages append ID --text 'Follow up tomorrow'\n\nFor automation:\n  onenote notebooks list -o json\n  onenote schema\n\nRequires Windows OneNote desktop. Connect locally on Windows/WSL or over SSH from any supported platform.\nNo app registration or separate sign-in. Writes use explicit pages create/append commands; preview with --dry-run.")]
pub struct Cli {
    /// Use a saved connection profile (also ONENOTE_PROFILE)
    #[arg(long, global = true, env = "ONENOTE_PROFILE", value_parser = crate::config::profile_name)]
    pub profile: Option<String>,
    /// Auto: text in a terminal, JSON when piped
    #[arg(short = 'o', long, global = true, value_enum, default_value = "auto")]
    pub output: Format,
    /// Disable color (also respects NO_COLOR)
    #[arg(long, global = true)]
    pub no_color: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Explore notebooks, sections, and pages in an interactive terminal
    Tui,
    /// Configure and check a local desktop or SSH connection
    Init(InitArgs),
    /// List, select, or remove saved connection profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Inspect connection settings and their storage location
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List notebooks open in the desktop application
    Notebooks {
        #[command(subcommand)]
        command: Notebooks,
    },
    /// Explore sections, including those inside section groups
    Sections {
        #[command(subcommand)]
        command: Sections,
    },
    /// Read, create, and append to pages
    Pages {
        #[command(subcommand)]
        command: Pages,
    },
    /// Search pages with OneNote's index or a literal text scan
    Search {
        #[arg(value_parser = query)]
        query: String,
        /// Limit search to a notebook, section group, or section ID
        #[arg(long, value_parser = id)]
        scope: Option<String>,
        /// Read page titles and text directly; case-insensitive literal matching
        #[arg(long)]
        scan: bool,
        /// Maximum pages to attempt per scan (default 100, maximum 500)
        #[arg(long, requires = "scan", value_parser = clap::value_parser!(u16).range(1..=500))]
        scan_limit: Option<u16>,
        /// Continue from next_scan_offset; independent of result --offset
        #[arg(long, requires = "scan", value_parser = clap::value_parser!(u32).range(0..=1_000_000))]
        scan_offset: Option<u32>,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Check the selected connection and OneNote access
    Doctor {
        /// Check configuration and local transport availability without connecting
        #[arg(long)]
        offline: bool,
    },
    /// Describe supported capabilities and requirements
    Capabilities,
    /// Emit the offline CLI Spec v0.3 contract
    Schema {
        #[arg(long)]
        command: Option<String>,
    },
    /// Generate shell completions
    Completions { shell: clap_complete::Shell },
}
#[derive(Debug, Subcommand)]
pub enum Notebooks {
    List(PageArgs),
}
#[derive(Debug, Subcommand)]
pub enum Sections {
    List {
        /// Notebook ID returned by notebooks list
        #[arg(long, value_parser = id)]
        notebook: String,
        #[command(flatten)]
        page: PageArgs,
    },
}
#[derive(Debug, Subcommand)]
pub enum Pages {
    List {
        /// Section ID returned by sections list
        #[arg(long, value_parser = id)]
        section: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Create a page with a title and plain-text content
    Create {
        #[arg(long, value_parser = id)]
        section: String,
        #[arg(long)]
        title: String,
        #[command(flatten)]
        content: ContentArgs,
    },
    /// Add a new text block below existing page content
    Append {
        #[arg(value_parser = id)]
        id: String,
        #[command(flatten)]
        content: ContentArgs,
    },
    Read {
        /// Page ID returned by pages list or search
        #[arg(value_parser = id)]
        id: String,
        /// Include original page XML in the result (without binary payloads)
        #[arg(long)]
        xml: bool,
    },
}
#[derive(Debug, Args)]
#[group(skip)]
pub struct ContentArgs {
    /// Plain text (not Markdown or HTML)
    #[arg(long, required_unless_present = "file", conflicts_with = "file")]
    pub text: Option<String>,
    /// Read UTF-8 text from a file; - reads piped stdin (maximum 1 MiB)
    #[arg(long)]
    pub file: Option<std::path::PathBuf>,
    /// Preview the request without connecting to OneNote or writing anything
    #[arg(long)]
    pub dry_run: bool,
}
#[derive(Debug, Args)]
pub struct PageArgs {
    /// Maximum records to return
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,
    /// Position from next_offset in the previous result
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=1_000_000))]
    pub offset: u32,
    /// Comma-separated fields to include in each record
    #[arg(long, value_delimiter = ',')]
    pub fields: Vec<String>,
}
pub fn id(value: &str) -> Result<String, String> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        Err("use a nonempty OneNote desktop ID returned by this CLI (maximum 4096 bytes)".into())
    } else {
        Ok(value.into())
    }
}
fn query(value: &str) -> Result<String, String> {
    if value.trim().is_empty() || value.len() > 8192 || value.contains('\0') {
        Err("search must contain 1–8192 bytes of text".into())
    } else {
        Ok(value.into())
    }
}
fn styles() -> clap::builder::Styles {
    use clap::builder::styling::AnsiColor;
    clap::builder::Styles::styled()
        .header(AnsiColor::Magenta.on_default().bold())
        .literal(AnsiColor::Magenta.on_default())
        .usage(AnsiColor::Magenta.on_default().bold())
        .placeholder(AnsiColor::Cyan.on_default())
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Connect to this desktop or a Windows machine over SSH
    #[arg(long, value_enum, default_value = "desktop")]
    pub backend: crate::config::Backend,
    /// SSH destination or host alias, optionally user@host
    #[arg(long)]
    pub host: Option<String>,
    /// Private-key file on this machine; otherwise use the SSH agent/config
    #[arg(long)]
    pub identity_file: Option<std::path::PathBuf>,
    /// SSH port; otherwise use the SSH configuration
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: Option<u16>,
    /// Refuse page writes through this profile
    #[arg(long)]
    pub read_only: bool,
    /// Save without checking the desktop connection
    #[arg(long)]
    pub no_check: bool,
    /// Replace an existing profile with these settings
    #[arg(long)]
    pub force: bool,
}
#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// List saved connections and mark the active profile
    List,
    /// Select the profile used by subsequent commands
    Use {
        #[arg(value_parser = crate::config::profile_name)]
        name: String,
    },
    /// Remove saved settings; leaves SSH keys and notebooks untouched
    Remove {
        #[arg(value_parser = crate::config::profile_name)]
        name: String,
    },
}
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show the selected connection settings, without connecting
    Show,
    /// Print the configuration file location
    Path,
}
