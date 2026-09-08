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
    after_help = "Start with your Windows desktop session:\n  onenote doctor\n  onenote notebooks list\n  onenote sections list --notebook ID\n  onenote pages list --section ID\n  onenote search 'quarterly review'\n  onenote pages read ID\n\nFor automation:\n  onenote notebooks list -o json\n  onenote schema\n\nRequires Windows OneNote desktop; WSL can use Windows PowerShell interop.\nNo app registration or separate sign-in. All commands are read-only.")]
pub struct Cli {
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
    /// List pages and read their content
    Pages {
        #[command(subcommand)]
        command: Pages,
    },
    /// Search page content using OneNote's search engine
    Search {
        #[arg(value_parser = query)]
        query: String,
        /// Limit search to a notebook, section group, or section ID
        #[arg(long, value_parser = id)]
        scope: Option<String>,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Check Windows PowerShell and the OneNote COM connection
    Doctor {
        /// Check the local platform without launching OneNote
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
