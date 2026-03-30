mod api;
mod commands;
mod resolver;
mod tui;

use std::io::IsTerminal;

use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser)]
#[command(
    name = "sp-client",
    about = "Check Python packages for supply chain security risks"
)]
struct Cli {
    /// Server URL
    #[arg(
        long,
        global = true,
        env = "SP_CLIENT_SERVER_URL",
        default_value = "http://localhost:8080"
    )]
    server: String,

    /// Output as JSON
    #[arg(long, global = true, default_value_t = false)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check packages from a requirements file
    Check {
        /// Path to requirements.txt
        #[arg(short, long)]
        requirements: String,

        /// Don't wait for analysis to complete — just show current status and exit
        #[arg(long, default_value_t = false)]
        no_wait: bool,

        /// Poll interval in seconds (default: 10)
        #[arg(long, default_value_t = 10)]
        interval: u64,

        /// Also fail (exit 1) for needs_review status
        #[arg(long, default_value_t = false)]
        fail_on_review: bool,

        /// Resolver to use
        #[arg(long)]
        resolver: Option<ResolverChoice>,

        /// Disable interactive TUI (uses log output instead)
        #[arg(long, default_value_t = false)]
        no_tui: bool,
    },

    /// View analysis details for a specific package
    Details {
        /// Package name
        package: String,

        /// Package version
        version: String,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum ResolverChoice {
    Uv,
    Pip,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Check {
            requirements,
            no_wait,
            interval,
            fail_on_review,
            resolver: resolver_choice,
            no_tui,
        } => {
            let watch = !no_wait;
            let interactive = !cli.json && watch && !no_tui && std::io::stdout().is_terminal();

            commands::check::run(commands::check::CheckArgs {
                requirements,
                server: cli.server,
                watch,
                interval,
                json: cli.json,
                fail_on_review,
                resolver: resolver_choice.map(|r| match r {
                    ResolverChoice::Uv => resolver::Resolver::Uv,
                    ResolverChoice::Pip => resolver::Resolver::Pip,
                }),
                interactive,
            })
            .await
        }
        Commands::Details { package, version } => {
            commands::details::run(commands::details::DetailsArgs {
                package,
                version,
                server: cli.server,
                json: cli.json,
            })
            .await
        }
    }
}
