use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use quinn::Connection;
use tokio::sync::Mutex;

use oxscada_node::config::Config;
use oxscada_node::consensus::messages::ConsensusMessage;
use oxscada_node::crypto::NodeKeypair;
use oxscada_node::network::transport::Transport;
use oxscada_node::node::{Node, NodeAction, NodeConfig, NodeEvent};

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

type PeerMap = Arc<Mutex<HashMap<SocketAddr, Connection>>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Process actions returned by the node state machine
async fn process_actions(actions: Vec<NodeAction>, peers: &PeerMap) {
    for action in actions {
        match action {
            NodeAction::Broadcast(msg) => {
                let data = msg.serialize();
                let conns = peers.lock().await;
                for (addr, conn) in conns.iter() {
                    if let Err(e) = Transport::send(conn, &data).await {
                        eprintln!("[oxscada] Failed to send to {addr}: {e}");
                    }
                }
            }
            NodeAction::SendTo { node_id, msg } => {
                let data = msg.serialize();
                let conns = peers.lock().await;
                // Find connection by node_id — for now we don't track node_id→addr mapping,
                // so broadcast to all (TODO: proper routing table)
                let _ = node_id; // suppress warning
                for (addr, conn) in conns.iter() {
                    if let Err(e) = Transport::send(conn, &data).await {
                        eprintln!("[oxscada] Failed to send to {addr}: {e}");
                    }
                }
            }
            NodeAction::BlockCommitted { height, hash } => {
                println!(
                    "[oxscada] Block committed height={height} hash={}",
                    &hex(&hash)[..16]
                );
            }
            NodeAction::None => {}
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let config = Config::load_or_default(cli.config.as_deref())
        .expect("Failed to load configuration");

    match cli.command {
        Command::Run => {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(run_node(config));
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
            println!("(RPC client not yet implemented)");
        }
    }
}

async fn run_node(config: Config) {
    // 1. Load keypair
    let keypair_path = config
        .keypair_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("node.key"));
    let keypair_bytes = std::fs::read(&keypair_path).unwrap_or_else(|_| {
        println!("[oxscada] No keypair at {}, generating new one...", keypair_path.display());
        let kp = NodeKeypair::generate();
        let bytes = kp.to_bytes();
        std::fs::create_dir_all(keypair_path.parent().unwrap_or(&PathBuf::from("."))).ok();
        std::fs::write(&keypair_path, &bytes).expect("Failed to write generated keypair");
        bytes.to_vec()
    });
    let secret: [u8; 32] = keypair_bytes
        .try_into()
        .expect("Keypair file must be exactly 32 bytes");
    let keypair = NodeKeypair::from_bytes(&secret).expect("Invalid keypair bytes");
    let node_id = keypair.public_key().0;
    println!("[oxscada] Node ID: {}", hex(&node_id));

    // 2. Create Node
    let node_config = NodeConfig {
        listen_addr: config.listen_addr.to_string(),
        seed_peers: config.seed_peers.iter().map(|a| a.to_string()).collect(),
        data_dir: config.data_dir.to_string_lossy().to_string(),
        block_time_ms: config.block_time_ms,
        coupling: config.coupling_k,
        chiral_eta: config.chiral_eta,
    };
    let node = Arc::new(Mutex::new(Node::new(
        node_config,
        keypair,
        vec![node_id], // initial validator set: just self
    )));

    // 3. Bind transport
    let transport = Transport::bind(config.listen_addr)
        .await
        .expect("Failed to bind QUIC transport");
    let transport = Arc::new(transport);
    println!("[oxscada] Listening on {}", transport.local_addr());

    // RPC server
    let tick_counter = Arc::new(AtomicU64::new(0));
    let rpc_ticks = tick_counter.clone();
    let rpc_node = node.clone();
    let rpc_port = config.rpc_port;
    tokio::spawn(async move {
        if let Err(e) = oxscada_node::rpc::run_rpc_server(rpc_port, rpc_node, rpc_ticks).await {
            eprintln!("[oxscada] RPC server error: {e}");
        }
    });

    // 4. Shared peer connections
    let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));

    // 5. Spawn tasks
    let shutdown = tokio::signal::ctrl_c();

    // -- Tick loop --
    let tick_node = node.clone();
    let tick_peers = peers.clone();
    let block_time = config.block_time_ms;
    let tick_counter_ref = tick_counter.clone();
    let tick_handle = tokio::spawn(async move {
        let mut tick_count: u64 = 0;
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(block_time));
        loop {
            interval.tick().await;
            tick_count += 1;
            tick_counter_ref.store(tick_count, Ordering::Relaxed);
            let ts = now_ms();
            let actions = {
                let mut n = tick_node.lock().await;
                let actions = n.handle_event(NodeEvent::Tick(ts));
                let status = n.status();
                println!(
                    "[oxscada] Tick {tick_count}: role={}, r={:.3}, peers={}",
                    status.role, status.order_parameter, status.connected_peers
                );
                actions
            };
            process_actions(actions, &tick_peers).await;
        }
    });

    // -- Peer connector (seed peers with backoff) --
    let seed_peers = config.seed_peers.clone();
    let connector_transport = transport.clone();
    let connector_peers = peers.clone();
    let connector_node = node.clone();
    let connector_handle = tokio::spawn(async move {
        for addr in seed_peers {
            let transport = connector_transport.clone();
            let peers = connector_peers.clone();
            let node = connector_node.clone();
            tokio::spawn(async move {
                let mut backoff_ms = 1000u64;
                loop {
                    match transport.connect(addr).await {
                        Ok(conn) => {
                            println!("[oxscada] Connected to peer {addr}");
                            peers.lock().await.insert(addr, conn.clone());
                            // Notify node
                            {
                                let mut n = node.lock().await;
                                // We don't know their node_id yet — use zeroed placeholder
                                n.handle_event(NodeEvent::PeerConnected {
                                    node_id: [0u8; 32],
                                });
                            }
                            // Spawn reader for this outbound connection
                            let reader_node = node.clone();
                            let reader_peers = peers.clone();
                            spawn_reader(conn, addr, reader_node, reader_peers).await;
                            // Connection lost — remove and retry
                            peers.lock().await.remove(&addr);
                            {
                                let mut n = node.lock().await;
                                n.handle_event(NodeEvent::PeerDisconnected {
                                    node_id: [0u8; 32],
                                });
                            }
                            println!("[oxscada] Disconnected from peer {addr}, reconnecting...");
                            backoff_ms = 1000;
                        }
                        Err(e) => {
                            eprintln!("[oxscada] Failed to connect to {addr}: {e} (retry in {backoff_ms}ms)");
                        }
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(30_000);
                }
            });
        }
    });

    // -- Acceptor --
    let accept_transport = transport.clone();
    let accept_peers = peers.clone();
    let accept_node = node.clone();
    let accept_handle = tokio::spawn(async move {
        loop {
            match accept_transport.accept().await {
                Ok(conn) => {
                    let addr = conn.remote_address();
                    println!("[oxscada] Accepted connection from {addr}");
                    accept_peers.lock().await.insert(addr, conn.clone());
                    {
                        let mut n = accept_node.lock().await;
                        n.handle_event(NodeEvent::PeerConnected {
                            node_id: [0u8; 32],
                        });
                    }
                    let reader_node = accept_node.clone();
                    let reader_peers = accept_peers.clone();
                    tokio::spawn(async move {
                        spawn_reader(conn, addr, reader_node.clone(), reader_peers.clone()).await;
                        reader_peers.lock().await.remove(&addr);
                        {
                            let mut n = reader_node.lock().await;
                            n.handle_event(NodeEvent::PeerDisconnected {
                                node_id: [0u8; 32],
                            });
                        }
                        println!("[oxscada] Peer {addr} disconnected");
                    });
                }
                Err(e) => {
                    eprintln!("[oxscada] Accept error: {e}");
                }
            }
        }
    });

    // Wait for Ctrl+C
    println!("[oxscada] Node running. Press Ctrl+C to shutdown.");
    let _ = shutdown.await;
    println!("\n[oxscada] Shutting down...");

    // Cleanup
    tick_handle.abort();
    connector_handle.abort();
    accept_handle.abort();
    transport.shutdown();
    println!("[oxscada] Goodbye.");
}

/// Read messages from a connection until it closes, feeding them to the node
async fn spawn_reader(
    conn: Connection,
    addr: SocketAddr,
    node: Arc<Mutex<Node>>,
    peers: PeerMap,
) {
    loop {
        match Transport::recv(&conn).await {
            Ok(data) => {
                match ConsensusMessage::deserialize(&data) {
                    Ok(msg) => {
                        let actions = {
                            let mut n = node.lock().await;
                            n.handle_event(NodeEvent::ConsensusMsg(msg))
                        };
                        process_actions(actions, &peers).await;
                    }
                    Err(e) => {
                        eprintln!("[oxscada] Bad message from {addr}: {e}");
                    }
                }
            }
            Err(_) => {
                // Connection closed
                break;
            }
        }
    }
}
