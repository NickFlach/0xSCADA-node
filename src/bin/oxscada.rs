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
use oxscada_node::bridge::{self, ScadaBridge, MAX_RECENT_EVENTS};
use oxscada_node::node::{Node, NodeAction, NodeConfig, NodeEvent};
use oxscada_node::tuner::{AdaptiveTuner, ConsensusMetrics, ParameterAdjustment, TuningConfig};
use oxscada_node::telemetry::{self, TelemetryStatus, TelemetryTuning};

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

/// Registry to track peer connections and routing
#[derive(Debug, Default)]
struct PeerRegistry {
    by_addr: HashMap<SocketAddr, Connection>,
    id_to_addr: HashMap<[u8; 32], SocketAddr>,
}

impl PeerRegistry {
    fn new() -> Self {
        Self::default()
    }

    fn insert(&mut self, addr: SocketAddr, conn: Connection, node_id: Option<[u8; 32]>) {
        self.by_addr.insert(addr, conn);
        if let Some(id) = node_id {
            self.id_to_addr.insert(id, addr);
        }
    }

    fn remove(&mut self, addr: SocketAddr) -> Option<[u8; 32]> {
        self.by_addr.remove(&addr);
        // Find and remove the reverse mapping
        let node_id = self.id_to_addr.iter()
            .find_map(|(id, &a)| if a == addr { Some(*id) } else { None });
        if let Some(id) = node_id {
            self.id_to_addr.remove(&id);
        }
        node_id
    }

    fn get_connection_by_addr(&self, addr: SocketAddr) -> Option<&Connection> {
        self.by_addr.get(&addr)
    }

    fn get_connection_by_id(&self, node_id: [u8; 32]) -> Option<&Connection> {
        self.id_to_addr.get(&node_id)
            .and_then(|&addr| self.by_addr.get(&addr))
    }

    fn register_node_id(&mut self, addr: SocketAddr, node_id: [u8; 32]) {
        if self.by_addr.contains_key(&addr) {
            self.id_to_addr.insert(node_id, addr);
        }
    }

    fn iter(&self) -> impl Iterator<Item = (&SocketAddr, &Connection)> {
        self.by_addr.iter()
    }

    fn len(&self) -> usize {
        self.by_addr.len()
    }
}

