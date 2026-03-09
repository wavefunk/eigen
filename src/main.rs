mod assets;
mod build;
mod cli;
mod config;
mod data;
mod dev;
mod discovery;
mod frontmatter;
mod init;
mod plugins;
mod template;

use clap::Parser;
use cli::{Cli, Command};
use eyre::Result;
use std::time::Instant;

fn main() -> Result<()> {
    // Install eyre's panic and error report handlers.
    color_eyre::install().ok();

    let cli = Cli::parse();

    // Set up tracing/logging based on verbosity flags.
    setup_logging(cli.verbose, cli.quiet);

    match cli.command {
        Command::Build { project } => {
            let project = std::fs::canonicalize(&project)?;
            let start = Instant::now();
            tracing::info!("Building site at {}...", project.display());
            build::build(&project)?;
            let elapsed = start.elapsed();
            eprintln!("Built site in {:.1?}", elapsed);
            Ok(())
        }
        Command::Init { name } => {
            tracing::info!("Initializing new project: {name}");
            init::init_project(&name)?;
            eprintln!("✓ Created new Eigen project in '{name}/'");
            eprintln!("  cd {name} && eigen build");
            Ok(())
        }
        Command::Dev { project, port } => {
            let project = std::fs::canonicalize(&project)?;
            tracing::info!("Starting dev server for {} on port {port}...", project.display());

            // Build and run the async dev server on the Tokio runtime.
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                dev::dev_command(&project, port).await
            })?;

            Ok(())
        }
    }
}

/// Configure tracing/logging based on verbosity flags.
fn setup_logging(verbose: bool, quiet: bool) {
    use tracing_subscriber::EnvFilter;

    let filter = if quiet {
        EnvFilter::new("error")
    } else if verbose {
        EnvFilter::new("eigen=debug,info")
    } else {
        EnvFilter::new("eigen=info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}
