//! Segment storage: groups many items' `.pv` envelopes (see `pv_format.rs`,
//! unchanged by this module) into a small number of shared container files
//! instead of one file per item. A `VaultFileEntry` locates its envelope by
//! `(segment_index, offset, length)` rather than owning a dedicated file.
//!
//! `chunk` (pv_format.rs) is the logical/encryption unit; `segment` is the
//! physical storage unit — changing segment size never touches ciphertext,
//! it only moves already-encrypted bytes around (see `repack`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::crypto::VaultKey;
use crate::error::{Result, VaultError};
use crate::meta::{ActiveSegmentInfo, SegmentEntry, VaultFileEntry, VaultMeta};
use crate::pv_format::{read_pv_body, read_pv_metadata, write_pv, JobControl, PvMetadata, CHUNK_SIZE};

/// AEAD nonce (12) + ciphertext length-prefix (4) + AES-GCM tag (16) — the
/// fixed per-block overhead `write_pv` adds around every plaintext block
/// (the metadata block and each chunk alike). Mirrored here so the exact
/// envelope size can be predicted *before* writing (see `envelope_len`),
/// which `append_item`/`repack` need in order to decide whether an item
/// still fits in the active segment without writing it twice.
const BLOCK_OVERHEAD: u64 = 12 + 4 + 16;

/// The size `write_pv` will produce for `metadata` + `source` of
/// `metadata.original_size` bytes, computed without touching the source
/// stream or the key. Kept in sync with `pv_format::write_pv`'s layout:
/// `[magic:4][version:1]` + `[meta envelope]` + `[chunk_count:4]` + per-chunk
/// envelopes.
fn envelope_len(metadata: &PvMetadata) -> Result<u64> {
    let meta_json_len = serde_json::to_vec(metadata)?.len() as u64;
    let chunk_count = if metadata.original_size == 0 {
        0
    } else {
        (metadata.original_size - 1) / CHUNK_SIZE as u64 + 1
    };
    Ok(5 // magic + version
        + BLOCK_OVERHEAD + meta_json_len // metadata block
        + 4 // chunk_count
        + BLOCK_OVERHEAD * chunk_count + metadata.original_size) // chunk blocks
}

fn active_filename_for(target_segment_bytes: Option<u64>) -> &'static str {
    if target_segment_bytes.is_none() {
        "vault.pv"
    } else {
        "active.pv"
    }
}

/// `vault <index>.<cumulative GiB>.pv` — the human-readable sanity-check name
/// from the feature doc. Never parsed back as authoritative; the manifest
/// (`SegmentEntry`) is the source of truth for which file holds what.
fn closed_segment_filename(index: u32, cumulative_size: u64) -> String {
    let gib = cumulative_size.div_ceil(crate::meta::GIB).max(1);
    format!("vault {index}.{gib}.pv")
}

/// Resolves which physical file holds segment `index` — the active segment
/// or one of the closed ones. A free function (rather than a `SegmentStore`
/// method) so read-only callers (`Vault::export_file`/`read_metadata`/
/// `read_to_memory`, which only have `&self`) don't need a mutable borrow of
/// `VaultMeta` just to read an item back.
fn segment_path(vault_dir: &Path, meta: &VaultMeta, index: u32) -> Result<PathBuf> {
    if let Some(active) = &meta.active_segment {
        if active.index == index {
            return Ok(vault_dir.join(&active.filename));
        }
    }
    meta.segments
        .iter()
        .find(|s| s.index == index)
        .map(|s| vault_dir.join(&s.filename))
        .ok_or_else(|| VaultError::FileNotFound(format!("segment {index}")))
}

/// Bounded reader positioned at `entry`'s envelope — hand this straight to
/// `read_pv_metadata`/`read_pv_body`/`read_pv`. Read-only: see `segment_path`.
pub fn read_item(vault_dir: &Path, meta: &VaultMeta, entry: &VaultFileEntry) -> Result<impl Read> {
    let path = segment_path(vault_dir, meta, entry.segment_index)?;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    Ok(file.take(entry.length))
}

/// Phase of an in-progress repack, for UI display. Copying is cancellable
/// (nothing durable has changed yet); Verifying/Committing are not — they're
/// expected to be short relative to Copying, and the whole point of the
/// commit step is that it's the single atomic point of no return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepackPhase {
    Copying,
    Verifying,
    Committing,
}

