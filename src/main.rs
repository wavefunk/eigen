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
            build::build(&project, false)?;
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
        Command::Dev { project, port, host } => {
            let project = std::fs::canonicalize(&project)?;
            tracing::info!("Starting dev server for {} on {host}:{port}...", project.display());

            // Build and run the async dev server on the Tokio runtime.
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                dev::dev_command(&project, port, &host).await
            })?;

            Ok(())
        }
        Command::Audit { project, format, output, no_build } => {
            let project = std::fs::canonicalize(&project)?;
            let start = Instant::now();

            if !no_build {
                tracing::info!("Building site...");
                build::build(&project, false)?;
            }

            let config = config::load_config(&project)?;
            let dist_dir = project.join("dist");

            if !dist_dir.exists() {
                eyre::bail!("dist/ directory not found. Run `eigen build` first or remove --no-build.");
            }

            // Discover rendered pages from dist/.
            let rendered_pages = discover_rendered_pages(&dist_dir)?;

            let report = build::audit::run_audit(&config, &dist_dir, &rendered_pages)?;

            match output {
                Some(path) => {
                    let json = build::audit::output::json::render_json(&report)?;
                    let md = build::audit::output::markdown::render_markdown(&report);
                    std::fs::write(format!("{}.json", path.display()), json)?;
                    std::fs::write(format!("{}.md", path.display()), md)?;
                    eprintln!("Wrote {}.json and {}.md", path.display(), path.display());
                }
                None => {
                    match format.as_str() {
                        "json" => {
                            let json = build::audit::output::json::render_json(&report)?;
                            println!("{}", json);
                        }
                        _ => {
                            let md = build::audit::output::markdown::render_markdown(&report);
                            print!("{}", md);
                        }
                    }
                }
            }

            let elapsed = start.elapsed();
            eprintln!("Audit completed in {:.1?} ({} issues)", elapsed, report.summary.total);
            Ok(())
        }
    }
}

/// Discover rendered pages by walking dist/ for HTML files.
fn discover_rendered_pages(dist_dir: &std::path::Path) -> Result<Vec<build::render::RenderedPage>> {
    let mut pages = Vec::new();
    for entry in walkdir::WalkDir::new(dist_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "html")
                .unwrap_or(false)
        })
    {
        let rel = entry.path().strip_prefix(dist_dir)?;
        let rel_str = rel.to_string_lossy();
        // Skip audit files, fragments, and error pages.
        if rel_str.starts_with("_audit")
            || rel_str.starts_with("_fragments")
            || rel_str.starts_with("_error")
        {
            continue;
        }
        let url_path = format!("/{}", rel_str);
        let is_index = rel_str.ends_with("index.html");
        pages.push(build::render::RenderedPage {
            url_path,
            is_index,
            is_dynamic: false,
            template_path: None,
        });
    }
    Ok(pages)
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
