//! Envelope encryption for data-at-rest (BYOK).
//!
//! Implements AES-256-GCM envelope encryption: each document gets its own
//! randomly generated Data Encryption Key (DEK), which is itself encrypted
//! (wrapped) with a master Key Encryption Key (KEK) provided by the operator.
//!
//! # Envelope Format
//!
//! An encrypted document is stored as a JSON object:
//!
//! ```json
//! {
//!   "_encrypted": true,
//!   "_key_id": "kek-001",
//!   "_algorithm": "AES-256-GCM",
//!   "_dek_ciphertext": "<base64-wrapped-DEK>",
//!   "_nonce": "<base64-12-byte-nonce>",
//!   "_ciphertext": "<base64-encrypted-plaintext>"
//! }
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use mnemo_core::encryption::EnvelopeEncryptor;
//!
//! // Master key must be exactly 32 bytes (AES-256)
//! let master_key = [0u8; 32];
//! let enc = EnvelopeEncryptor::new(master_key, "kek-001".to_string());
//!
//! let plaintext = r#"{"name":"Alice","email":"alice@example.com"}"#;
//! let encrypted = enc.encrypt(plaintext).unwrap();
//! let decrypted = enc.decrypt(&encrypted).unwrap();
//! assert_eq!(plaintext, decrypted);
//! ```

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::MnemoError;

/// Marker to detect encrypted documents during deserialization.
const ENCRYPTED_MARKER: &str = "_encrypted";

/// Envelope metadata stored alongside the ciphertext.
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedEnvelope {
    /// Always `true` — marker for encrypted documents.
    _encrypted: bool,
    /// Identifier of the KEK used to wrap the DEK.
    _key_id: String,
    /// Algorithm used (always "AES-256-GCM").
    _algorithm: String,
    /// The DEK encrypted (wrapped) with the KEK, base64-encoded.
    _dek_ciphertext: String,
    /// 12-byte nonce used for the data encryption, base64-encoded.
    _nonce: String,
    /// The actual data encrypted with the DEK, base64-encoded.
    _ciphertext: String,
}

/// AES-256-GCM envelope encryptor.
///
/// The master key (KEK) is used to wrap per-document DEKs. Each document
/// gets a fresh random DEK and nonce for forward secrecy.
///
/// Debug is manually implemented to avoid leaking key material.
pub struct EnvelopeEncryptor {
    /// Key Encryption Key — wraps DEKs.
    kek: [u8; 32],
    /// Identifier for this KEK version (enables key rotation).
    key_id: String,
}

impl std::fmt::Debug for EnvelopeEncryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvelopeEncryptor")
            .field("key_id", &self.key_id)
            .field("kek", &"[REDACTED]")
            .finish()
    }
}

impl EnvelopeEncryptor {
    /// Create a new encryptor with the given 256-bit master key and key ID.
    pub fn new(kek: [u8; 32], key_id: String) -> Self {
        Self { kek, key_id }
    }

    /// Create from a base64-encoded master key string.
    pub fn from_base64(key_b64: &str, key_id: String) -> Result<Self, MnemoError> {
        let bytes = BASE64
            .decode(key_b64)
            .map_err(|e| MnemoError::Config(format!("Invalid base64 master key: {e}")))?;
        if bytes.len() != 32 {
            return Err(MnemoError::Config(format!(
                "Master key must be exactly 32 bytes (got {})",
                bytes.len()
            )));
        }
        let mut kek = [0u8; 32];
        kek.copy_from_slice(&bytes);
        Ok(Self { kek, key_id })
    }

    /// Encrypt plaintext using envelope encryption.
    ///
    /// 1. Generate a random 256-bit DEK.
    /// 2. Encrypt the plaintext with the DEK (AES-256-GCM).
    /// 3. Wrap (encrypt) the DEK with the KEK.
    /// 4. Return the envelope as a JSON string.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, MnemoError> {
        // Generate random DEK
        let mut dek_bytes = Aes256Gcm::generate_key(OsRng);

        // Encrypt plaintext with DEK
        let dek_cipher = Aes256Gcm::new(&dek_bytes);
        let data_nonce = Aes256Gcm::generate_nonce(OsRng);
        let ciphertext = dek_cipher
            .encrypt(&data_nonce, plaintext.as_bytes())
            .map_err(|e| MnemoError::Internal(format!("Encryption failed: {e}")))?;

