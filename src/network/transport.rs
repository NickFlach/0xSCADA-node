use quinn::{Connection, Endpoint, RecvStream, SendStream};
use std::net::SocketAddr;
use std::sync::Arc;

/// Framed message: length-prefixed CBOR over QUIC streams
/// Format: [u32 big-endian length][CBOR payload]
pub const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024; // 4MB

/// QUIC transport layer
pub struct Transport {
    endpoint: Endpoint,
    local_addr: SocketAddr,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("quinn error: {0}")]
    Quinn(String),
    #[error("connection error: {0}")]
    Connection(String),
    #[error("message too large: {0} > {1}")]
    MessageTooLarge(usize, usize),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Custom certificate verifier that accepts all certs.
/// We verify identity via ed25519 signatures on messages, not TLS certs.
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl Transport {
    /// Create a new QUIC endpoint listening on the given address.
    /// Generates a self-signed TLS certificate for the connection.
    pub async fn bind(addr: SocketAddr) -> Result<Self, TransportError> {
        // Generate self-signed cert
        let key_pair = rcgen::KeyPair::generate()
            .map_err(|e| TransportError::Quinn(e.to_string()))?;
        let cert_params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .map_err(|e| TransportError::Quinn(e.to_string()))?;
        let cert = cert_params
            .self_signed(&key_pair)
            .map_err(|e| TransportError::Quinn(e.to_string()))?;

        let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
            .map_err(|e| TransportError::Quinn(e.to_string()))?;

        // Server config
        let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TransportError::Quinn(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der.clone_key())
        .map_err(|e| TransportError::Quinn(e.to_string()))?;

        let server_config = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_config)
                .map_err(|e| TransportError::Quinn(e.to_string()))?,
        ));

        // Client config
        let client_config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TransportError::Quinn(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(client_config)
                .map_err(|e| TransportError::Quinn(e.to_string()))?,
        ));

        let mut endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| TransportError::Quinn(e.to_string()))?;
        endpoint.set_default_client_config(client_config);

        let local_addr = endpoint
            .local_addr()
            .map_err(|e| TransportError::Quinn(e.to_string()))?;

        Ok(Self {
            endpoint,
            local_addr,
        })
    }

    /// Connect to a remote peer
    pub async fn connect(&self, addr: SocketAddr) -> Result<Connection, TransportError> {
        self.endpoint
            .connect(addr, "localhost")
            .map_err(|e| TransportError::Connection(e.to_string()))?
            .await
            .map_err(|e| TransportError::Connection(e.to_string()))
    }

    /// Accept an incoming connection
    pub async fn accept(&self) -> Result<Connection, TransportError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| TransportError::Connection("endpoint closed".into()))?;
        incoming
            .await
            .map_err(|e| TransportError::Connection(e.to_string()))
    }

    /// Send a framed message on a new unidirectional stream
    pub async fn send(conn: &Connection, data: &[u8]) -> Result<(), TransportError> {
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge(data.len(), MAX_MESSAGE_SIZE));
        }
        let mut stream = conn
            .open_uni()
            .await
            .map_err(|e| TransportError::Connection(e.to_string()))?;
        write_frame(&mut stream, data).await?;
        stream
            .finish()
            .map_err(|e| TransportError::Connection(e.to_string()))?;
        Ok(())
    }

    /// Receive a framed message from a unidirectional stream
    pub async fn recv(conn: &Connection) -> Result<Vec<u8>, TransportError> {
        let mut stream = conn
            .accept_uni()
            .await
            .map_err(|e| TransportError::Connection(e.to_string()))?;
        read_frame(&mut stream).await
    }

    /// Local address
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Shutdown the endpoint
    pub fn shutdown(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

/// Helper: write a length-prefixed frame
async fn write_frame(stream: &mut SendStream, data: &[u8]) -> Result<(), TransportError> {
    let len = data.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| TransportError::Connection(e.to_string()))?;
    stream
        .write_all(data)
        .await
        .map_err(|e| TransportError::Connection(e.to_string()))?;
    Ok(())
}