impl RepackPhase {
    fn as_u8(self) -> u8 {
        match self {
            RepackPhase::Copying => 0,
            RepackPhase::Verifying => 1,
            RepackPhase::Committing => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => RepackPhase::Verifying,
            2 => RepackPhase::Committing,
            _ => RepackPhase::Copying,
        }
    }
}

/// Shared cancellation + progress + phase handle for a running repack — the
/// repack analogue of `pv_format::JobControl`, with an added phase so a
/// polling UI can show "Reorganizing…" vs "Verifying…" vs "Committing…".
#[derive(Default)]
pub struct RepackControl {
    pub cancel: AtomicBool,
    pub bytes_done: AtomicU64,
    phase: AtomicU8,
}

impl RepackControl {
    pub fn phase(&self) -> RepackPhase {
        RepackPhase::from_u8(self.phase.load(Ordering::Relaxed))
    }

    fn set_phase(&self, phase: RepackPhase) {
        self.phase.store(phase.as_u8(), Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn add_progress(&self, n: u64) {
        self.bytes_done.fetch_add(n, Ordering::Relaxed);
    }
}

/// Borrows a vault's directory + manifest for the duration of one
/// append/read/repack operation — never owns a separate copy of the segment
/// state, so it can't drift from `VaultMeta`.
pub struct SegmentStore<'a> {
    vault_dir: PathBuf,
    meta: &'a mut VaultMeta,
}

impl<'a> SegmentStore<'a> {
    pub fn new(vault_dir: PathBuf, meta: &'a mut VaultMeta) -> Self {
        Self { vault_dir, meta }
    }

    fn ensure_active_segment(&mut self) {
        if self.meta.active_segment.is_none() {
            let index = self.meta.segments.len() as u32 + 1;
            let filename = active_filename_for(self.meta.segment_settings.target_segment_bytes).to_string();
            self.meta.active_segment = Some(ActiveSegmentInfo {
                index,
                filename,
                bytes_written: 0,
            });
        }
    }

    /// Renames the active segment's file to its final closed name and
    /// records it in `segments`. Only ever called in segmented mode (single
    /// -file mode's active segment never finalizes) — see `append_item`.
    fn finalize_active_segment(&mut self) -> Result<()> {
        let active = self
            .meta
            .active_segment
            .take()
            .expect("finalize_active_segment called with no active segment");
        let prior_total: u64 = self.meta.segments.iter().map(|s| s.file_size).sum();
        let cumulative_size = prior_total + active.bytes_written;
        let filename = closed_segment_filename(active.index, cumulative_size);
        fs::rename(
            self.vault_dir.join(&active.filename),
            self.vault_dir.join(&filename),
        )?;
        self.meta.segments.push(SegmentEntry {
            index: active.index,
            filename,
            cumulative_size,
            file_size: active.bytes_written,
        });
        Ok(())
    }

    /// Writes `metadata`+`source` as a `.pv` envelope, appended to the vault's
    /// active segment (rolling to a fresh segment first if the item wouldn't
    /// fit under the target size) — never inline in an existing segment
    /// mid-file. Returns `(segment_index, offset, length)` to store on the
    /// new `VaultFileEntry`.
    pub fn append_item<R: Read>(
        &mut self,
        key: &VaultKey,
        metadata: &PvMetadata,
        source: R,
        control: &JobControl,
    ) -> Result<(u32, u64, u64)> {
        let item_len = envelope_len(metadata)?;

        self.ensure_active_segment();
        let target = self.meta.segment_settings.target_segment_bytes;
        let active = self.meta.active_segment.as_ref().unwrap();
        let would_exceed = matches!(target, Some(t) if active.bytes_written > 0 && active.bytes_written + item_len > t);
        if would_exceed {
            self.finalize_active_segment()?;
            self.ensure_active_segment();
        }

        let active = self.meta.active_segment.as_ref().unwrap();
        let path = self.vault_dir.join(&active.filename);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        // The offset is always the file's true on-disk length, never a
        // cached counter — a crash between a successful write and the next
        // `Vault::save()` can leave `bytes_written` stale, but the next
        // append still lands correctly after any orphaned trailing bytes.
        let offset = file.metadata()?.len();

        let write_result = write_pv(&mut file, key, metadata, source, control).and_then(|()| file.flush().map_err(VaultError::from));
        if let Err(e) = write_result {
            // Truncate back to the pre-write offset so a failed/cancelled
            // append doesn't leave a dangling partial envelope at the end of
            // a segment other entries may still be appended to later.
            let _ = file.set_len(offset);
            if offset == 0 {
                // This call created a brand-new segment that never held a
                // single successful item — remove the empty file and undo
                // the bootstrap, so a vault that only ever had failed/
                // cancelled appends leaves no trace on disk.
                drop(file);
                let _ = fs::remove_file(&path);
                self.meta.active_segment = None;
            }
            return Err(e);
        }

        let new_len = file.metadata()?.len();
        let length = new_len - offset;

        let active_mut = self.meta.active_segment.as_mut().unwrap();
        active_mut.bytes_written = new_len;

        Ok((active_mut.index, offset, length))
    }