        // Wrap DEK with KEK
        let kek_key = Key::<Aes256Gcm>::from_slice(&self.kek);
        let kek_cipher = Aes256Gcm::new(kek_key);
        let kek_nonce = Aes256Gcm::generate_nonce(OsRng);

        // Prepend KEK nonce to the wrapped DEK so we can unwrap later
        let wrapped_dek = kek_cipher
            .encrypt(&kek_nonce, dek_bytes.as_slice())
            .map_err(|e| MnemoError::Internal(format!("DEK wrapping failed: {e}")))?;

        // Combine KEK nonce + wrapped DEK into one blob
        let mut dek_blob = kek_nonce.to_vec();
        dek_blob.extend_from_slice(&wrapped_dek);

        // Zeroize raw DEK from memory
        dek_bytes.zeroize();

        let envelope = EncryptedEnvelope {
            _encrypted: true,
            _key_id: self.key_id.clone(),
            _algorithm: "AES-256-GCM".to_string(),
            _dek_ciphertext: BASE64.encode(&dek_blob),
            _nonce: BASE64.encode(data_nonce),
            _ciphertext: BASE64.encode(ciphertext),
        };

        serde_json::to_string(&envelope)
            .map_err(|e| MnemoError::Internal(format!("Envelope serialization failed: {e}")))
    }

    /// Decrypt an envelope-encrypted document.
    ///
    /// 1. Parse the JSON envelope.
    /// 2. Unwrap the DEK using the KEK.
    /// 3. Decrypt the ciphertext with the DEK.
    /// 4. Return the plaintext string.
    pub fn decrypt(&self, envelope_json: &str) -> Result<String, MnemoError> {
        let envelope: EncryptedEnvelope = serde_json::from_str(envelope_json)
            .map_err(|e| MnemoError::Internal(format!("Envelope parse failed: {e}")))?;

        if !envelope._encrypted {
            return Err(MnemoError::Internal(
                "Document is not encrypted (marker is false)".to_string(),
            ));
        }

        // Decode base64 fields
        let dek_blob = BASE64
            .decode(&envelope._dek_ciphertext)
            .map_err(|e| MnemoError::Internal(format!("Invalid DEK ciphertext base64: {e}")))?;
        let data_nonce_bytes = BASE64
            .decode(&envelope._nonce)
            .map_err(|e| MnemoError::Internal(format!("Invalid nonce base64: {e}")))?;
        let ciphertext = BASE64
            .decode(&envelope._ciphertext)
            .map_err(|e| MnemoError::Internal(format!("Invalid ciphertext base64: {e}")))?;

        // Split DEK blob into KEK nonce (12 bytes) + wrapped DEK
        if dek_blob.len() < 13 {
            return Err(MnemoError::Internal(
                "DEK blob too short (expected nonce + wrapped key)".to_string(),
            ));
        }
        let (kek_nonce_bytes, wrapped_dek) = dek_blob.split_at(12);
        let kek_nonce = Nonce::from_slice(kek_nonce_bytes);

        // Unwrap DEK with KEK
        let kek_key = Key::<Aes256Gcm>::from_slice(&self.kek);
        let kek_cipher = Aes256Gcm::new(kek_key);
        let mut dek_bytes = kek_cipher.decrypt(kek_nonce, wrapped_dek).map_err(|_| {
            MnemoError::Internal(
                "DEK unwrap failed — wrong master key or corrupted data".to_string(),
            )
        })?;

        if dek_bytes.len() != 32 {
            dek_bytes.zeroize();
            return Err(MnemoError::Internal(format!(
                "Unwrapped DEK has wrong length (expected 32, got {})",
                dek_bytes.len()
            )));
        }

        // Decrypt data with DEK
        let dek_key = Key::<Aes256Gcm>::from_slice(&dek_bytes);
        let dek_cipher = Aes256Gcm::new(dek_key);
        let data_nonce = Nonce::from_slice(&data_nonce_bytes);
        let plaintext_bytes = dek_cipher
            .decrypt(data_nonce, ciphertext.as_slice())
            .map_err(|_| {
                MnemoError::Internal(
                    "Data decryption failed — corrupted ciphertext or wrong DEK".to_string(),
                )
            })?;

        // Zeroize DEK from memory
        dek_bytes.zeroize();

        String::from_utf8(plaintext_bytes)
            .map_err(|e| MnemoError::Internal(format!("Decrypted data is not valid UTF-8: {e}")))
    }
}

