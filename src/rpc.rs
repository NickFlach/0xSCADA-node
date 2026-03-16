use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::bridge::EventLog;
use crate::node::Node;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn json_response(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, OPTIONS")
        .header("Access-Control-Allow-Headers", "*")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    node: Arc<Mutex<Node>>,
    ticks: Arc<AtomicU64>,
    events: EventLog,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    // Handle CORS preflight
    if req.method() == hyper::Method::OPTIONS {
        return Ok(json_response(StatusCode::NO_CONTENT, String::new()));
    }

    let resp = match req.uri().path() {
        "/status" => {
            let status = node.lock().await.status();
            let uptime = ticks.load(Ordering::Relaxed);
            let peer_phases: Vec<serde_json::Value> = status.peer_phases.iter().map(|p| {
                serde_json::json!({
                    "node_id": hex(&p.node_id),
                    "phase": p.phase,
                    "natural_freq": p.natural_freq,
                    "last_updated": p.last_updated,
                })
            }).collect();
            let body = serde_json::json!({
                "node_id": hex(&status.node_id),
                "height": status.latest_height,
                "role": status.role,
                "order_parameter": status.order_parameter,
                "mean_phase": status.mean_phase,
                "local_phase": status.local_phase,
                "peer_phases": peer_phases,
                "peers": status.connected_peers,
                "mempool": status.mempool_size,
                "uptime_ticks": uptime,
            });
            json_response(StatusCode::OK, body.to_string())
        }
        "/events" => {
            let log = events.lock().await;
            let recent: Vec<_> = log.iter().rev().take(50).cloned().collect();
            let body = serde_json::to_string(&recent).unwrap();
            json_response(StatusCode::OK, body)
        }
        "/health" => {
            json_response(StatusCode::OK, r#"{"ok":true}"#.to_string())
        }
        _ => {
            json_response(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#.to_string())
        }
    };
    Ok(resp)
}

pub async fn run_rpc_server(
    port: u16,
    node: Arc<Mutex<Node>>,
    ticks: Arc<AtomicU64>,
    events: EventLog,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    println!("[oxscada] RPC server listening on http://{addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        let node = node.clone();
        let ticks = ticks.clone();
        let events = events.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req| {
                let node = node.clone();
                let ticks = ticks.clone();
                let events = events.clone();
                async move { handle_request(req, node, ticks, events).await }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                eprintln!("[oxscada] RPC connection error: {e}");
            }
        });
    }
}