    /// Bounded reader positioned at `entry`'s envelope — hand this straight
    /// to `read_pv_metadata`/`read_pv_body`/`read_pv`.
    pub fn read_item(&self, entry: &VaultFileEntry) -> Result<impl Read> {
        read_item(&self.vault_dir, self.meta, entry)
    }

    /// Reorganizes the physical segment layout to target `new_target_bytes`,
    /// without ever decrypting/re-encrypting an item — every byte moved is a
    /// raw copy of an existing ciphertext envelope. Crash-safe by
    /// construction: everything durable happens in a temp dir first, and the
    /// live vault (`VaultMeta` + segment files) is only touched in the final
    /// commit step, so any earlier interruption (cancel, crash, power loss)
    /// leaves the original vault completely untouched.
    pub fn repack(
        &mut self,
        new_target_bytes: Option<u64>,
        reclaim: bool,
        key: &VaultKey,
        control: &RepackControl,
    ) -> Result<()> {
        if !reclaim && new_target_bytes == self.meta.segment_settings.target_segment_bytes {
            return Ok(());
        }

        let total_len: u64 = self.meta.files.iter().map(|f| f.length).sum();
        let available = fs4::available_space(&self.vault_dir)?;
        let margin = total_len / 10;
        if available < total_len.saturating_add(margin) {
            return Err(VaultError::InsufficientDiskSpace);
        }

        let tmp_dir = self.vault_dir.join(".repack_tmp");
        if tmp_dir.exists() {
            fs::remove_dir_all(&tmp_dir)?;
        }
        fs::create_dir_all(&tmp_dir)?;

        control.set_phase(RepackPhase::Copying);
        let result = self.repack_copy_and_verify(new_target_bytes, &tmp_dir, key, control);
        let (new_segments, new_active, new_entries) = match result {
            Ok(built) => built,
            Err(e) => {
                let _ = fs::remove_dir_all(&tmp_dir);
                return Err(e);
            }
        };

        control.set_phase(RepackPhase::Committing);
        self.commit_repack(new_target_bytes, new_segments, new_active, new_entries, &tmp_dir)?;
        let _ = fs::remove_dir_all(&tmp_dir);
        Ok(())
    }

