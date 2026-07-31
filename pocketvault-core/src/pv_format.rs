//! .pv file format
//!
//! Layout:
//!   [magic: 4]  [version: 1]
//!   [meta_nonce: 12]  [meta_len: 4 LE]  [enc_metadata: meta_len]
//!   [chunk_count: 4 LE]
//!   for each chunk:
//!     [chunk_nonce: 12]  [chunk_len: 4 LE]  [enc_chunk: chunk_len]

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::crypto::{decrypt, encrypt, VaultKey, NONCE_SIZE};
use crate::error::{Result, VaultError};

const MAGIC: &[u8; 4] = b"PVLT";
const VERSION: u8 = 1;
pub const CHUNK_SIZE: usize = 65536; // 64 KB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PvMetadata {
    pub original_name: String,
    pub original_size: u64,
    pub created_ts: i64,
    pub modified_ts: i64,
    pub mime_type: String,
}

pub fn write_pv<W: Write>(
    writer: &mut W,
    key: &VaultKey,
    metadata: &PvMetadata,
    plaintext: &[u8],
) -> Result<()> {
    writer.write_all(MAGIC)?;
    writer.write_all(&[VERSION])?;

    let meta_json = serde_json::to_vec(metadata)?;
    let (enc_meta, meta_nonce) = encrypt(key, &meta_json)?;
    writer.write_all(&meta_nonce)?;
    writer.write_all(&(enc_meta.len() as u32).to_le_bytes())?;
    writer.write_all(&enc_meta)?;

    let chunks: Vec<&[u8]> = plaintext.chunks(CHUNK_SIZE).collect();
    writer.write_all(&(chunks.len() as u32).to_le_bytes())?;

    for chunk in chunks {
        let (enc_chunk, chunk_nonce) = encrypt(key, chunk)?;
        writer.write_all(&chunk_nonce)?;
        writer.write_all(&(enc_chunk.len() as u32).to_le_bytes())?;
        writer.write_all(&enc_chunk)?;
    }

    Ok(())
}

fn read_header<R: Read>(reader: &mut R) -> Result<()> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(VaultError::InvalidFileFormat);
    }
    let mut ver = [0u8; 1];
    reader.read_exact(&mut ver)?;
    if ver[0] != VERSION {
        return Err(VaultError::InvalidFileFormat);
    }
    Ok(())
}

fn read_meta<R: Read>(reader: &mut R, key: &VaultKey) -> Result<PvMetadata> {
    let mut nonce = [0u8; NONCE_SIZE];
    reader.read_exact(&mut nonce)?;

    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut enc = vec![0u8; len];
    reader.read_exact(&mut enc)?;

    let json = decrypt(key, &enc, &nonce)?;
    Ok(serde_json::from_slice(&json)?)
}

pub fn read_pv_metadata<R: Read>(reader: &mut R, key: &VaultKey) -> Result<PvMetadata> {
    read_header(reader)?;
    read_meta(reader, key)
}

pub fn read_pv<R: Read>(reader: &mut R, key: &VaultKey) -> Result<(PvMetadata, Vec<u8>)> {
    read_header(reader)?;
    let metadata = read_meta(reader, key)?;

    let mut count_buf = [0u8; 4];
    reader.read_exact(&mut count_buf)?;
    let chunk_count = u32::from_le_bytes(count_buf) as usize;

    let mut plaintext = Vec::with_capacity(metadata.original_size as usize);

    for _ in 0..chunk_count {
        let mut chunk_nonce = [0u8; NONCE_SIZE];
        reader.read_exact(&mut chunk_nonce)?;

        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut enc_chunk = vec![0u8; len];
        reader.read_exact(&mut enc_chunk)?;

        let chunk = decrypt(key, &enc_chunk, &chunk_nonce)?;
        plaintext.extend_from_slice(&chunk);
    }

    Ok((metadata, plaintext))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_vault_key;
    use std::io::Cursor;

    fn meta(name: &str, size: u64) -> PvMetadata {
        PvMetadata {
            original_name: name.to_string(),
            original_size: size,
            created_ts: 1_000_000,
            modified_ts: 2_000_000,
            mime_type: "application/octet-stream".to_string(),
        }
    }

    #[test]
    fn roundtrip_small() {
        let key = generate_vault_key();
        let data = b"hello pocketvault";
        let mut buf = Vec::new();
        write_pv(&mut buf, &key, &meta("hello.txt", data.len() as u64), data).unwrap();

        let (m, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(out, data);
        assert_eq!(m.original_name, "hello.txt");
        assert_eq!(m.original_size, data.len() as u64);
        assert_eq!(m.created_ts, 1_000_000);
    }

    #[test]
    fn roundtrip_empty() {
        let key = generate_vault_key();
        let mut buf = Vec::new();
        write_pv(&mut buf, &key, &meta("empty.bin", 0), b"").unwrap();

        let (_, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn roundtrip_exactly_one_chunk() {
        let key = generate_vault_key();
        let data = vec![0xAAu8; CHUNK_SIZE];
        let mut buf = Vec::new();
        write_pv(&mut buf, &key, &meta("chunk.bin", CHUNK_SIZE as u64), &data).unwrap();

        let (_, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn roundtrip_multi_chunk() {
        let key = generate_vault_key();
        // 3 chunks + partial
        let data: Vec<u8> = (0..220_000).map(|i| (i % 251) as u8).collect();
        let mut buf = Vec::new();
        write_pv(&mut buf, &key, &meta("big.bin", data.len() as u64), &data).unwrap();

        let (_, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn metadata_only_read() {
        let key = generate_vault_key();
        let mut buf = Vec::new();
        write_pv(&mut buf, &key, &meta("photo.jpg", 99), b"fake jpeg").unwrap();

        let m = read_pv_metadata(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(m.original_name, "photo.jpg");
        assert_eq!(m.original_size, 99);
    }

    #[test]
    fn wrong_key_fails() {
        let k1 = generate_vault_key();
        let k2 = generate_vault_key();
        let mut buf = Vec::new();
        write_pv(&mut buf, &k1, &meta("f.bin", 4), b"test").unwrap();

        assert!(read_pv(&mut Cursor::new(&buf), &k2).is_err());
    }

    #[test]
    fn invalid_magic_fails() {
        let key = generate_vault_key();
        let buf = vec![0u8; 64];
        assert!(read_pv(&mut Cursor::new(&buf), &key).is_err());
    }

    #[test]
    fn tampered_chunk_fails() {
        let key = generate_vault_key();
        let mut buf = Vec::new();
        write_pv(&mut buf, &key, &meta("f.bin", 5), b"hello").unwrap();

        // Flip a byte near the end (inside encrypted chunk)
        let last = buf.len() - 1;
        buf[last] ^= 0xFF;

        assert!(read_pv(&mut Cursor::new(&buf), &key).is_err());
    }
}
