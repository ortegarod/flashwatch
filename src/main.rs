use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

pub mod rpc;
pub mod stream;
pub mod types;
pub mod monitor;
pub mod analyze;
pub mod format;
pub mod decode;
pub mod serve;
pub mod rules;
pub mod alert;
pub mod store;

#[derive(Parser)]
#[command(
    name = "flashwatch",
    about = "Real-time Base L2 flashblock monitor and analyzer",
    version
)]
struct Cli {
    /// Base node WebSocket URL (must support flashblocks)
    #[arg(
        short,
        long,
        env = "BASE_WS_URL",
        default_value = "wss://mainnet.flashblocks.base.org/ws"
    )]
    url: String,

    /// Base node HTTP RPC URL (for JSON-RPC calls)
    #[arg(
        short = 'r',
        long,
        env = "BASE_RPC_URL",
        default_value = "https://mainnet.base.org"
    )]
    rpc_url: String,

    /// Output format
    #[arg(short, long, default_value = "pretty")]
    format: format::OutputFormat,

    /// Verbosity level
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Stream flashblocks in real-time
    Stream {
        /// Show full transaction details (not just hashes)
        #[arg(long)]
        full_txs: bool,

        /// Maximum number of flashblocks to display (0 = unlimited)
        #[arg(short, long, default_value_t = 0)]
        limit: u64,
    },

    /// Monitor flashblock metrics (rate, gas, tx count, latency)
    Monitor {
        /// Refresh interval in milliseconds
        #[arg(short, long, default_value_t = 1000)]
        interval: u64,
    },

    /// Watch for specific events/logs at flashblock speed
    Logs {
        /// Contract address to filter (hex, 0x-prefixed)
        #[arg(short, long)]
        address: Option<String>,

        /// Event topic0 to filter (hex, 0x-prefixed)
        #[arg(short, long)]
        topic: Option<String>,
    },

    /// Track a transaction from submission to flashblock to canonical block
    Track {
        /// Transaction hash to track (hex, 0x-prefixed)
        tx_hash: String,
    },

    /// Show current Base chain info and flashblock status
    Info,

    /// Watch for transactions matching rules and alert via log/webhook
    Alert {
        /// Path to rules TOML config file
        #[arg(short = 'R', long)]
        rules: String,

        /// Output alerts as JSON lines (for piping)
        #[arg(long)]
        json: bool,
    },

    /// Launch web dashboard with live flashblock visualization
    Serve {
        /// Port for the web server
        #[arg(short, long, default_value_t = 3000)]
        port: u16,

        /// Bind address
        #[arg(long, default_value = "0.0.0.0")]
        bind: String,

        /// Path to alert rules TOML config (enables alerting)
        #[arg(short = 'R', long)]
        rules: Option<String>,

        /// Path to SQLite database for alert storage
        #[arg(long, default_value = "flashwatch.db")]
        db: String,
    },
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    // Set up tracing
    let filter = match cli.verbose {
        0 => "flashwatch=info",
        1 => "flashwatch=debug",
        _ => "flashwatch=trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .init();

    match cli.command {
        Commands::Stream { full_txs, limit } => {
            stream::run(&cli.url, full_txs, limit, &cli.format).await?;
        }
        Commands::Monitor { interval } => {
            monitor::run(&cli.url, interval).await?;
        }
        Commands::Logs { address, topic } => {
            stream::logs(&cli.url, address, topic).await?;
        }
        Commands::Track { tx_hash } => {
            analyze::track(&cli.url, &cli.rpc_url, &tx_hash).await?;
        }
        Commands::Info => {
            rpc::info(&cli.rpc_url).await?;
        }
        Commands::Alert { rules, json } => {
            alert::run(&cli.url, &rules, json).await?;
        }
        Commands::Serve { port, bind, rules, db } => {
            serve::run(&cli.url, &cli.rpc_url, &bind, port, rules.as_deref(), Some(&db)).await?;
        }
    }

    Ok(())
}