    /// Copies every live item into a fresh layout under `tmp_dir`, then
    /// fully re-decrypts every item from that new layout to verify it. Never
    /// mutates `self.meta` or any file outside `tmp_dir` — see `repack`.
    #[allow(clippy::type_complexity)]
    fn repack_copy_and_verify(
        &self,
        new_target_bytes: Option<u64>,
        tmp_dir: &Path,
        key: &VaultKey,
        control: &RepackControl,
    ) -> Result<(Vec<SegmentEntry>, Option<ActiveSegmentInfo>, Vec<(String, u32, u64, u64)>)> {
        let mut new_segments: Vec<SegmentEntry> = Vec::new();
        let mut new_active: Option<ActiveSegmentInfo> = None;
        let mut new_entries: Vec<(String, u32, u64, u64)> = Vec::new();

        // Snapshot of the OLD manifest to read from — untouched throughout.
        let entries: Vec<VaultFileEntry> = self.meta.files.clone();

        for entry in &entries {
            if control.is_cancelled() {
                return Err(VaultError::Cancelled);
            }
            let item_len = entry.length;

            let would_exceed = matches!(new_target_bytes, Some(t) if new_active.as_ref().is_some_and(|a| a.bytes_written > 0 && a.bytes_written + item_len > t));
            if would_exceed {
                finalize_into_tmp(tmp_dir, &mut new_segments, new_active.take().unwrap())?;
            }
            if new_active.is_none() {
                let index = new_segments.len() as u32 + 1;
                new_active = Some(ActiveSegmentInfo {
                    index,
                    filename: active_filename_for(new_target_bytes).to_string(),
                    bytes_written: 0,
                });
            }

            let active = new_active.as_mut().unwrap();
            let dest_path = tmp_dir.join(&active.filename);
            let mut dest = OpenOptions::new().create(true).append(true).open(&dest_path)?;
            let offset = dest.metadata()?.len();

            let mut src = self.read_item(entry)?;
            let copied = io::copy(&mut src, &mut dest)?;
            control.add_progress(copied);

            active.bytes_written = offset + copied;
            new_entries.push((entry.id.clone(), active.index, offset, copied));
        }

        control.set_phase(RepackPhase::Verifying);
        for (_, seg_index, offset, length) in &new_entries {
            let filename = if new_active.as_ref().map(|a| a.index) == Some(*seg_index) {
                new_active.as_ref().unwrap().filename.clone()
            } else {
                new_segments
                    .iter()
                    .find(|s| s.index == *seg_index)
                    .expect("segment index just written must be present")
                    .filename
                    .clone()
            };
            let mut f = File::open(tmp_dir.join(&filename))?;
            f.seek(SeekFrom::Start(*offset))?;
            let mut bounded = f.take(*length);
            read_pv_metadata(&mut bounded, key)?;
            read_pv_body(&mut bounded, key, &mut io::sink(), &JobControl::default())?;
        }

        Ok((new_segments, new_active, new_entries))
    }

    /// The single atomic point of no return: moves the new layout's files
    /// into place, removes whatever old segment files they didn't overwrite,
    /// and only then updates + saves the manifest.
    fn commit_repack(
        &mut self,
        new_target_bytes: Option<u64>,
        new_segments: Vec<SegmentEntry>,
        new_active: Option<ActiveSegmentInfo>,
        new_entries: Vec<(String, u32, u64, u64)>,
        tmp_dir: &Path,
    ) -> Result<()> {
        let mut kept_filenames: Vec<&str> = new_segments.iter().map(|s| s.filename.as_str()).collect();
        if let Some(a) = &new_active {
            kept_filenames.push(a.filename.as_str());
        }

        for entry in fs::read_dir(tmp_dir)? {
            let entry = entry?;
            let dest = self.vault_dir.join(entry.file_name());
            fs::rename(entry.path(), dest)?;
        }

        let mut old_filenames: Vec<String> = self.meta.segments.iter().map(|s| s.filename.clone()).collect();
        if let Some(a) = &self.meta.active_segment {
            old_filenames.push(a.filename.clone());
        }
        for filename in old_filenames {
            if !kept_filenames.contains(&filename.as_str()) {
                let _ = fs::remove_file(self.vault_dir.join(&filename));
            }
        }

        self.meta.segment_settings.target_segment_bytes = new_target_bytes;
        self.meta.segments = new_segments;
        self.meta.active_segment = new_active;
        for (id, segment_index, offset, length) in new_entries {
            if let Some(f) = self.meta.files.iter_mut().find(|f| f.id == id) {
                f.segment_index = segment_index;
                f.offset = offset;
                f.length = length;
            }
        }

        Ok(())
    }
}

fn finalize_into_tmp(tmp_dir: &Path, segments: &mut Vec<SegmentEntry>, active: ActiveSegmentInfo) -> Result<()> {
    let prior_total: u64 = segments.iter().map(|s| s.file_size).sum();
    let cumulative_size = prior_total + active.bytes_written;
    let filename = closed_segment_filename(active.index, cumulative_size);
    fs::rename(tmp_dir.join(&active.filename), tmp_dir.join(&filename))?;
    segments.push(SegmentEntry {
        index: active.index,
        filename,
        cumulative_size,
        file_size: active.bytes_written,
    });
    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_vault_key;
    use crate::meta::VaultMeta;
    use tempfile::TempDir;

    fn item(name: &str, size: u64) -> PvMetadata {
        PvMetadata {
            original_name: name.to_string(),
            original_size: size,
            created_ts: 1,
            modified_ts: 2,
            mime_type: "application/octet-stream".to_string(),
        }
    }

    fn new_meta() -> VaultMeta {
        VaultMeta::create_new("pw").unwrap().0
    }

    #[test]
    fn append_and_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = new_meta();
        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);

