use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use uuid::Uuid;

use crate::{
    error::{Result, VaultError},
    meta::{VaultFileEntry, VaultFolder, VaultMeta},
    pv_format::{read_pv, read_pv_metadata, write_pv, PvMetadata},
    crypto::VaultKey,
};

pub struct Vault {
    pub base_dir: PathBuf,
    pub meta: VaultMeta,
}

impl Vault {
    pub fn create(base_dir: &Path, password: &str) -> Result<Vault> {
        let meta_path = base_dir.join("vault.meta");
        if meta_path.exists() {
            return Err(VaultError::VaultAlreadyExists);
        }

        fs::create_dir_all(base_dir.join("vault"))?;

        let (meta, _dek) = VaultMeta::create_new(password)?;
        let json = serde_json::to_string_pretty(&meta)?;
        fs::write(&meta_path, json)?;

        Ok(Vault {
            base_dir: base_dir.to_path_buf(),
            meta,
        })
    }

    pub fn open(base_dir: &Path) -> Result<Vault> {
        let meta_path = base_dir.join("vault.meta");
        if !meta_path.exists() {
            return Err(VaultError::VaultNotFound);
        }
        let json = fs::read_to_string(&meta_path)?;
        let meta: VaultMeta = serde_json::from_str(&json)?;
        Ok(Vault {
            base_dir: base_dir.to_path_buf(),
            meta,
        })
    }

    pub fn exists(base_dir: &Path) -> bool {
        base_dir.join("vault.meta").exists()
    }

    // ── Paths ───────────────────────────────────────────────────────────────

    pub fn vault_dir(&self) -> PathBuf {
        self.base_dir.join("vault")
    }

    fn pv_path(&self, pv_filename: &str) -> PathBuf {
        self.vault_dir().join(pv_filename)
    }

