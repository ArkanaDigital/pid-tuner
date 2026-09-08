//! Light at-rest protection for API keys: ChaCha20-Poly1305 with a key derived
//! from the machine id. This keeps keys out of plain text but anyone who can
//! read the file *and* obtain the machine id can decrypt them.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use serde::{Deserialize, Serialize};

pub const SALT: &[u8] = b"pid-tuner-settings-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncBlob {
    /// base64, 12 bytes
    pub nonce: String,
    /// base64 ciphertext + tag
    pub ct: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("decryption failed")]
    Open,
    #[error("malformed blob: {0}")]
    Malformed(String),
}

/// Key derived from the machine id (falls back to a fixed string when the id
/// is unavailable, e.g. inside containers).
pub fn machine_key() -> [u8; 32] {
    let id = machine_uid::get().unwrap_or_else(|_| "no-machine-id".to_string());
    derive_key(id.as_bytes())
}

pub fn derive_key(material: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(material);
    h.update(SALT);
    *h.finalize().as_bytes()
}

fn random_nonce() -> [u8; 12] {
    let mut n = [0u8; 12];
    // getrandom via the aead OsRng feature would add a dependency; use the
    // system time + a blake3 of it, which is unique enough for a per-save nonce.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let seed = blake3::hash(&(now ^ (pid << 64)).to_le_bytes());
    n.copy_from_slice(&seed.as_bytes()[..12]);
    n
}

pub fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> EncBlob {
    use base64::Engine;
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = random_nonce();
    let n = Nonce::try_from(&nonce[..]).expect("12-byte nonce");
    let ct = cipher
        .encrypt(&n, Payload { msg: plain, aad })
        .expect("chacha20poly1305 encrypt");
    EncBlob {
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        ct: base64::engine::general_purpose::STANDARD.encode(ct),
    }
}

pub fn open(key: &[u8; 32], aad: &[u8], blob: &EncBlob) -> Result<Vec<u8>, CryptoError> {
    use base64::Engine;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(&blob.nonce)
        .map_err(|e| CryptoError::Malformed(e.to_string()))?;
    let ct = base64::engine::general_purpose::STANDARD
        .decode(&blob.ct)
        .map_err(|e| CryptoError::Malformed(e.to_string()))?;
    if nonce.len() != 12 {
        return Err(CryptoError::Malformed("nonce length".into()));
    }
    let cipher = ChaCha20Poly1305::new(key.into());
    let n = Nonce::try_from(&nonce[..]).map_err(|_| CryptoError::Malformed("nonce".into()))?;
    cipher
        .decrypt(&n, Payload { msg: &ct, aad })
        .map_err(|_| CryptoError::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_tamper_detection() {
        let k = derive_key(b"machine-A");
        let b = seal(&k, b"anthropic", b"sk-ant-secret");
        assert_eq!(open(&k, b"anthropic", &b).unwrap(), b"sk-ant-secret");
        assert!(matches!(open(&k, b"openai", &b), Err(CryptoError::Open)));
        let k2 = derive_key(b"machine-B");
        assert!(matches!(
            open(&k2, b"anthropic", &b),
            Err(CryptoError::Open)
        ));
        let mut bad = b.clone();
        bad.ct = "!!!".into();
        assert!(matches!(
            open(&k, b"anthropic", &bad),
            Err(CryptoError::Malformed(_))
        ));
        // fresh nonce per seal
        let b2 = seal(&k, b"anthropic", b"sk-ant-secret");
        assert_ne!(b.nonce, b2.nonce);
    }
}