        let data = b"hello segmented world";
        let (idx, offset, length) = store
            .append_item(&key, &item("a.txt", data.len() as u64), &data[..], &JobControl::default())
            .unwrap();
        assert_eq!(idx, 1);
        assert_eq!(offset, 0);

        let entry = VaultFileEntry {
            id: "x".into(),
            folder_id: None,
            segment_index: idx,
            offset,
            length,
        };
        let mut reader = store.read_item(&entry).unwrap();
        let (m, out) = crate::pv_format::read_pv(&mut reader, &key).unwrap();
        assert_eq!(out, data);
        assert_eq!(m.original_name, "a.txt");
    }

    #[test]
    fn second_item_appends_after_first_in_same_segment() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = new_meta();
        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);

        let (idx1, off1, len1) = store
            .append_item(&key, &item("a.txt", 5), &b"AAAAA"[..], &JobControl::default())
            .unwrap();
        let (idx2, off2, _len2) = store
            .append_item(&key, &item("b.txt", 5), &b"BBBBB"[..], &JobControl::default())
            .unwrap();

        assert_eq!(idx1, idx2);
        assert_eq!(off2, off1 + len1);
    }

    #[test]
    fn segment_rolls_over_at_target_boundary() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = new_meta();
        meta.segment_settings.target_segment_bytes = Some(200); // tiny, forces rollover
        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);

        let mut segment_indices = Vec::new();
        for i in 0..10 {
            let data = vec![i as u8; 50];
            let (idx, _, _) = store
                .append_item(&key, &item("f.bin", 50), data.as_slice(), &JobControl::default())
                .unwrap();
            segment_indices.push(idx);
        }

        assert!(segment_indices.iter().max().unwrap() > &1, "expected at least one rollover");
        assert!(meta.segments.len() >= 1, "earlier segments should have been finalized");
        assert!(meta.active_segment.is_some());
    }

    #[test]
    fn oversized_single_item_gets_its_own_segment_without_growing_further() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = new_meta();
        meta.segment_settings.target_segment_bytes = Some(100);
        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);

        // First a small item, then one bigger than the whole target.
        store
            .append_item(&key, &item("small.bin", 10), &vec![0u8; 10][..], &JobControl::default())
            .unwrap();
        let big_data = vec![1u8; 500];
        let (idx, _, length) = store
            .append_item(&key, &item("big.bin", 500), big_data.as_slice(), &JobControl::default())
            .unwrap();

        // The oversized item forced a rollover into its own segment (not
        // appended after "small.bin" in an already-near-full segment).
        assert!(length > 100);
        assert_eq!(meta.segments.len(), 1); // "small.bin"'s segment got finalized
        assert_eq!(meta.active_segment.as_ref().unwrap().index, idx);
    }

    #[test]
    fn single_file_mode_never_finalizes() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = new_meta();
        meta.segment_settings.target_segment_bytes = None;
        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);

        for _ in 0..20 {
            store
                .append_item(&key, &item("f.bin", 1_000_000), vec![0u8; 1_000_000].as_slice(), &JobControl::default())
                .unwrap();
        }

        assert!(meta.segments.is_empty());
        assert_eq!(meta.active_segment.as_ref().unwrap().filename, "vault.pv");
    }

    fn build_test_vault(dir: &Path, key: &VaultKey, target: Option<u64>, count: usize) -> VaultMeta {
        let mut meta = new_meta();
        meta.segment_settings.target_segment_bytes = target;
        {
            let mut store = SegmentStore::new(dir.to_path_buf(), &mut meta);
            for i in 0..count {
                let data = vec![i as u8; 300];
                let (idx, offset, length) = store
                    .append_item(key, &item(&format!("f{i}.bin"), 300), data.as_slice(), &JobControl::default())
                    .unwrap();
                store.meta.files.push(VaultFileEntry {
                    id: format!("id{i}"),
                    folder_id: None,
                    segment_index: idx,
                    offset,
                    length,
                });
            }
        }
        meta
    }

    #[test]
    fn repack_preserves_all_items_under_new_target() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = build_test_vault(dir.path(), &key, Some(200), 15);

        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
        store.repack(Some(1000), false, &key, &RepackControl::default()).unwrap();

        assert_eq!(meta.segment_settings.target_segment_bytes, Some(1000));
        for i in 0..15 {
            let entry = meta.files.iter().find(|f| f.id == format!("id{i}")).unwrap();
            let mut reader = SegmentStore::new(dir.path().to_path_buf(), &mut meta.clone())
                .read_item(entry)
                .unwrap();
            let (_, out) = crate::pv_format::read_pv(&mut reader, &key).unwrap();
            assert_eq!(out, vec![i as u8; 300]);
        }
    }

    #[test]
    fn repack_to_single_file_then_back() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = build_test_vault(dir.path(), &key, Some(200), 10);

        {
            let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
            store.repack(None, false, &key, &RepackControl::default()).unwrap();
        }
        assert!(meta.segments.is_empty());
        assert_eq!(meta.active_segment.as_ref().unwrap().filename, "vault.pv");

        {
            let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
            store.repack(Some(150), false, &key, &RepackControl::default()).unwrap();
        }
        assert!(!meta.segments.is_empty());

        for i in 0..10 {
            let entry = meta.files.iter().find(|f| f.id == format!("id{i}")).unwrap().clone();
            let store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
            let mut reader = store.read_item(&entry).unwrap();
            let (_, out) = crate::pv_format::read_pv(&mut reader, &key).unwrap();
            assert_eq!(out, vec![i as u8; 300]);
        }
    }

    #[test]
    fn repack_noop_when_target_unchanged() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = build_test_vault(dir.path(), &key, Some(200), 5);
        let segments_before = meta.segments.clone();

        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
        store.repack(Some(200), false, &key, &RepackControl::default()).unwrap();

        assert_eq!(meta.segments.len(), segments_before.len());
    }

    #[test]
    fn repack_reclaim_recompacts_even_with_unchanged_target() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = build_test_vault(dir.path(), &key, Some(200), 5);
        // Delete a couple of entries to create dead space, then reclaim.
        meta.files.retain(|f| f.id != "id0" && f.id != "id1");

        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
        store.repack(Some(200), true, &key, &RepackControl::default()).unwrap();

        assert_eq!(meta.files.len(), 3);
        for entry in meta.files.clone() {
            let mut reader = SegmentStore::new(dir.path().to_path_buf(), &mut meta).read_item(&entry).unwrap();
            assert!(crate::pv_format::read_pv(&mut reader, &key).is_ok());
        }
    }

    #[test]
    fn repack_cancelled_leaves_original_untouched() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = build_test_vault(dir.path(), &key, Some(200), 10);
        let segments_before = meta.segments.clone();
        let files_before = meta.files.clone();

        let control = RepackControl::default();
        control.cancel.store(true, Ordering::Relaxed);
        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
        let err = store.repack(Some(1000), false, &key, &control).unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));

        assert_eq!(meta.segments.len(), segments_before.len());
        assert_eq!(meta.files.len(), files_before.len());
        assert!(!dir.path().join(".repack_tmp").exists());

        // Original data must still be fully readable.
        for entry in meta.files.clone() {
            let mut reader = SegmentStore::new(dir.path().to_path_buf(), &mut meta).read_item(&entry).unwrap();
            assert!(crate::pv_format::read_pv(&mut reader, &key).is_ok());
        }
    }

    #[test]
    fn repack_empty_vault() {
        let dir = TempDir::new().unwrap();
        let key = generate_vault_key();
        let mut meta = new_meta();
        meta.segment_settings.target_segment_bytes = Some(200);

        let mut store = SegmentStore::new(dir.path().to_path_buf(), &mut meta);
        store.repack(Some(1000), false, &key, &RepackControl::default()).unwrap();

        assert!(meta.files.is_empty());
        assert!(meta.segments.is_empty());
    }
}
