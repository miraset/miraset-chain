use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Public key (address)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address([u8; 32]);

impl Address {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.to_hex())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.to_hex()[..8])
    }
}

/// Keypair for signing
#[derive(Clone, Debug)]
pub struct KeyPair {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut secret_bytes = [0u8; 32];
        rand::Rng::fill(&mut OsRng, &mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    pub fn address(&self) -> Address {
        Address(*self.verifying_key.as_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }
}

pub fn verify_signature(address: &Address, message: &[u8], signature: &[u8; 64]) -> bool {
    let verifying_key = match VerifyingKey::from_bytes(address.as_bytes()) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(signature);
    verifying_key.verify(message, &sig).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypair_sign_verify() {
        let kp = KeyPair::generate();
        let msg = b"hello world";
        let sig = kp.sign(msg);
        assert!(verify_signature(&kp.address(), msg, &sig));
        assert!(!verify_signature(&kp.address(), b"other", &sig));
    }

    #[test]
    fn test_address_hex() {
        let kp = KeyPair::generate();
        let addr = kp.address();
        let hex = addr.to_hex();
        let addr2 = Address::from_hex(&hex).unwrap();
        assert_eq!(addr, addr2);
    }

    #[test]
    fn test_keypair_from_bytes() {
        let secret = [42u8; 32];
        let kp1 = KeyPair::from_bytes(&secret);
        let kp2 = KeyPair::from_bytes(&secret);

        // Same secret should produce same address
        assert_eq!(kp1.address(), kp2.address());
        assert_eq!(kp1.secret_bytes(), kp2.secret_bytes());
    }

    #[test]
    fn test_signature_replay() {
        let kp = KeyPair::generate();
        let msg1 = b"message 1";
        let msg2 = b"message 2";

        let sig1 = kp.sign(msg1);
        let sig2 = kp.sign(msg2);

        // Each message should have unique signature
        assert_ne!(sig1, sig2);

        // Signatures should not be interchangeable
        assert!(!verify_signature(&kp.address(), msg1, &sig2));
        assert!(!verify_signature(&kp.address(), msg2, &sig1));
    }

    #[test]
    fn test_invalid_signature() {
        let kp = KeyPair::generate();
        let msg = b"test message";
        let sig = kp.sign(msg);

        // Modify signature
        let mut bad_sig = sig;
        bad_sig[0] = bad_sig[0].wrapping_add(1);

        assert!(!verify_signature(&kp.address(), msg, &bad_sig));
    }

    #[test]
    fn test_wrong_address() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let msg = b"test";
        let sig = kp1.sign(msg);

        // Signature from kp1 should not verify with kp2's address
        assert!(!verify_signature(&kp2.address(), msg, &sig));
    }

    #[test]
    fn test_address_from_invalid_hex() {
        assert!(Address::from_hex("invalid").is_err());
        assert!(Address::from_hex("").is_err());
        assert!(Address::from_hex("00").is_err()); // Too short
    }

    #[test]
    fn test_address_display() {
        let kp = KeyPair::generate();
        let addr = kp.address();
        let display = format!("{}", addr);

        // Display should be 8 characters (first 8 hex chars)
        assert_eq!(display.len(), 8);
    }

    #[test]
    fn test_address_debug() {
        let kp = KeyPair::generate();
        let addr = kp.address();
        let debug = format!("{:?}", addr);

        // Debug should contain full hex
        assert!(debug.contains("Address("));
        assert!(debug.len() > 40); // Address(64 hex chars...)
    }

    #[test]
    fn test_signature_deterministic() {
        let secret = [7u8; 32];
        let kp = KeyPair::from_bytes(&secret);
        let msg = b"deterministic test";

        let sig1 = kp.sign(msg);
        let sig2 = kp.sign(msg);

        // Same keypair and message should produce same signature
        assert_eq!(sig1, sig2);
    }
}