/// Check if a JSON string represents an encrypted envelope.
pub fn is_encrypted(json_str: &str) -> bool {
    // Fast path: check for the marker field without full parse
    json_str.contains(&format!("\"{}\"", ENCRYPTED_MARKER)) && json_str.contains("\"_ciphertext\"")
}

impl Drop for EnvelopeEncryptor {
    fn drop(&mut self) {
        self.kek.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        // Deterministic test key — never use in production
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let enc = EnvelopeEncryptor::new(test_key(), "test-key-001".to_string());
        let plaintext = r#"{"name":"Alice","email":"alice@example.com"}"#;

        let encrypted = enc.encrypt(plaintext).unwrap();
        assert!(is_encrypted(&encrypted));
        assert!(!encrypted.contains("Alice")); // plaintext must not appear

        let decrypted = enc.decrypt(&encrypted).unwrap();
        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_different_encryptions_produce_different_ciphertext() {
        let enc = EnvelopeEncryptor::new(test_key(), "test-key-001".to_string());
        let plaintext = "hello world";

        let a = enc.encrypt(plaintext).unwrap();
        let b = enc.encrypt(plaintext).unwrap();
        // Different random DEK + nonce each time
        assert_ne!(a, b);

        // Both decrypt to the same plaintext
        assert_eq!(enc.decrypt(&a).unwrap(), plaintext);
        assert_eq!(enc.decrypt(&b).unwrap(), plaintext);
    }

    #[test]
    fn test_wrong_key_fails_decryption() {
        let enc1 = EnvelopeEncryptor::new(test_key(), "key-1".to_string());
        let mut wrong_key = test_key();
        wrong_key[0] ^= 0xFF; // flip one byte
        let enc2 = EnvelopeEncryptor::new(wrong_key, "key-2".to_string());

        let encrypted = enc1.encrypt("secret data").unwrap();
        let result = enc2.decrypt(&encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_encrypted_detection() {
        assert!(!is_encrypted(r#"{"name":"Alice"}"#));
        assert!(is_encrypted(r#"{"_encrypted":true,"_ciphertext":"abc"}"#));
    }

    #[test]
    fn test_from_base64_valid() {
        let key = test_key();
        let b64 = BASE64.encode(key);
        let enc = EnvelopeEncryptor::from_base64(&b64, "k1".to_string()).unwrap();

        let ct = enc.encrypt("test").unwrap();
        let pt = enc.decrypt(&ct).unwrap();
        assert_eq!(pt, "test");
    }

    #[test]
    fn test_from_base64_wrong_length() {
        let result = EnvelopeEncryptor::from_base64(&BASE64.encode([0u8; 16]), "k1".to_string());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("32 bytes"));
    }

    #[test]
    fn test_from_base64_invalid_base64() {
        let result = EnvelopeEncryptor::from_base64("not-valid-base64!!!", "k1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_large_document() {
        let enc = EnvelopeEncryptor::new(test_key(), "test-key-001".to_string());
        // 1 MB of data
        let large = "x".repeat(1_000_000);
        let encrypted = enc.encrypt(&large).unwrap();
        let decrypted = enc.decrypt(&encrypted).unwrap();
        assert_eq!(large, decrypted);
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let enc = EnvelopeEncryptor::new(test_key(), "test-key-001".to_string());
        let encrypted = enc.encrypt("sensitive data").unwrap();

        // Tamper with the ciphertext field
        let tampered = encrypted.replace("_ciphertext\":\"", "_ciphertext\":\"AAAA");
        let result = enc.decrypt(&tampered);
        assert!(result.is_err());
    }

    #[test]
    fn test_envelope_contains_key_id() {
        let enc = EnvelopeEncryptor::new(test_key(), "kek-2025-q1".to_string());
        let encrypted = enc.encrypt("data").unwrap();
        assert!(encrypted.contains("kek-2025-q1"));
    }
}
