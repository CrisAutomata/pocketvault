//! .pv file format
//!
//! Layout:
//!   [magic: 4]  [version: 1]
//!   [meta_nonce: 12]  [meta_len: 4 LE]  [enc_metadata: meta_len]
//!   [chunk_count: 4 LE]
//!   for each chunk:
//!     [chunk_nonce: 12]  [chunk_len: 4 LE]  [enc_chunk: chunk_len]

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Streams `source` through in `CHUNK_SIZE` pieces rather than buffering the
/// whole file in memory first — the chunk count is derived from
/// `metadata.original_size` up front so the format (chunk-count-then-chunks)
/// stays unchanged for a caller that only has a `Read` stream, not a slice.
pub fn write_pv<W: Write, R: Read>(
    writer: &mut W,
    key: &VaultKey,
    metadata: &PvMetadata,
    mut source: R,
    cancel: &AtomicBool,
) -> Result<()> {
    writer.write_all(MAGIC)?;
    writer.write_all(&[VERSION])?;

    let meta_json = serde_json::to_vec(metadata)?;
    let (enc_meta, meta_nonce) = encrypt(key, &meta_json)?;
    writer.write_all(&meta_nonce)?;
    writer.write_all(&(enc_meta.len() as u32).to_le_bytes())?;
    writer.write_all(&enc_meta)?;

    let chunk_count = if metadata.original_size == 0 {
        0
    } else {
        (metadata.original_size - 1) / CHUNK_SIZE as u64 + 1
    } as u32;
    writer.write_all(&chunk_count.to_le_bytes())?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    for _ in 0..chunk_count {
        if cancel.load(Ordering::Relaxed) {
            return Err(VaultError::Cancelled);
        }
        let filled = read_up_to(&mut source, &mut buf)?;
        let (enc_chunk, chunk_nonce) = encrypt(key, &buf[..filled])?;
        writer.write_all(&chunk_nonce)?;
        writer.write_all(&(enc_chunk.len() as u32).to_le_bytes())?;
        writer.write_all(&enc_chunk)?;
    }

    Ok(())
}

