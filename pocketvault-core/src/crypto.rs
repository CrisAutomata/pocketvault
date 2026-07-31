use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::ZeroizeOnDrop;

use crate::error::{Result, VaultError};

pub const KEY_SIZE: usize = 32;
pub const NONCE_SIZE: usize = 12;

#[derive(Clone, ZeroizeOnDrop)]
pub struct VaultKey(pub [u8; KEY_SIZE]);

pub struct KdfParams {
    pub salt: Vec<u8>,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl KdfParams {
    pub fn default_secure() -> Self {
        Self {
            salt: random_bytes(32),
            // Use minimal params during tests so every test that calls
            // create_new / unlock completes in milliseconds instead of seconds.
            m_cost: if cfg!(test) { 4096 } else { 65536 },
            t_cost: if cfg!(test) { 1 } else { 3 },
            p_cost: 1,
        }
    }
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    OsRng.fill_bytes(&mut buf);
    buf
}

pub fn derive_key(password: &str, params: &KdfParams) -> Result<VaultKey> {
    let argon2_params = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(KEY_SIZE))
        .map_err(|_| VaultError::EncryptionFailed)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let mut key_bytes = [0u8; KEY_SIZE];
    argon2
        .hash_password_into(password.as_bytes(), &params.salt, &mut key_bytes)
        .map_err(|_| VaultError::EncryptionFailed)?;

    Ok(VaultKey(key_bytes))
}

pub fn encrypt(key: &VaultKey, plaintext: &[u8]) -> Result<(Vec<u8>, [u8; NONCE_SIZE])> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.0));
    let nonce_generic = Aes256Gcm::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce_generic, plaintext)
        .map_err(|_| VaultError::EncryptionFailed)?;

    let mut nonce = [0u8; NONCE_SIZE];
    nonce.copy_from_slice(&nonce_generic);

    Ok((ciphertext, nonce))
}

pub fn decrypt(key: &VaultKey, ciphertext: &[u8], nonce: &[u8; NONCE_SIZE]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.0));
    let nonce = Nonce::from_slice(nonce);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| VaultError::DecryptionFailed)
}

pub fn generate_vault_key() -> VaultKey {
    let mut key = [0u8; KEY_SIZE];
    OsRng.fill_bytes(&mut key);
    VaultKey(key)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_is_deterministic() {
        let params = KdfParams {
            salt: vec![1u8; 32],
            m_cost: 4096,
            t_cost: 1,
            p_cost: 1,
        };
        let k1 = derive_key("password", &params).unwrap();
        let k2 = derive_key("password", &params).unwrap();
        assert_eq!(k1.0, k2.0);
    }

    #[test]
    fn different_passwords_produce_different_keys() {
        let params = KdfParams {
            salt: vec![1u8; 32],
            m_cost: 4096,
            t_cost: 1,
            p_cost: 1,
        };
        let k1 = derive_key("password1", &params).unwrap();
        let k2 = derive_key("password2", &params).unwrap();
        assert_ne!(k1.0, k2.0);
    }

    #[test]
    fn different_salts_produce_different_keys() {
        let p1 = KdfParams {
            salt: vec![1u8; 32],
            m_cost: 4096,
            t_cost: 1,
            p_cost: 1,
        };
        let p2 = KdfParams {
            salt: vec![2u8; 32],
            m_cost: 4096,
            t_cost: 1,
            p_cost: 1,
        };
        let k1 = derive_key("password", &p1).unwrap();
        let k2 = derive_key("password", &p2).unwrap();
        assert_ne!(k1.0, k2.0);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = generate_vault_key();
        let plaintext = b"hello pocketvault!";
        let (ct, nonce) = encrypt(&key, plaintext).unwrap();
        let pt = decrypt(&key, &ct, &nonce).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn encrypt_empty_roundtrip() {
        let key = generate_vault_key();
        let (ct, nonce) = encrypt(&key, b"").unwrap();
        let pt = decrypt(&key, &ct, &nonce).unwrap();
        assert_eq!(pt, b"");
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let k1 = generate_vault_key();
        let k2 = generate_vault_key();
        let (ct, nonce) = encrypt(&k1, b"secret").unwrap();
        assert!(decrypt(&k2, &ct, &nonce).is_err());
    }

    #[test]
    fn decrypt_tampered_ciphertext_fails() {
        let key = generate_vault_key();
        let (mut ct, nonce) = encrypt(&key, b"secret").unwrap();
        ct[0] ^= 0xFF;
        assert!(decrypt(&key, &ct, &nonce).is_err());
    }

    #[test]
    fn each_encrypt_uses_unique_nonce() {
        let key = generate_vault_key();
        let (_, n1) = encrypt(&key, b"same").unwrap();
        let (_, n2) = encrypt(&key, b"same").unwrap();
        assert_ne!(n1, n2);
    }

    #[test]
    fn encrypt_large_data() {
        let key = generate_vault_key();
        let data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        let (ct, nonce) = encrypt(&key, &data).unwrap();
        let pt = decrypt(&key, &ct, &nonce).unwrap();
        assert_eq!(pt, data);
    }
}