/// Helper: read a length-prefixed frame
async fn read_frame(stream: &mut RecvStream) -> Result<Vec<u8>, TransportError> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| TransportError::Connection(e.to_string()))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(TransportError::MessageTooLarge(len, MAX_MESSAGE_SIZE));
    }
    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| TransportError::Connection(e.to_string()))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn localhost() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    }

    #[tokio::test]
    async fn test_bind_and_local_addr() {
        let transport = Transport::bind(localhost()).await.unwrap();
        let addr = transport.local_addr();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0);
        transport.shutdown();
    }

    #[tokio::test]
    async fn test_connect_and_send_recv() {
        let server = Transport::bind(localhost()).await.unwrap();
        let client = Transport::bind(localhost()).await.unwrap();
        let server_addr = server.local_addr();

        let msg = b"hello QUIC";

        let server_handle = tokio::spawn(async move {
            let conn = server.accept().await.unwrap();
            let received = Transport::recv(&conn).await.unwrap();
            assert_eq!(received, msg);
            server.shutdown();
        });

        let conn = client.connect(server_addr).await.unwrap();
        Transport::send(&conn, msg).await.unwrap();

        server_handle.await.unwrap();
        client.shutdown();
    }

    #[tokio::test]
    async fn test_message_roundtrip() {
        use crate::consensus::messages::ConsensusMessage;

        let server = Transport::bind(localhost()).await.unwrap();
        let client = Transport::bind(localhost()).await.unwrap();
        let server_addr = server.local_addr();

        let original = ConsensusMessage::PhaseUpdate {
            node_id: [42u8; 32],
            phase: 1.234,
            natural_freq: 0.5,
            timestamp: 7,
        };

        let encoded = original.serialize();

        let encoded_clone = encoded.clone();
        let server_handle = tokio::spawn(async move {
            let conn = server.accept().await.unwrap();
            let received = Transport::recv(&conn).await.unwrap();
            let decoded = ConsensusMessage::deserialize(&received).unwrap();
            if let ConsensusMessage::PhaseUpdate { phase, timestamp, .. } = decoded {
                assert_eq!(phase, 1.234);
                assert_eq!(timestamp, 7);
            } else {
                panic!("wrong variant");
            }
            server.shutdown();
        });

        let conn = client.connect(server_addr).await.unwrap();
        Transport::send(&conn, &encoded_clone).await.unwrap();

        server_handle.await.unwrap();
        client.shutdown();
    }

    #[tokio::test]
    async fn test_reject_oversized_message() {
        let server = Transport::bind(localhost()).await.unwrap();
        let client = Transport::bind(localhost()).await.unwrap();
        let server_addr = server.local_addr();

        let oversized = vec![0u8; MAX_MESSAGE_SIZE + 1];

        // Server must accept so the client can connect
        let server_handle = tokio::spawn(async move {
            let _conn = server.accept().await.unwrap();
            // keep connection alive until client is done
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            server.shutdown();
        });

        let conn = client.connect(server_addr).await.unwrap();
        let result = Transport::send(&conn, &oversized).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TransportError::MessageTooLarge(size, max) => {
                assert_eq!(size, MAX_MESSAGE_SIZE + 1);
                assert_eq!(max, MAX_MESSAGE_SIZE);
            }
            other => panic!("expected MessageTooLarge, got: {other}"),
        }

        client.shutdown();
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_multiple_messages() {
        let server = Transport::bind(localhost()).await.unwrap();
        let client = Transport::bind(localhost()).await.unwrap();
        let server_addr = server.local_addr();

        let server_handle = tokio::spawn(async move {
            let conn = server.accept().await.unwrap();
            for i in 0u32..10 {
                let received = Transport::recv(&conn).await.unwrap();
                let expected = format!("message-{i}");
                assert_eq!(received, expected.as_bytes());
            }
            server.shutdown();
        });

        let conn = client.connect(server_addr).await.unwrap();
        for i in 0u32..10 {
            let msg = format!("message-{i}");
            Transport::send(&conn, msg.as_bytes()).await.unwrap();
        }

        server_handle.await.unwrap();
        client.shutdown();
    }
}
