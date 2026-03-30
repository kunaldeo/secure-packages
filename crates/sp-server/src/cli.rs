use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "secure-packages",
    about = "Supply chain security scanner for packages"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the HTTP server and background workers
    Serve,

    /// Start background workers only (no HTTP server)
    Worker,

    /// Run database migrations
    Migrate,

    /// Manually analyze a single package version
    Analyze {
        /// Package name
        #[arg(long)]
        package: String,

        /// Package version
        #[arg(long)]
        version: String,
    },
}