/// Fills `buf` from `source`, stopping early only at EOF (a single `Read::read`
/// call may return fewer bytes than requested even mid-stream).
fn read_up_to<R: Read>(source: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match source.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
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

/// Reads and decrypts the chunk section into `writer`, one `CHUNK_SIZE` piece
/// at a time — the caller must have already consumed the header + metadata
/// (e.g. via `read_pv_metadata`) so the reader is positioned at the chunk
/// count. Used to export/preview without buffering the whole file in memory.
pub fn read_pv_body<R: Read, W: Write>(
    reader: &mut R,
    key: &VaultKey,
    writer: &mut W,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut count_buf = [0u8; 4];
    reader.read_exact(&mut count_buf)?;
    let chunk_count = u32::from_le_bytes(count_buf) as usize;

    for _ in 0..chunk_count {
        if cancel.load(Ordering::Relaxed) {
            return Err(VaultError::Cancelled);
        }
        let mut chunk_nonce = [0u8; NONCE_SIZE];
        reader.read_exact(&mut chunk_nonce)?;

        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut enc_chunk = vec![0u8; len];
        reader.read_exact(&mut enc_chunk)?;

        let chunk = decrypt(key, &enc_chunk, &chunk_nonce)?;
        writer.write_all(&chunk)?;
    }

    Ok(())
}

pub fn read_pv<R: Read>(reader: &mut R, key: &VaultKey) -> Result<(PvMetadata, Vec<u8>)> {
    let metadata = read_pv_metadata(reader, key)?;
    let mut plaintext = Vec::with_capacity(metadata.original_size as usize);
    read_pv_body(reader, key, &mut plaintext, &AtomicBool::new(false))?;
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
        write_pv(
            &mut buf,
            &key,
            &meta("hello.txt", data.len() as u64),
            &data[..],
            &AtomicBool::new(false),
        )
        .unwrap();

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
        write_pv(
            &mut buf,
            &key,
            &meta("empty.bin", 0),
            &b""[..],
            &AtomicBool::new(false),
        )
        .unwrap();

        let (_, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn roundtrip_exactly_one_chunk() {
        let key = generate_vault_key();
        let data = vec![0xAAu8; CHUNK_SIZE];
        let mut buf = Vec::new();
        write_pv(
            &mut buf,
            &key,
            &meta("chunk.bin", CHUNK_SIZE as u64),
            data.as_slice(),
            &AtomicBool::new(false),
        )
        .unwrap();

        let (_, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn roundtrip_multi_chunk() {
        let key = generate_vault_key();
        // 3 chunks + partial
        let data: Vec<u8> = (0..220_000).map(|i| (i % 251) as u8).collect();
        let mut buf = Vec::new();
        write_pv(
            &mut buf,
            &key,
            &meta("big.bin", data.len() as u64),
            data.as_slice(),
            &AtomicBool::new(false),
        )
        .unwrap();

        let (_, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn metadata_only_read() {
        let key = generate_vault_key();
        let mut buf = Vec::new();
        write_pv(
            &mut buf,
            &key,
            &meta("photo.jpg", 99),
            &b"fake jpeg"[..],
            &AtomicBool::new(false),
        )
        .unwrap();

        let m = read_pv_metadata(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(m.original_name, "photo.jpg");
        assert_eq!(m.original_size, 99);
    }

    #[test]
    fn wrong_key_fails() {
        let k1 = generate_vault_key();
        let k2 = generate_vault_key();
        let mut buf = Vec::new();
        write_pv(
            &mut buf,
            &k1,
            &meta("f.bin", 4),
            &b"test"[..],
            &AtomicBool::new(false),
        )
        .unwrap();

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
        write_pv(
            &mut buf,
            &key,
            &meta("f.bin", 5),
            &b"hello"[..],
            &AtomicBool::new(false),
        )
        .unwrap();

        // Flip a byte near the end (inside encrypted chunk)
        let last = buf.len() - 1;
        buf[last] ^= 0xFF;

        assert!(read_pv(&mut Cursor::new(&buf), &key).is_err());
    }

    /// A `Read` that asserts it's never asked to fill more than `CHUNK_SIZE`
    /// bytes in one call, proving `write_pv` streams the source rather than
    /// buffering it whole (the bug behind "crash on big file" imports).
    struct BoundedReader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl<'a> Read for BoundedReader<'a> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            assert!(
                buf.len() <= CHUNK_SIZE,
                "write_pv requested {} bytes in one read — it must never exceed CHUNK_SIZE ({})",
                buf.len(),
                CHUNK_SIZE
            );
            let remaining = &self.data[self.pos..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn write_pv_streams_source_in_bounded_chunks() {
        let key = generate_vault_key();
        // Several chunks' worth — large enough that buffering it all at once
        // (the old behavior) vs. reading it in CHUNK_SIZE pieces would differ.
        let data: Vec<u8> = (0..(CHUNK_SIZE * 4 + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        let reader = BoundedReader {
            data: &data,
            pos: 0,
        };

        let mut buf = Vec::new();
        write_pv(
            &mut buf,
            &key,
            &meta("big.bin", data.len() as u64),
            reader,
            &AtomicBool::new(false),
        )
        .unwrap();

        let (m, out) = read_pv(&mut Cursor::new(&buf), &key).unwrap();
        assert_eq!(out, data);
        assert_eq!(m.original_size, data.len() as u64);
    }

    #[test]
    fn write_pv_stops_at_cancellation() {
        let key = generate_vault_key();
        let data = vec![0xABu8; CHUNK_SIZE * 4];
        let cancel = AtomicBool::new(true);

        let mut buf = Vec::new();
        let err = write_pv(
            &mut buf,
            &key,
            &meta("f.bin", data.len() as u64),
            data.as_slice(),
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));
    }

    #[test]
    fn read_pv_body_stops_at_cancellation() {
        let key = generate_vault_key();
        let data = vec![0xCDu8; CHUNK_SIZE * 4];
        let mut buf = Vec::new();
        write_pv(
            &mut buf,
            &key,
            &meta("f.bin", data.len() as u64),
            data.as_slice(),
            &AtomicBool::new(false),
        )
        .unwrap();

        let mut reader = Cursor::new(&buf);
        let _metadata = read_pv_metadata(&mut reader, &key).unwrap();
        let mut out = Vec::new();
        let err = read_pv_body(&mut reader, &key, &mut out, &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));
    }
}
