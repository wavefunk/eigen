use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "eigen", version, about = "A static site generator with HTMX support")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Enable verbose output (show each page rendered, each data fetch, etc.)
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Build the site into the dist/ directory
    Build {
        /// Path to the project root (default: current directory)
        #[arg(short, long, default_value = ".")]
        project: PathBuf,
    },
    /// Initialize a new Eigen project
    Init {
        /// Name of the project directory to create
        name: String,
    },
    /// Start the development server with live reload
    Dev {
        /// Path to the project root (default: current directory)
        #[arg(short, long, default_value = ".")]
        project: PathBuf,

        /// Port to bind the dev server to
        #[arg(long, default_value_t = 3000)]
        port: u16,

        /// Host address to bind the dev server to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Run SEO, performance, and accessibility audit on the site
    Audit {
        /// Path to the project root (default: current directory)
        #[arg(short, long, default_value = ".")]
        project: PathBuf,

        /// Output format: "markdown" (default) or "json"
        #[arg(short, long, default_value = "markdown")]
        format: String,

        /// Write report files to this path prefix
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Skip building and audit existing dist/ directory
        #[arg(long)]
        no_build: bool,
    },
}
