use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;

pub struct NodeKeypair {
    signing_key: SigningKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(pub [u8; 32]);

#[derive(Debug, Clone)]
pub struct Signature(pub [u8; 64]);

impl From<PublicKey> for [u8; 32] {
    fn from(pk: PublicKey) -> [u8; 32] {
        pk.0
    }
}

impl From<[u8; 32]> for PublicKey {
    fn from(bytes: [u8; 32]) -> PublicKey {
        PublicKey(bytes)
    }
}

impl From<Signature> for [u8; 64] {
    fn from(sig: Signature) -> [u8; 64] {
        sig.0
    }
}

impl From<[u8; 64]> for Signature {
    fn from(bytes: [u8; 64]) -> Signature {
        Signature(bytes)
    }
}

impl NodeKeypair {
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut OsRng),
        }
    }

    pub fn from_bytes(secret: &[u8; 32]) -> Result<Self, ed25519_dalek::SignatureError> {
        let signing_key = SigningKey::from_bytes(secret);
        // Verify we can derive the verifying key (always works for ed25519-dalek v2)
        let _ = signing_key.verifying_key();
        Ok(Self { signing_key })
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing_key.verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        let sig = self.signing_key.sign(message);
        Signature(sig.to_bytes())
    }
}

pub fn verify(public_key: &PublicKey, message: &[u8], signature: &Signature) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(&public_key.0) else {
        return false;
    };
    let sig = ed25519_dalek::Signature::from_bytes(&signature.0);
    verifying_key.verify(message, &sig).is_ok()
}

pub fn hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypair_generation() {
        let kp = NodeKeypair::generate();
        let msg = b"hello 0xSCADA";
        let sig = kp.sign(msg);
        assert!(verify(&kp.public_key(), msg, &sig));
    }

    #[test]
    fn test_invalid_signature() {
        let kp = NodeKeypair::generate();
        let sig = kp.sign(b"original message");
        assert!(!verify(&kp.public_key(), b"tampered message", &sig));
    }

    #[test]
    fn test_hash_determinism() {
        let data = b"deterministic input";
        assert_eq!(hash(data), hash(data));
    }

    #[test]
    fn test_different_inputs_different_hashes() {
        assert_ne!(hash(b"input A"), hash(b"input B"));
    }

    #[test]
    fn test_keypair_from_bytes_roundtrip() {
        let kp1 = NodeKeypair::generate();
        let secret = kp1.signing_key.to_bytes();
        let kp2 = NodeKeypair::from_bytes(&secret).unwrap();
        assert_eq!(kp1.public_key(), kp2.public_key());

        let msg = b"roundtrip test";
        let sig = kp1.sign(msg);
        assert!(verify(&kp2.public_key(), msg, &sig));
    }

    #[test]
    fn test_public_key_from_into() {
        let pk = PublicKey([42u8; 32]);
        let bytes: [u8; 32] = pk.clone().into();
        assert_eq!(bytes, [42u8; 32]);
        let pk2: PublicKey = bytes.into();
        assert_eq!(pk, pk2);
    }

    #[test]
    fn test_signature_from_into() {
        let sig = Signature([0xAB; 64]);
        let bytes: [u8; 64] = sig.into();
        assert_eq!(bytes, [0xAB; 64]);
        let sig2: Signature = bytes.into();
        assert_eq!(sig2.0, [0xAB; 64]);
    }
}
