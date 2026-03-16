use clap::{Parser, Subcommand};
use std::path::PathBuf;

use oxscada_node::config::Config;
use oxscada_node::crypto::NodeKeypair;

#[derive(Parser)]
#[command(name = "oxscada", about = "0xSCADA blockchain node")]
struct Cli {
    /// Path to TOML configuration file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the node
    Run,
    /// Generate a new ed25519 keypair
    Keygen {
        /// Output file path (default: from config or ./node.key)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Query node status via RPC
    Status {
        /// RPC endpoint (default: http://127.0.0.1:<rpc_port>)
        #[arg(short, long)]
        endpoint: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let config = Config::load_or_default(cli.config.as_deref())
        .expect("Failed to load configuration");

    match cli.command {
        Command::Run => {
            println!("Starting 0xSCADA node with configuration:");
            println!("{}", toml::to_string_pretty(&config).unwrap());
        }
        Command::Keygen { output } => {
            let path = output
                .or_else(|| config.keypair_path.clone())
                .unwrap_or_else(|| PathBuf::from("node.key"));

            let keypair = NodeKeypair::generate();
            std::fs::write(&path, keypair.to_bytes()).expect("Failed to write keypair");
            println!("Generated ed25519 keypair → {}", path.display());
        }
        Command::Status { endpoint } => {
            let ep = endpoint.unwrap_or_else(|| format!("http://127.0.0.1:{}", config.rpc_port));
            println!("Querying node status at {} ...", ep);
            // TODO: actual RPC call
            println!("(RPC client not yet implemented)");
        }
    }
}
