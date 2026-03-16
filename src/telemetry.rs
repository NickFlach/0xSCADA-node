use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct TelemetryStatus {
    pub node_id: String,           // hex
    pub height: u64,
    pub role: String,
    pub order_parameter: f64,
    pub mean_phase: f64,
    pub local_phase: f64,
    pub peer_count: usize,
    pub coupling_k: f64,
    pub chiral_eta: f64,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TelemetryTuning {
    pub node_id: String,
    pub coupling_k: f64,
    pub chiral_eta: f64,
    pub mean_r: f64,
    pub adjustment: Option<String>, // description of last adjustment
    pub timestamp_ms: u64,
}

pub struct TelemetryPublisher {
    client: async_nats::Client,
    prefix: String,
    node_id_short: String,
}

impl TelemetryPublisher {
    pub async fn connect(nats_url: &str, prefix: &str, node_id: &str) -> Result<Self, async_nats::ConnectError> {
        let client = async_nats::connect(nats_url).await?;
        Ok(Self {
            client,
            prefix: prefix.to_string(),
            node_id_short: node_id[..16].to_string(),
        })
    }

    pub async fn publish_status(&self, status: &TelemetryStatus) -> Result<(), async_nats::PublishError> {
        let subject = format!("{}.{}.status", self.prefix, self.node_id_short);
        let payload = serde_json::to_vec(status).unwrap_or_default();
        self.client.publish(subject, payload.into()).await
    }

    pub async fn publish_tuning(&self, tuning: &TelemetryTuning) -> Result<(), async_nats::PublishError> {
        let subject = format!("{}.{}.tuning", self.prefix, self.node_id_short);
        let payload = serde_json::to_vec(tuning).unwrap_or_default();
        self.client.publish(subject, payload.into()).await
    }
}

/// Try to connect, return None if NATS unavailable (graceful degradation)
pub async fn try_connect(nats_url: &str, prefix: &str, node_id: &str) -> Option<TelemetryPublisher> {
    match TelemetryPublisher::connect(nats_url, prefix, node_id).await {
        Ok(pub_) => {
            println!("[telemetry] Connected to NATS at {nats_url}, prefix={prefix}");
            Some(pub_)
        }
        Err(e) => {
            eprintln!("[telemetry] NATS connection failed (degraded mode): {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_status_serialization() {
        let status = TelemetryStatus {
            node_id: "abcdef1234567890".to_string(),
            height: 100,
            role: "leader".to_string(),
            order_parameter: 0.85,
            mean_phase: 1.57,
            local_phase: 1.60,
            peer_count: 3,
            coupling_k: 0.5,
            chiral_eta: 0.01,
            timestamp_ms: 1234567890000,
        };

        let json = serde_json::to_value(&status).unwrap();
        
        // Check that all expected fields are present
        assert!(json["node_id"].is_string());
        assert!(json["height"].is_number());
        assert!(json["role"].is_string());
        assert!(json["order_parameter"].is_number());
        assert!(json["mean_phase"].is_number());
        assert!(json["local_phase"].is_number());
        assert!(json["peer_count"].is_number());
        assert!(json["coupling_k"].is_number());
        assert!(json["chiral_eta"].is_number());
        assert!(json["timestamp_ms"].is_number());

        // Check specific values
        assert_eq!(json["node_id"], "abcdef1234567890");
        assert_eq!(json["height"], 100);
        assert_eq!(json["role"], "leader");
        assert_eq!(json["order_parameter"], 0.85);
        assert_eq!(json["peer_count"], 3);
    }

    #[test]
    fn test_telemetry_tuning_serialization() {
        let tuning = TelemetryTuning {
            node_id: "abcdef1234567890".to_string(),
            coupling_k: 0.6,
            chiral_eta: 0.02,
            mean_r: 0.75,
            adjustment: Some("increased coupling due to low order parameter".to_string()),
            timestamp_ms: 1234567890000,
        };

        let json = serde_json::to_value(&tuning).unwrap();
        
        // Check that all expected fields are present
        assert!(json["node_id"].is_string());
        assert!(json["coupling_k"].is_number());
        assert!(json["chiral_eta"].is_number());
        assert!(json["mean_r"].is_number());
        assert!(json["adjustment"].is_string());
        assert!(json["timestamp_ms"].is_number());

        // Check specific values
        assert_eq!(json["node_id"], "abcdef1234567890");
        assert_eq!(json["coupling_k"], 0.6);
        assert_eq!(json["chiral_eta"], 0.02);
        assert_eq!(json["mean_r"], 0.75);
        assert_eq!(json["adjustment"], "increased coupling due to low order parameter");
    }

    #[test]
    fn test_telemetry_tuning_serialization_no_adjustment() {
        let tuning = TelemetryTuning {
            node_id: "abcdef1234567890".to_string(),
            coupling_k: 0.5,
            chiral_eta: 0.01,
            mean_r: 0.80,
            adjustment: None,
            timestamp_ms: 1234567890000,
        };

        let json = serde_json::to_value(&tuning).unwrap();
        assert!(json["adjustment"].is_null());
    }
}