    // ── Persistence ─────────────────────────────────────────────────────────

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.meta)?;
        fs::write(self.base_dir.join("vault.meta"), json)?;
        Ok(())
    }

    // ── Folder operations ────────────────────────────────────────────────────

    pub fn create_folder(&mut self, name: &str, parent_id: Option<&str>) -> Result<String> {
        let id = self.meta.add_folder(name, parent_id);
        self.save()?;
        Ok(id)
    }

    pub fn rename_folder(&mut self, folder_id: &str, new_name: &str) -> Result<()> {
        if !self.meta.rename_folder(folder_id, new_name) {
            return Err(VaultError::FileNotFound(folder_id.to_string()));
        }
        self.save()?;
        Ok(())
    }

    pub fn delete_folder(&mut self, folder_id: &str) -> Result<()> {
        let removed = self.meta.remove_folder(folder_id);
        for f in &removed {
            let path = self.pv_path(&f.pv_filename);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        self.save()?;
        Ok(())
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
    ) -> Result<String> {
        let filename = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| VaultError::FileNotFound(source.to_string_lossy().to_string()))?;

        let data = fs::read(source)?;

        let modified_ts = fs::metadata(source)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(now_ts());

        let pv_meta = PvMetadata {
            original_name: filename.to_string(),
            original_size: data.len() as u64,
            created_ts: now_ts(),
            modified_ts,
            mime_type: mime_for(filename),
        };

        let pv_filename = format!("{}.pv", Uuid::new_v4());
        let pv_path = self.pv_path(&pv_filename);

        let mut file = fs::File::create(&pv_path)?;
        write_pv(&mut file, key, &pv_meta, &data)?;

        let id = self.meta.add_file(&pv_filename, folder_id);
        self.save()?;

        Ok(id)
    }

    pub fn delete_file(&mut self, file_id: &str) -> Result<()> {
        let entry = self
            .meta
            .remove_file(file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let path = self.pv_path(&entry.pv_filename);
        if path.exists() {
            fs::remove_file(path)?;
        }
        self.save()?;
        Ok(())
    }

    pub fn export_file(&self, file_id: &str, dest_dir: &Path, key: &VaultKey) -> Result<PathBuf> {
        let entry = self
            .meta
            .files
            .iter()
            .find(|f| f.id == file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let mut f = fs::File::open(self.pv_path(&entry.pv_filename))?;
        let (meta, plaintext) = read_pv(&mut f, key)?;

        let dest = dest_dir.join(&meta.original_name);
        fs::write(&dest, &plaintext)?;

        Ok(dest)
    }

    pub fn read_metadata(&self, file_id: &str, key: &VaultKey) -> Result<PvMetadata> {
        let entry = self
            .meta
            .files
            .iter()
            .find(|f| f.id == file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let mut f = fs::File::open(self.pv_path(&entry.pv_filename))?;
        read_pv_metadata(&mut f, key)
    }

    pub fn read_to_memory(&self, file_id: &str, key: &VaultKey) -> Result<(PvMetadata, Vec<u8>)> {
        let entry = self
            .meta
            .files
            .iter()
            .find(|f| f.id == file_id)
            .ok_or_else(|| VaultError::FileNotFound(file_id.to_string()))?;

        let mut f = fs::File::open(self.pv_path(&entry.pv_filename))?;
        read_pv(&mut f, key)
    }

    pub fn files_in_folder(&self, folder_id: Option<&str>) -> Vec<&VaultFileEntry> {
        self.meta.files_in_folder(folder_id)
    }
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn mime_for(name: &str) -> String {
    match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
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
        assert!(matches!(Vault::create(d.path(), "p"), Err(VaultError::VaultAlreadyExists)));
    }

    #[test]
    fn open_missing_fails() {
        let d = TempDir::new().unwrap();
        assert!(matches!(Vault::open(d.path()), Err(VaultError::VaultNotFound)));
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

        let fid = v.encrypt_file(&path, None, &key).unwrap();
        assert_eq!(v.meta.files.len(), 1);

        let export_dir = TempDir::new().unwrap();
        let out = v.export_file(&fid, export_dir.path(), &key).unwrap();
        assert_eq!(fs::read(&out).unwrap(), original);
    }

    #[test]
    fn pv_file_stored_with_uuid_name() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let path = write_temp(src.path(), "secret.txt", b"data");

        v.encrypt_file(&path, None, &key).unwrap();

        let pv_name = &v.meta.files[0].pv_filename;
        assert!(pv_name.ends_with(".pv"));
        assert_ne!(pv_name.as_str(), "secret.txt.pv"); // UUID, not original name
    }

    #[test]
    fn metadata_preserves_original_name_and_mime() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let path = write_temp(src.path(), "photo.jpg", b"JFIF");

        let fid = v.encrypt_file(&path, None, &key).unwrap();
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

        let fid = v.encrypt_file(&path, None, &key).unwrap();
        let export = TempDir::new().unwrap();
        let out = v.export_file(&fid, export.path(), &key).unwrap();
        assert_eq!(fs::read(&out).unwrap(), data);
    }

    #[test]
    fn wrong_key_cannot_export() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let fid = v.encrypt_file(&write_temp(src.path(), "f.txt", b"x"), None, &key).unwrap();

        // Create a different vault just to get a different key
        let d2 = TempDir::new().unwrap();
        let v2 = Vault::create(d2.path(), "other").unwrap();
        let bad_key = v2.meta.unlock("other").unwrap();

        let export = TempDir::new().unwrap();
        assert!(v.export_file(&fid, export.path(), &bad_key).is_err());
    }

    // ── Delete ───────────────────────────────────────────────────────────

    #[test]
    fn delete_removes_file_and_pv() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let src = TempDir::new().unwrap();
        let fid = v.encrypt_file(&write_temp(src.path(), "f.txt", b"data"), None, &key).unwrap();

        let pv_path = v.vault_dir().join(&v.meta.files[0].pv_filename);
        assert!(pv_path.exists());

        v.delete_file(&fid).unwrap();
        assert!(v.meta.files.is_empty());
        assert!(!pv_path.exists());
    }

    // ── Folders ──────────────────────────────────────────────────────────

    #[test]
    fn files_scoped_to_folder() {
        let d = TempDir::new().unwrap();
        let mut v = make_vault(d.path());
        let key = unlock(&v);
        let folder_id = v.create_folder("Photos", None).unwrap();

        let src = TempDir::new().unwrap();
        v.encrypt_file(&write_temp(src.path(), "a.jpg", b"img"), Some(&folder_id), &key).unwrap();
        v.encrypt_file(&write_temp(src.path(), "b.txt", b"txt"), None, &key).unwrap();

        assert_eq!(v.files_in_folder(Some(&folder_id)).len(), 1);
        assert_eq!(v.files_in_folder(None).len(), 1);
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
