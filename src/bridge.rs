use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::transaction::{Transaction, TransactionBody};

/// Max events kept in the recent events buffer.
pub const MAX_RECENT_EVENTS: usize = 100;

/// A SCADA event as received from NATS.
///
/// Canonical wire schema (issue 0xSCADA#440): snake_case, defined in
/// `0xSCADA/shared/wire-schema/scada-event.ts` and pinned by twin fixtures
/// on both sides (`test_canonical_wire_fixture` here). The `alias` /
/// `default` attributes are a TRANSITIONAL net for legacy camelCase
/// publishers (`AnchorableEvent`) — remove once every publisher goes
/// through the shared schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScadaEvent {
    #[serde(default)]
    pub asset: String,
    #[serde(alias = "eventType")]
    pub event_type: String,
    #[serde(alias = "siteId")]
    pub site_id: String,
    pub timestamp: String,
    #[serde(default, alias = "data")]
    pub payload: serde_json::Value,
}

/// A SCADA event record stored for the /events endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScadaEventRecord {
    pub asset: String,
    pub event_type: String,
    pub site_id: String,
    pub timestamp: String,
    pub tx_hash: String,
    pub block_height: Option<u64>,
}

/// Shared recent events buffer.
pub type EventLog = Arc<Mutex<VecDeque<ScadaEventRecord>>>;

pub fn new_event_log() -> EventLog {
    Arc::new(Mutex::new(VecDeque::with_capacity(MAX_RECENT_EVENTS)))
}

/// The NATS SCADA bridge.
pub struct ScadaBridge {
    client: async_nats::Client,
}

impl ScadaBridge {
    /// Connect to a NATS server.
    pub async fn connect(nats_url: &str) -> Result<Self, async_nats::ConnectError> {
        let client = async_nats::connect(nats_url).await?;
        Ok(Self { client })
    }

    /// Subscribe to a subject and return a subscriber stream.
    pub async fn subscribe(
        &self,
        subject: &str,
    ) -> Result<async_nats::Subscriber, async_nats::SubscribeError> {
        self.client.subscribe(subject.to_string()).await
    }
}

/// Parse a SCADA event from JSON bytes and create an unsigned AnchorBatch transaction.
/// Returns (Transaction-like fields, ScadaEventRecord) or an error.
pub fn event_to_transaction(data: &[u8]) -> Result<(TransactionBody, ScadaEventRecord), String> {
    let event: ScadaEvent =
        serde_json::from_slice(data).map_err(|e| format!("JSON parse error: {e}"))?;

    // Hash the entire event payload with blake3 to get merkle_root
    let payload_bytes = serde_json::to_vec(&event.payload).unwrap_or_default();
    let merkle_root: [u8; 32] = *blake3::hash(&payload_bytes).as_bytes();

    let metadata = serde_json::to_vec(&serde_json::json!({
        "asset": event.asset,
        "event_type": event.event_type,
        "timestamp": event.timestamp,
    }))
    .unwrap_or_default();

    let body = TransactionBody::AnchorBatch {
        merkle_root,
        event_count: 1,
        site_id: event.site_id.clone(),
        metadata,
    };

    let tx_hash = body.compute_hash();
    let tx_hash_hex = tx_hash.iter().map(|b| format!("{b:02x}")).collect::<String>();

    let record = ScadaEventRecord {
        asset: event.asset,
        event_type: event.event_type,
        site_id: event.site_id,
        timestamp: event.timestamp,
        tx_hash: tx_hash_hex,
        block_height: None,
    };

    Ok((body, record))
}

/// Create a full (unsigned) Transaction from the body — zeroed signature/signer.
pub fn unsigned_transaction(body: TransactionBody) -> Transaction {
    let hash = body.compute_hash();
    Transaction {
        hash,
        body,
        signature: [0u8; 64],
        signer: [0u8; 32],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_to_transaction() {
        let json = r#"{
            "asset": "TR-MAIN-01",
            "event_type": "BREAKER_TRIP",
            "site_id": "site-1",
            "timestamp": "2026-03-15T22:00:00Z",
            "payload": {"voltage": 138.5}
        }"#;

        let (body, record) = event_to_transaction(json.as_bytes()).unwrap();
        assert!(matches!(body, TransactionBody::AnchorBatch { event_count: 1, .. }));
        assert_eq!(record.asset, "TR-MAIN-01");
        assert_eq!(record.event_type, "BREAKER_TRIP");
        assert_eq!(record.site_id, "site-1");
        assert!(!record.tx_hash.is_empty());
        assert_eq!(record.block_height, None);

        // Verify the transaction is valid
        let tx = unsigned_transaction(body);
        assert!(tx.verify_hash());
        assert!(tx.validate_size().is_ok());
    }

    #[test]
    fn test_bad_json() {
        assert!(event_to_transaction(b"not json").is_err());
    }

    /// Twin contract fixture (issue 0xSCADA#440): byte-identical to the JSON
    /// asserted by `0xSCADA/shared/wire-schema/__tests__/scada-event.test.ts`.
    /// If you change it here, change it there in the same commit.
    #[test]
    fn test_canonical_wire_fixture() {
        let fixture = r#"{"asset":"TR-MAIN-01","event_type":"BREAKER_TRIP","site_id":"site-1","timestamp":"2026-03-15T22:00:00Z","payload":{"voltage":138.5},"site_name":"Substation Alpha","asset_type":"BREAKER","details":"Breaker tripped on overcurrent"}"#;

        let (body, record) = event_to_transaction(fixture.as_bytes()).unwrap();
        assert!(matches!(body, TransactionBody::AnchorBatch { event_count: 1, .. }));
        assert_eq!(record.asset, "TR-MAIN-01");
        assert_eq!(record.event_type, "BREAKER_TRIP");
        assert_eq!(record.site_id, "site-1");
        assert_eq!(record.timestamp, "2026-03-15T22:00:00Z");
        // Extra context fields (site_name, asset_type, details) are ignored.
    }

    /// Transitional net: legacy camelCase publishers (AnchorableEvent shape)
    /// still parse via serde aliases instead of being dropped silently.
    #[test]
    fn test_legacy_camelcase_alias_net() {
        let legacy = r#"{
            "id": "evt-1",
            "eventType": "BREAKER_TRIP",
            "siteId": "site-1",
            "timestamp": "2026-03-15T22:00:00Z",
            "severity": "critical",
            "message": "Breaker tripped",
            "data": {"voltage": 138.5}
        }"#;

        let (_, record) = event_to_transaction(legacy.as_bytes()).unwrap();
        assert_eq!(record.event_type, "BREAKER_TRIP");
        assert_eq!(record.site_id, "site-1");
        // Legacy shape has no asset field; the default is empty, not an error.
        assert_eq!(record.asset, "");
    }
}