type PeerMap = Arc<Mutex<PeerRegistry>>;

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
                match &msg {
                    ConsensusMessage::BlockProposal { .. } => {
                        println!("[oxscada] Broadcasting block proposal");
                    }
                    ConsensusMessage::BlockAttestation { block_hash, .. } => {
                        println!("[oxscada] Broadcasting attestation for block {}", &hex(block_hash)[..16]);
                    }
                    _ => {}
                }
                let data = msg.serialize();
                let conns = peers.lock().await;
                for (addr, conn) in conns.iter() {
                    if let Err(e) = Transport::send(conn, &data).await {
                        eprintln!("[oxscada] Failed to send to {addr}: {e}");
                    }
                }
            }
            NodeAction::SendTo { node_id, msg } => {
                println!("[oxscada] SendTo {}: {:?}", &hex(&node_id)[..16], std::mem::discriminant(&msg));
                let data = msg.serialize();
                let conns = peers.lock().await;
                
                // Route to specific peer by node_id
                if let Some(conn) = conns.get_connection_by_id(node_id) {
                    if let Err(e) = Transport::send(conn, &data).await {
                        eprintln!("[oxscada] Failed to send to node {}: {e}", &hex(&node_id)[..16]);
                    }
                } else {
                    eprintln!("[oxscada] No connection found for node {}", &hex(&node_id)[..16]);
                    // Fallback: broadcast to all peers (for backward compatibility during transition)
                    for (addr, conn) in conns.iter() {
                        if let Err(e) = Transport::send(conn, &data).await {
                            eprintln!("[oxscada] Failed to send to {addr}: {e}");
                        }
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
    let bridge_keypair = NodeKeypair::from_bytes(&secret).expect("Invalid keypair bytes");
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

    // Shared event log
    let event_log = bridge::new_event_log();

    // RPC server
    let tick_counter = Arc::new(AtomicU64::new(0));
    let rpc_ticks = tick_counter.clone();
    let rpc_node = node.clone();
    let rpc_port = config.rpc_port;
    let rpc_events = event_log.clone();
    tokio::spawn(async move {
        if let Err(e) = oxscada_node::rpc::run_rpc_server(rpc_port, rpc_node, rpc_ticks, rpc_events).await {
            eprintln!("[oxscada] RPC server error: {e}");
        }
    });

    // NATS SCADA bridge
    if let Some(nats_url) = &config.nats_url {
        let bridge_node = node.clone();
        let bridge_events = event_log.clone();
        let nats_url = nats_url.clone();
        let bridge_kp = bridge_keypair;
        tokio::spawn(async move {
            println!("[oxscada] Connecting to NATS at {nats_url}...");
            let bridge = match ScadaBridge::connect(&nats_url).await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[oxscada] NATS connection failed: {e}");
                    return;
                }
            };
            println!("[oxscada] NATS connected, subscribing to scada.events");
            let mut sub = match bridge.subscribe("scada.events").await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[oxscada] NATS subscribe failed: {e}");
                    return;
                }
            };
            use futures::StreamExt;
            while let Some(msg) = sub.next().await {
                match bridge::event_to_transaction(&msg.payload) {
                    Ok((body, record)) => {
                        let tx = oxscada_node::transaction::Transaction::new(body, &bridge_kp);
                        let hash_hex = &record.tx_hash;
                        println!(
                            "[oxscada] SCADA event: {} {} → tx {}",
                            record.asset, record.event_type, hash_hex
                        );
                        // Store in event log
                        {
                            let mut log = bridge_events.lock().await;
                            if log.len() >= MAX_RECENT_EVENTS {
                                log.pop_front();
                            }
                            log.push_back(record);
                        }
                        // Feed to node
                        {
                            let mut n = bridge_node.lock().await;
                            n.handle_event(NodeEvent::NewTransaction(tx));
                        }
                    }
                    Err(e) => {
                        eprintln!("[oxscada] Bad SCADA event: {e}");
                    }
                }
            }
            println!("[oxscada] NATS subscription ended");
        });
    }

    // 4. Shared peer connections
    let peers: PeerMap = Arc::new(Mutex::new(PeerRegistry::new()));

    // 5. Spawn tasks
    let shutdown = tokio::signal::ctrl_c();

    // -- Telemetry publisher (optional, graceful degradation) --
    let telemetry_publisher = if let Some(nats_url) = &config.nats_url {
        let prefix = config.telemetry.as_ref()
            .map(|t| t.subject_prefix.as_str())
            .unwrap_or("scada.consensus");
        telemetry::try_connect(nats_url, prefix, &hex(&node_id)).await
    } else {
        None
    };
    let telemetry_publisher = Arc::new(Mutex::new(telemetry_publisher));

    // -- Adaptive tuner --
    let tuner = Arc::new(Mutex::new(AdaptiveTuner::new(
        TuningConfig::default(),
        config.coupling_k,
        config.chiral_eta,
    )));

    // -- Tick loop --
    let tick_node = node.clone();
    let tick_peers = peers.clone();
    let block_time = config.block_time_ms;
    let tick_counter_ref = tick_counter.clone();
    let tick_tuner = tuner.clone();
    let tick_telemetry = telemetry_publisher.clone();
    let tick_node_id_hex = hex(&node_id);
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

                // Feed metrics to adaptive tuner
                let leader_id = n.consensus_leader().unwrap_or([0u8; 32]);
                let metrics = ConsensusMetrics::new(
                    status.order_parameter,
                    status.mean_phase,
                    status.local_phase,
                    leader_id,
                    true, // TODO: track actual attestation success
                    status.connected_peers,
                    status.latest_height,
                    ts,
                );

                let mut t = tick_tuner.lock().await;
                t.record(metrics);

                // Check for parameter adjustments
                if let Some(adjustment) = t.maybe_adjust() {
                    match &adjustment {
                        ParameterAdjustment::CouplingK(k) => {
                            println!("[tuner] Adjusting coupling_k → {k:.4}");
                            n.set_coupling(*k);
                        }
                        ParameterAdjustment::ChiralEta(eta) => {
                            println!("[tuner] Adjusting chiral_eta → {eta:.4}");
                            n.set_chiral_eta(*eta);
                        }
                        ParameterAdjustment::Both { coupling_k, chiral_eta } => {
                            println!("[tuner] Adjusting coupling_k → {coupling_k:.4}, chiral_eta → {chiral_eta:.4}");
                            n.set_coupling(*coupling_k);
                            n.set_chiral_eta(*chiral_eta);
                        }
                    }

                    // Publish tuning event
                    let (ck, ce) = t.current_params();
                    let tuning_msg = TelemetryTuning {
                        node_id: tick_node_id_hex.clone(),
                        coupling_k: ck,
                        chiral_eta: ce,
                        mean_r: status.order_parameter,
                        adjustment: Some(format!("{:?}", adjustment)),
                        timestamp_ms: ts,
                    };
                    let tp = tick_telemetry.lock().await;
                    if let Some(ref pub_) = *tp {
                        let _ = pub_.publish_tuning(&tuning_msg).await;
                    }
                }

                // Publish status telemetry
                if tick_count % 5 == 0 { // every 5 ticks to avoid flooding
                    let (ck, ce) = t.current_params();
                    let tel_status = TelemetryStatus {
                        node_id: tick_node_id_hex.clone(),
                        height: status.latest_height,
                        role: status.role.clone(),
                        order_parameter: status.order_parameter,
                        mean_phase: status.mean_phase,
                        local_phase: status.local_phase,
                        peer_count: status.connected_peers,
                        coupling_k: ck,
                        chiral_eta: ce,
                        timestamp_ms: ts,
                    };
                    let tp = tick_telemetry.lock().await;
                    if let Some(ref pub_) = *tp {
                        let _ = pub_.publish_status(&tel_status).await;
                    }
                }

                println!(
                    "[oxscada] Tick {tick_count}: role={}, r={:.3}, h={}, peers={}",
                    status.role, status.order_parameter, status.latest_height, status.connected_peers
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
    let connector_node_id = node_id;
    let connector_handle = tokio::spawn(async move {
        for addr in seed_peers {
            let transport = connector_transport.clone();
            let peers = connector_peers.clone();
            let node = connector_node.clone();
            let local_node_id = connector_node_id;
            tokio::spawn(async move {
                let mut backoff_ms = 1000u64;
                loop {
                    match transport.connect(addr).await {
                        Ok(conn) => {
                            println!("[oxscada] Connected to peer {addr}");
                            peers.lock().await.insert(addr, conn.clone(), None);
                            
                            // Send handshake PhaseUpdate immediately
                            let handshake = ConsensusMessage::PhaseUpdate {
                                node_id: local_node_id,
                                phase: 0.0,
                                natural_freq: 1.0,
                                timestamp: now_ms(),
                            };
                            if let Err(e) = Transport::send(&conn, &handshake.serialize()).await {
                                eprintln!("[oxscada] Failed to send handshake to {addr}: {e}");
                            }
                            
                            // Spawn reader for this outbound connection
                            let reader_node = node.clone();
                            let reader_peers = peers.clone();
                            spawn_reader(conn, addr, reader_node, reader_peers, local_node_id).await;
                            
                            // Connection lost — remove and notify
                            let disconnected_node_id = peers.lock().await.remove(addr);
                            {
                                let mut n = node.lock().await;
                                n.handle_event(NodeEvent::PeerDisconnected {
                                    node_id: disconnected_node_id.unwrap_or([0u8; 32]),
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
    let accept_node_id = node_id;
    let accept_handle = tokio::spawn(async move {
        loop {
            match accept_transport.accept().await {
                Ok(conn) => {
                    let addr = conn.remote_address();
                    println!("[oxscada] Accepted connection from {addr}");
                    accept_peers.lock().await.insert(addr, conn.clone(), None);
                    
                    // Send handshake PhaseUpdate immediately
                    let handshake = ConsensusMessage::PhaseUpdate {
                        node_id: accept_node_id,
                        phase: 0.0,
                        natural_freq: 1.0,
                        timestamp: now_ms(),
                    };
                    if let Err(e) = Transport::send(&conn, &handshake.serialize()).await {
                        eprintln!("[oxscada] Failed to send handshake to {addr}: {e}");
                    }
                    
                    let reader_node = accept_node.clone();
                    let reader_peers = accept_peers.clone();
                    let local_node_id = accept_node_id;
                    tokio::spawn(async move {
                        spawn_reader(conn, addr, reader_node.clone(), reader_peers.clone(), local_node_id).await;
                        let disconnected_node_id = reader_peers.lock().await.remove(addr);
                        {
                            let mut n = reader_node.lock().await;
                            n.handle_event(NodeEvent::PeerDisconnected {
                                node_id: disconnected_node_id.unwrap_or([0u8; 32]),
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
    local_node_id: [u8; 32],
) {
    let mut peer_node_id: Option<[u8; 32]> = None;
    let mut first_message = true;
    
    loop {
        match Transport::recv(&conn).await {
            Ok(data) => {
                match ConsensusMessage::deserialize(&data) {
                    Ok(msg) => {
                        // Handle handshake: first PhaseUpdate message establishes peer identity
                        if first_message {
                            if let ConsensusMessage::PhaseUpdate { node_id, .. } = &msg {
                                peer_node_id = Some(*node_id);
                                peers.lock().await.register_node_id(addr, *node_id);
                                
                                // Now notify node of the real peer connection
                                {
                                    let mut n = node.lock().await;
                                    n.handle_event(NodeEvent::PeerConnected {
                                        node_id: *node_id,
                                    });
                                }
                                println!("[oxscada] Handshake complete with peer {} ({})", &hex(node_id)[..16], addr);
                            }
                            first_message = false;
                        }
                        
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[test]
    fn test_peer_registry_basic_operations() {
        let mut registry = PeerRegistry::new();
        let addr1 = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 9001));
        let addr2 = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 9002));
        let node_id1 = [42u8; 32];
        let node_id2 = [43u8; 32];

        // Mock connection - we'll use a dummy for testing structure
        // Note: In real test environment, we'd create actual quinn connections
        // For unit testing the registry logic, we focus on the mapping behavior
        
        // Test that we can handle the basic mapping operations without actual connections
        assert_eq!(registry.by_addr.len(), 0);
        assert_eq!(registry.id_to_addr.len(), 0);
        
        // Test id registration mapping
        registry.id_to_addr.insert(node_id1, addr1);
        registry.id_to_addr.insert(node_id2, addr2);
        
        assert_eq!(registry.id_to_addr.len(), 2);
        assert_eq!(registry.id_to_addr.get(&node_id1), Some(&addr1));
        assert_eq!(registry.id_to_addr.get(&node_id2), Some(&addr2));
        
        // Test reverse lookup would work
        let found_id = registry.id_to_addr.iter()
            .find_map(|(id, &a)| if a == addr1 { Some(*id) } else { None });
        assert_eq!(found_id, Some(node_id1));
        
        // Test removal logic
        registry.id_to_addr.remove(&node_id1);
        assert_eq!(registry.id_to_addr.len(), 1);
        assert_eq!(registry.id_to_addr.get(&node_id1), None);
        assert_eq!(registry.id_to_addr.get(&node_id2), Some(&addr2));
    }
}
