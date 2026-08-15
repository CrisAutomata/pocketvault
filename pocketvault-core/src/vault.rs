use std::{
    fs::{self, File, TryLockError},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    crypto::VaultKey,
    error::{Result, VaultError},
    meta::{VaultFileEntry, VaultFolder, VaultMeta},
    pv_format::{read_pv, read_pv_body, read_pv_metadata, JobControl, PvMetadata},
    segment::{RepackControl, SegmentStore},
};

/// Cross-process advisory lock on `vault.lock`, held for as long as any clone
/// of this `Vault` is alive (shared via `Arc`, not re-acquired per clone) —
/// segments are shared, appended-to files, unlike the old one-file-per-item
/// layout, so two processes opening the same vault concurrently could
/// otherwise corrupt them.
#[derive(Debug)]
pub struct VaultLock(#[allow(dead_code)] File);

fn acquire_lock(base_dir: &Path) -> Result<Arc<VaultLock>> {
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(base_dir.join("vault.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Arc::new(VaultLock(file))),
        Err(TryLockError::WouldBlock) => Err(VaultError::VaultLockedByAnotherProcess),
        Err(TryLockError::Error(e)) => Err(VaultError::Io(e)),
    }
}

#[derive(Debug, Clone)]
pub struct Vault {
    pub base_dir: PathBuf,
    pub meta: VaultMeta,
    /// Held only for its `Drop` impl (releases the advisory lock) — never
    /// read, so `#[allow(dead_code)]` rather than a false "unused field".
    #[allow(dead_code)]
    lock: Arc<VaultLock>,
}

impl Vault {
    pub fn create(base_dir: &Path, password: &str) -> Result<Vault> {
        let meta_path = base_dir.join("vault.meta");
        if meta_path.exists() {
            return Err(VaultError::VaultAlreadyExists);
        }

        fs::create_dir_all(base_dir.join("vault"))?;
        let lock = acquire_lock(base_dir)?;

        let (meta, _dek) = VaultMeta::create_new(password)?;
        let json = serde_json::to_string_pretty(&meta)?;
        fs::write(&meta_path, json)?;

        Ok(Vault {
            base_dir: base_dir.to_path_buf(),
            meta,
            lock,
        })
    }

    pub fn open(base_dir: &Path) -> Result<Vault> {
        let meta_path = base_dir.join("vault.meta");
        if !meta_path.exists() {
            return Err(VaultError::VaultNotFound);
        }
        let lock = acquire_lock(base_dir)?;
        let json = fs::read_to_string(&meta_path)?;
        let meta: VaultMeta = serde_json::from_str(&json)?;
        Ok(Vault {
            base_dir: base_dir.to_path_buf(),
            meta,
            lock,
        })
    }

    pub fn exists(base_dir: &Path) -> bool {
        base_dir.join("vault.meta").exists()
    }

    // ── Paths ───────────────────────────────────────────────────────────────

    pub fn vault_dir(&self) -> PathBuf {
        self.base_dir.join("vault")
    }

    // ── Persistence ─────────────────────────────────────────────────────────

    /// Writes to a temp file in the vault directory and renames it over
    /// `vault.meta` — a crash mid-write can't corrupt the manifest, unlike a
    /// plain `fs::write`. Now higher-stakes than before segmentation: this
    /// file is the only map to every item's location inside shared segments.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.meta)?;
        let tmp_path = self.base_dir.join("vault.meta.tmp");
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, self.base_dir.join("vault.meta"))?;
        Ok(())
    }

    /// Reorganizes the physical segment layout to `new_target_bytes` (see
    /// `SegmentSettings`), without ever decrypting/re-encrypting an item.
    /// `reclaim`, when true, forces a repack (compacting dead space from
    /// deletes) even if the target size isn't changing.
    pub fn repack(
        &mut self,
        new_target_bytes: Option<u64>,
        reclaim: bool,
        key: &VaultKey,
        control: &RepackControl,
    ) -> Result<()> {
        let mut store = SegmentStore::new(self.vault_dir(), &mut self.meta);
        store.repack(new_target_bytes, reclaim, key, control)?;
        self.save()
    }

    // ── Folder operations ────────────────────────────────────────────────────

    pub fn create_folder(&mut self, name: &str, parent_id: Option<&str>) -> Result<String> {
        let id = self.create_folder_inner(name, parent_id);
        self.save()?;
        Ok(id)
    }

    /// Same as `create_folder` but without the persist — used by batch/recursive
    /// callers (`encrypt_folder_contents`) that save once at the end of the whole
    /// operation instead of once per subfolder created.
    fn create_folder_inner(&mut self, name: &str, parent_id: Option<&str>) -> String {
        self.meta.add_folder(name, parent_id)
    }

    pub fn rename_folder(&mut self, folder_id: &str, new_name: &str) -> Result<()> {
        if !self.meta.rename_folder(folder_id, new_name) {
            return Err(VaultError::FileNotFound(folder_id.to_string()));
        }
        self.save()?;
        Ok(())
    }

    /// Deletes `folder_id` and its whole subtree, returning the ids of every
    /// file that was removed (so a caller can prune any cached metadata for
    /// them — a folder delete can remove many files, not just direct children).
    ///
    /// Only drops the manifest entries — see `delete_file_inner` for why the
    /// underlying bytes aren't reclaimed until a repack.
    pub fn delete_folder(&mut self, folder_id: &str) -> Result<Vec<String>> {
        let removed = self.meta.remove_folder(folder_id);
        self.save()?;
        Ok(removed.into_iter().map(|f| f.id).collect())
    }

    pub fn folders(&self, parent_id: Option<&str>) -> Vec<&VaultFolder> {
        self.meta.subfolders(parent_id)
    }

    pub fn all_folders(&self) -> &[VaultFolder] {
        &self.meta.folders
    }

    // ── File operations ──────────────────────────────────────────────────────

    pub fn encrypt_file(
        &mut self,
        source: &Path,
        folder_id: Option<&str>,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<String> {
        let id = self.encrypt_file_inner(source, folder_id, key, control)?;
        self.save()?;
        Ok(id)
    }

    /// Same as `encrypt_file` but without the persist — used by batch/recursive
    /// callers (`encrypt_files`, `encrypt_folder_contents`) that save once at the
    /// end of the whole operation instead of once per file. Rewriting the entire
    /// vault.meta JSON after every single file was the dominant cost — and the
    /// source of wildly wrong ETA estimates — for folders with many files.
    fn encrypt_file_inner(
        &mut self,
        source: &Path,
        folder_id: Option<&str>,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<String> {
        let filename = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| VaultError::FileNotFound(source.to_string_lossy().to_string()))?;

        let source_meta = fs::metadata(source)?;
        let modified_ts = source_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(now_ts());

        let pv_meta = PvMetadata {
            original_name: filename.to_string(),
            original_size: source_meta.len(),
            created_ts: now_ts(),
            modified_ts,
            mime_type: mime_for(filename),
        };

        // Streams the source through in fixed-size chunks (see `write_pv`)
        // instead of reading the whole file into memory first — a multi-GB
        // import previously allocated a same-sized `Vec<u8>` up front, which
        // could exhaust memory and abort the process. The envelope is
        // appended into the vault's active segment rather than a dedicated
        // file — see `SegmentStore::append_item`.
        let source_file = BufReader::new(fs::File::open(source)?);
        let mut store = SegmentStore::new(self.vault_dir(), &mut self.meta);
        let (segment_index, offset, length) =
            store.append_item(key, &pv_meta, source_file, control)?;

        let id = self.meta.add_file(segment_index, offset, length, folder_id);

        Ok(id)
    }

    /// Encrypts every path in `paths` into `folder_id` as one all-or-nothing
    /// batch: if any file fails (including cancellation), every file already
    /// added earlier in this same batch is rolled back. Persists once at the
    /// end (success or rollback) rather than once per file.
    pub fn encrypt_files(
        &mut self,
        paths: &[PathBuf],
        folder_id: Option<&str>,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<Vec<String>> {
        let mut ids: Vec<String> = Vec::new();
        for path in paths {
            if control.cancel.load(Ordering::Relaxed) {
                for id in &ids {
                    let _ = self.delete_file_inner(id);
                }
                let _ = self.save();
                return Err(VaultError::Cancelled);
            }
            match self.encrypt_file_inner(path, folder_id, key, control) {
                Ok(id) => ids.push(id),
                Err(e) => {
                    for id in &ids {
                        let _ = self.delete_file_inner(id);
                    }
                    let _ = self.save();
                    return Err(e);
                }
            }
        }
        self.save()?;
        Ok(ids)
    }

    /// Recursively encrypts every file under `source_dir` into the vault, creating
    /// a matching vault folder for `source_dir` itself and for each subdirectory
    /// (even ones with no files) so the original folder structure is preserved.
    ///
    /// On any error — including cancellation — everything created by this call
    /// (the root folder plus its whole subtree) is rolled back before the error
    /// is returned, so a failed/cancelled job leaves the vault exactly as it was.
    pub fn encrypt_folder(
        &mut self,
        source_dir: &Path,
        parent_folder_id: Option<&str>,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<Vec<String>> {
        let name = source_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| VaultError::FileNotFound(source_dir.to_string_lossy().to_string()))?;
        let folder_id = self.create_folder_inner(name, parent_folder_id);

        match self.encrypt_folder_contents(source_dir, &folder_id, key, control) {
            Ok(ids) => {
                self.save()?;
                Ok(ids)
            }
            Err(e) => {
                // `delete_folder` persists its own rollback in a single save, so
                // the in-memory adds made by `encrypt_folder_contents` (which,
                // like `encrypt_files`, doesn't save per-entry) still end up
                // correctly reverted on disk here.
                let _ = self.delete_folder(&folder_id);
                Err(e)
            }
        }
    }

    /// Recurses without persisting — `encrypt_folder` saves once for the whole
    /// subtree (success) or relies on `delete_folder`'s single save (failure),
    /// instead of rewriting vault.meta after every file and every subfolder.
    fn encrypt_folder_contents(
        &mut self,
        source_dir: &Path,
        folder_id: &str,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<Vec<String>> {
        let mut entries: Vec<_> = fs::read_dir(source_dir)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());

        let mut file_ids = Vec::new();
        for entry in entries {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(VaultError::Cancelled);
            }
            let path = entry.path();
            if path.is_dir() {
                let sub_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| VaultError::FileNotFound(path.to_string_lossy().to_string()))?;
                let sub_id = self.create_folder_inner(sub_name, Some(folder_id));
                file_ids.extend(self.encrypt_folder_contents(&path, &sub_id, key, control)?);
            } else if path.is_file() {
                file_ids.push(self.encrypt_file_inner(&path, Some(folder_id), key, control)?);
            }
        }
        Ok(file_ids)
    }

    pub fn delete_file(&mut self, file_id: &str) -> Result<()> {
        self.delete_file_inner(file_id)?;
        self.save()?;
        Ok(())
    }

    /// Same as `delete_file` but without the persist — used by `encrypt_files`'s
    /// rollback path, which saves once after the whole rollback instead of once
    /// per file removed.
    ///
    /// Only drops the manifest entry — the item's bytes inside its segment
    /// are dead space until the next repack, not reclaimed immediately (see
    /// `SegmentStore::repack`'s `reclaim` option).
    fn delete_file_inner(&mut self, file_id: &str) -> Result<()> {
        self.meta
            .remove_file(file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;
        Ok(())
    }

    pub fn export_file(
        &self,
        file_id: &str,
        dest_dir: &Path,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<PathBuf> {
        let entry = self
            .meta
            .files
            .iter()
            .find(|f| f.id == file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let mut f = BufReader::new(crate::segment::read_item(&self.vault_dir(), &self.meta, entry)?);
        let meta = read_pv_metadata(&mut f, key)?;

        let dest = dest_dir.join(&meta.original_name);
        // Streams decrypted chunks straight to disk instead of buffering the
        // whole plaintext in memory first (same rationale as `encrypt_file`).
        // Buffered on both sides so each chunk's several small reads/writes
        // don't each cost a separate syscall.
        let mut out = BufWriter::new(fs::File::create(&dest)?);
        let read_result = read_pv_body(&mut f, key, &mut out, control)
            .and_then(|()| out.flush().map_err(VaultError::from));
        if let Err(e) = read_result {
            drop(out);
            let _ = fs::remove_file(&dest); // no partial export left behind on cancel/error
            return Err(e);
        }

        Ok(dest)
    }

    /// Recursively exports every file under `folder_id` to disk, recreating the
    /// vault folder's own subfolder structure under `dest_dir` (the reverse of
    /// `encrypt_folder`). Returns the created root directory.
    ///
    /// On any error — including cancellation — the destination directory created
    /// for this call is removed before the error is returned.
    pub fn export_folder(
        &self,
        folder_id: &str,
        dest_dir: &Path,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<PathBuf> {
        let folder = self
            .meta
            .folders
            .iter()
            .find(|f| f.id == folder_id)
            .ok_or_else(|| VaultError::FileNotFound(folder_id.to_string()))?;

        let out_dir = dest_dir.join(&folder.name);
        fs::create_dir_all(&out_dir)?;

        match self.export_folder_contents(folder_id, &out_dir, key, control) {
            Ok(()) => Ok(out_dir),
            Err(e) => {
                let _ = fs::remove_dir_all(&out_dir);
                Err(e)
            }
        }
    }

    fn export_folder_contents(
        &self,
        folder_id: &str,
        out_dir: &Path,
        key: &VaultKey,
        control: &JobControl,
    ) -> Result<()> {
        for file in self.meta.files_in_folder(Some(folder_id)) {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(VaultError::Cancelled);
            }
            self.export_file(&file.id, out_dir, key, control)?;
        }
        for sub in self.meta.subfolders(Some(folder_id)) {
            if control.cancel.load(Ordering::Relaxed) {
                return Err(VaultError::Cancelled);
            }
            self.export_folder(&sub.id, out_dir, key, control)?;
        }
        Ok(())
    }

    pub fn read_metadata(&self, file_id: &str, key: &VaultKey) -> Result<PvMetadata> {
        let entry = self
            .meta
            .files
            .iter()
            .find(|f| f.id == file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let mut f = crate::segment::read_item(&self.vault_dir(), &self.meta, entry)?;
        read_pv_metadata(&mut f, key)
    }

    pub fn read_to_memory(&self, file_id: &str, key: &VaultKey) -> Result<(PvMetadata, Vec<u8>)> {
        let entry = self
            .meta
            .files
            .iter()
            .find(|f| f.id == file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let mut f = BufReader::new(crate::segment::read_item(&self.vault_dir(), &self.meta, entry)?);
        read_pv(&mut f, key)
    }

    pub fn files_in_folder(&self, folder_id: Option<&str>) -> Vec<&VaultFileEntry> {
        self.meta.files_in_folder(folder_id)
    }
}

/// Recursive, stat-only (no reads) sum of every file's size under `dir` —
/// used to decide whether a folder is "big" before starting an encrypt job.
pub fn dir_total_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            total += dir_total_size(&path)?;
        } else if path.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn mime_for(name: &str) -> String {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "txt" | "md" | "log" | "csv" => "text/plain",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub fn is_previewable(mime: &str) -> bool {
    mime.starts_with("image/") || mime == "text/plain"
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn make_vault(dir: &Path) -> Vault {
        Vault::create(dir, "test_pass").unwrap()
    }

    fn unlock(v: &Vault) -> VaultKey {
        v.meta.unlock("test_pass").unwrap()
    }

    fn write_temp(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, data).unwrap();
        path
    }

    fn cancelled_control() -> JobControl {
        JobControl {
            cancel: AtomicBool::new(true),
            ..JobControl::default()
        }
    }

    // ── Create / open ────────────────────────────────────────────────────

    #[test]
    fn create_makes_vault_dir_and_meta() {
        let d = TempDir::new().unwrap();
        let _ = make_vault(d.path());
        assert!(d.path().join("vault.meta").exists());
        assert!(d.path().join("vault").is_dir());
    }

    #[test]
    fn open_existing_vault() {
        let d = TempDir::new().unwrap();
        let _ = make_vault(d.path());
        let v = Vault::open(d.path()).unwrap();
        assert!(v.meta.files.is_empty());
    }

    #[test]
    fn create_twice_fails() {
        let d = TempDir::new().unwrap();
        let _ = make_vault(d.path());
        assert!(matches!(
            Vault::create(d.path(), "p"),
            Err(VaultError::VaultAlreadyExists)
        ));
    }

    #[test]
    fn open_missing_fails() {
        let d = TempDir::new().unwrap();
        assert!(matches!(
            Vault::open(d.path()),
            Err(VaultError::VaultNotFound)
        ));
    }

    #[test]
    fn second_open_is_rejected_while_first_is_still_held() {
        let d = TempDir::new().unwrap();
        let _first = make_vault(d.path());

        assert!(matches!(
            Vault::open(d.path()),
            Err(VaultError::VaultLockedByAnotherProcess)
        ));

        drop(_first);
        // Releasing the first handle frees the lock for the next opener.
        assert!(Vault::open(d.path()).is_ok());
    }

    // ── Encrypt / export ─────────────────────────────────────────────────

    #[test]
    fn encrypt_and_export_exact_bytes() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        let src = TempDir::new().unwrap();
        let original = b"Hello, PocketVault!";
        let path = write_temp(src.path(), "notes.txt", original);

        let fid = v
            .encrypt_file(&path, None, &key, &JobControl::default())
            .unwrap();
        assert_eq!(v.meta.files.len(), 1);

        let export_dir = TempDir::new().unwrap();
        let out = v
            .export_file(&fid, export_dir.path(), &key, &JobControl::default())
            .unwrap();
        assert_eq!(fs::read(&out).unwrap(), original);
    }

    #[test]
    fn encrypted_item_lands_in_the_active_segment_not_a_dedicated_file() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let path = write_temp(src.path(), "secret.txt", b"data");

        v.encrypt_file(&path, None, &key, &JobControl::default())
            .unwrap();

        let entry = &v.meta.files[0];
        assert_eq!(entry.segment_index, 1);
        assert_eq!(entry.offset, 0);
        assert!(entry.length > 0);
        assert!(v.vault_dir().join("active.pv").exists());
    }

    #[test]
    fn metadata_preserves_original_name_and_mime() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let path = write_temp(src.path(), "photo.jpg", b"JFIF");

        let fid = v
            .encrypt_file(&path, None, &key, &JobControl::default())
            .unwrap();
        let meta = v.read_metadata(&fid, &key).unwrap();

        assert_eq!(meta.original_name, "photo.jpg");
        assert_eq!(meta.mime_type, "image/jpeg");
    }

    #[test]
    fn large_file_roundtrip_multi_chunk() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        // 5 MB — spans multiple 64 KB chunks
        let data: Vec<u8> = (0..5 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let src = TempDir::new().unwrap();
        let path = write_temp(src.path(), "big.bin", &data);

        let fid = v
            .encrypt_file(&path, None, &key, &JobControl::default())
            .unwrap();
        let export = TempDir::new().unwrap();
        let out = v
            .export_file(&fid, export.path(), &key, &JobControl::default())
            .unwrap();
        assert_eq!(fs::read(&out).unwrap(), data);
    }

    #[test]
    fn wrong_key_cannot_export() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let fid = v
            .encrypt_file(
                &write_temp(src.path(), "f.txt", b"x"),
                None,
                &key,
                &JobControl::default(),
            )
            .unwrap();

        // Create a different vault just to get a different key
        let d2 = TempDir::new().unwrap();
        let v2 = Vault::create(d2.path(), "other").unwrap();
        let bad_key = v2.meta.unlock("other").unwrap();

        let export = TempDir::new().unwrap();
        assert!(v
            .export_file(&fid, export.path(), &bad_key, &JobControl::default())
            .is_err());
    }

    // ── Delete ───────────────────────────────────────────────────────────

    #[test]
    fn delete_removes_manifest_entry_but_leaves_segment_file_for_later_reclaim() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let fid = v
            .encrypt_file(
                &write_temp(src.path(), "f.txt", b"data"),
                None,
                &key,
                &JobControl::default(),
            )
            .unwrap();

        let segment_path = v.vault_dir().join("active.pv");
        assert!(segment_path.exists());

        v.delete_file(&fid).unwrap();
        assert!(v.meta.files.is_empty());
        // The segment file itself isn't deleted — its dead bytes are only
        // reclaimed by an explicit repack (see `SegmentStore::repack`).
        assert!(segment_path.exists());
    }

    // ── Folders ──────────────────────────────────────────────────────────

    #[test]
    fn files_scoped_to_folder() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let folder_id = v.create_folder("Photos", None).unwrap();

        let src = TempDir::new().unwrap();
        v.encrypt_file(
            &write_temp(src.path(), "a.jpg", b"img"),
            Some(&folder_id),
            &key,
            &JobControl::default(),
        )
        .unwrap();
        v.encrypt_file(
            &write_temp(src.path(), "b.txt", b"txt"),
            None,
            &key,
            &JobControl::default(),
        )
        .unwrap();

        assert_eq!(v.files_in_folder(Some(&folder_id)).len(), 1);
        assert_eq!(v.files_in_folder(None).len(), 1);
    }

    #[test]
    fn encrypt_folder_preserves_structure_including_empty_subfolders() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        // src/
        //   top.txt
        //   sub/          (has a file)
        //     nested.txt
        //   empty_sub/    (no files at all)
        let src = TempDir::new().unwrap();
        let root = src.path().join("Photos");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join("empty_sub")).unwrap();
        fs::write(root.join("top.txt"), b"top").unwrap();
        fs::write(root.join("sub").join("nested.txt"), b"nested").unwrap();

        let ids = v
            .encrypt_folder(&root, None, &key, &JobControl::default())
            .unwrap();
        assert_eq!(ids.len(), 2); // top.txt + nested.txt, empty_sub contributes none

        // "Photos" created as a root vault folder
        let photos = v.meta.subfolders(None);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].name, "Photos");
        let photos_id = photos[0].id.clone();

        // "sub" and "empty_sub" both created under "Photos", including the empty one
        let mut children: Vec<&str> = v
            .meta
            .subfolders(Some(&photos_id))
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        children.sort();
        assert_eq!(children, vec!["empty_sub", "sub"]);

        // top.txt lives directly under Photos; nested.txt lives under Photos/sub
        assert_eq!(v.files_in_folder(Some(&photos_id)).len(), 1);
        let sub_id = v
            .meta
            .subfolders(Some(&photos_id))
            .iter()
            .find(|f| f.name == "sub")
            .unwrap()
            .id
            .clone();
        assert_eq!(v.files_in_folder(Some(&sub_id)).len(), 1);
    }

    #[test]
    fn export_folder_recreates_structure_on_disk() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        let src = TempDir::new().unwrap();
        let root = src.path().join("Photos");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join("empty_sub")).unwrap();
        fs::write(root.join("top.txt"), b"top").unwrap();
        fs::write(root.join("sub").join("nested.txt"), b"nested").unwrap();

        v.encrypt_folder(&root, None, &key, &JobControl::default())
            .unwrap();
        let photos_id = v.meta.subfolders(None)[0].id.clone();

        let export_dir = TempDir::new().unwrap();
        let out_dir = v
            .export_folder(&photos_id, export_dir.path(), &key, &JobControl::default())
            .unwrap();

        assert_eq!(out_dir, export_dir.path().join("Photos"));
        assert_eq!(fs::read(out_dir.join("top.txt")).unwrap(), b"top");
        assert_eq!(
            fs::read(out_dir.join("sub").join("nested.txt")).unwrap(),
            b"nested"
        );
        assert!(out_dir.join("empty_sub").is_dir());
    }

    #[test]
    fn encrypt_folder_recurses_to_arbitrary_depth() {
        // level1/level2/level3/level4/level5/deep.txt — proves encrypt_folder
        // has no hardcoded depth cap (unlike the sidebar's 2-level display).
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        let src = TempDir::new().unwrap();
        let deep_dir = src
            .path()
            .join("level1")
            .join("level2")
            .join("level3")
            .join("level4")
            .join("level5");
        fs::create_dir_all(&deep_dir).unwrap();
        fs::write(deep_dir.join("deep.txt"), b"buried treasure").unwrap();

        let ids = v
            .encrypt_folder(
                &src.path().join("level1"),
                None,
                &key,
                &JobControl::default(),
            )
            .unwrap();
        assert_eq!(ids.len(), 1);

        // Walk the vault folder chain level1 -> ... -> level5 and confirm the
        // file landed in level5, decrypting back to the original bytes.
        let mut current = v.meta.subfolders(None)[0].id.clone(); // level1
        for expected_name in ["level2", "level3", "level4", "level5"] {
            let children = v.meta.subfolders(Some(&current));
            assert_eq!(children.len(), 1);
            assert_eq!(children[0].name, expected_name);
            current = children[0].id.clone();
        }
        assert_eq!(v.files_in_folder(Some(&current)).len(), 1);

        let export_dir = TempDir::new().unwrap();
        let out = v
            .export_file(&ids[0], export_dir.path(), &key, &JobControl::default())
            .unwrap();
        assert_eq!(fs::read(out).unwrap(), b"buried treasure");
    }

    #[test]
    fn encrypt_multiple_files_in_one_batch() {
        // Mirrors the desktop app's multi-select "Encrypt Files" flow: one
        // batch job over every path returned by the native multi-select file
        // picker (rfd's `pick_files`, plural).
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();

        let paths = vec![
            write_temp(src.path(), "a.txt", b"AAA"),
            write_temp(src.path(), "b.txt", b"BBB"),
            write_temp(src.path(), "c.txt", b"CCC"),
        ];
        let ids = v
            .encrypt_files(&paths, None, &key, &JobControl::default())
            .unwrap();

        assert_eq!(v.meta.files.len(), 3);
        let mut names: Vec<String> = ids
            .iter()
            .map(|id| v.read_metadata(id, &key).unwrap().original_name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt", "c.txt"]);
    }

    // ── Cancellation / rollback ───────────────────────────────────────────

    #[test]
    fn encrypt_file_cancelled_leaves_no_partial_pv_or_meta_entry() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let path = write_temp(src.path(), "f.txt", b"data");

        let err = v
            .encrypt_file(&path, None, &key, &cancelled_control())
            .unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));

        assert!(v.meta.files.is_empty());
        assert!(fs::read_dir(v.vault_dir()).unwrap().next().is_none()); // no orphan .pv
    }

    #[test]
    fn encrypt_files_batch_rolls_back_on_mid_batch_failure() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();

        let paths = vec![
            write_temp(src.path(), "a.txt", b"AAA"),
            write_temp(src.path(), "b.txt", b"BBB"),
            src.path().join("missing.txt"), // never written — encrypt_file will fail on this one
        ];

        let err = v
            .encrypt_files(&paths, None, &key, &JobControl::default())
            .unwrap_err();
        assert!(matches!(err, VaultError::Io(_)));

        // a.txt and b.txt were encrypted successfully before the failure, but
        // the whole batch is all-or-nothing, so both are rolled back — their
        // manifest entries are gone even though the segment file that holds
        // their now-orphaned bytes isn't deleted (dead space is only
        // reclaimed by an explicit repack, see `SegmentStore::repack`).
        assert!(v.meta.files.is_empty());
    }

    #[test]
    fn encrypt_folder_cancelled_rolls_back_everything() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        let src = TempDir::new().unwrap();
        let root = src.path().join("Photos");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("top.txt"), b"top").unwrap();

        let err = v
            .encrypt_folder(&root, None, &key, &cancelled_control())
            .unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));

        assert!(v.meta.folders.is_empty());
        assert!(v.meta.files.is_empty());
        assert!(fs::read_dir(v.vault_dir()).unwrap().next().is_none());
    }

    #[test]
    fn export_file_cancelled_leaves_no_partial_destination_file() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let fid = v
            .encrypt_file(
                &write_temp(src.path(), "f.txt", b"data"),
                None,
                &key,
                &JobControl::default(),
            )
            .unwrap();

        let export_dir = TempDir::new().unwrap();
        let err = v
            .export_file(&fid, export_dir.path(), &key, &cancelled_control())
            .unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));
        assert!(!export_dir.path().join("f.txt").exists());
    }

    #[test]
    fn export_folder_cancelled_leaves_no_partial_destination_dir() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);

        let src = TempDir::new().unwrap();
        let root = src.path().join("Photos");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("top.txt"), b"top").unwrap();
        v.encrypt_folder(&root, None, &key, &JobControl::default())
            .unwrap();
        let photos_id = v.meta.subfolders(None)[0].id.clone();

        let export_dir = TempDir::new().unwrap();
        let err = v
            .export_folder(&photos_id, export_dir.path(), &key, &cancelled_control())
            .unwrap_err();
        assert!(matches!(err, VaultError::Cancelled));
        assert!(!export_dir.path().join("Photos").exists());
    }

    #[test]
    fn dir_total_size_sums_nested_files() {
        let src = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("sub")).unwrap();
        fs::write(src.path().join("a.bin"), vec![0u8; 100]).unwrap();
        fs::write(src.path().join("sub").join("b.bin"), vec![0u8; 250]).unwrap();

        assert_eq!(dir_total_size(src.path()).unwrap(), 350);
    }

    // ── Mime / preview helpers ────────────────────────────────────────────

    #[test]
    fn mime_detection() {
        assert_eq!(mime_for("photo.jpg"), "image/jpeg");
        assert_eq!(mime_for("photo.JPEG"), "image/jpeg");
        assert_eq!(mime_for("doc.pdf"), "application/pdf");
        assert_eq!(mime_for("notes.txt"), "text/plain");
        assert_eq!(mime_for("video.mp4"), "video/mp4");
        assert_eq!(mime_for("unknown.xyz"), "application/octet-stream");
    }

    #[test]
    fn preview_only_for_image_and_text() {
        assert!(is_previewable("image/jpeg"));
        assert!(is_previewable("image/png"));
        assert!(is_previewable("text/plain"));
        assert!(!is_previewable("application/pdf"));
        assert!(!is_previewable("video/mp4"));
    }
}
