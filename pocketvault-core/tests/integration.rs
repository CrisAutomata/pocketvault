use pocketvault_core::{Vault, VaultError};
use std::fs;
use std::sync::atomic::AtomicBool;
use tempfile::TempDir;

// ── Full lifecycle ────────────────────────────────────────────────────────────

/// Encrypt → close → reopen → unlock → export → byte-compare
#[test]
fn full_vault_lifecycle() {
    let dir = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();

    // 1. Create vault and folders
    let mut vault = Vault::create(dir.path(), "StrongPassw0rd!").unwrap();
    let key = vault.meta.unlock("StrongPassw0rd!").unwrap();

    let photos = vault.create_folder("Photos", None).unwrap();
    let docs = vault.create_folder("Documents", None).unwrap();

    // 2. Encrypt files
    let img_data = b"JPEG\xFF\xD8\xFF\xE0 fake image bytes";
    let doc_data = b"This is a secret document.";

    let img_path = src.path().join("photo.jpg");
    let doc_path = src.path().join("report.txt");
    fs::write(&img_path, img_data).unwrap();
    fs::write(&doc_path, doc_data).unwrap();

    let img_id = vault.encrypt_file(&img_path, Some(&photos), &key, &AtomicBool::new(false)).unwrap();
    let doc_id = vault.encrypt_file(&doc_path, Some(&docs), &key, &AtomicBool::new(false)).unwrap();

    // 3. Verify in-memory state
    assert_eq!(vault.folders(None).len(), 2);
    assert_eq!(vault.files_in_folder(Some(&photos)).len(), 1);
    assert_eq!(vault.files_in_folder(Some(&docs)).len(), 1);
    assert_eq!(vault.files_in_folder(None).len(), 0);

    // 4. Close (drop key + vault)
    drop(key);
    drop(vault);

    // 5. Reopen from disk
    let vault = Vault::open(dir.path()).unwrap();
    assert_eq!(vault.meta.folders.len(), 2);
    assert_eq!(vault.meta.files.len(), 2);

    // 6. Unlock and export
    let key = vault.meta.unlock("StrongPassw0rd!").unwrap();
    let export = TempDir::new().unwrap();

    let img_out = vault.export_file(&img_id, export.path(), &key, &AtomicBool::new(false)).unwrap();
    let doc_out = vault.export_file(&doc_id, export.path(), &key, &AtomicBool::new(false)).unwrap();

    assert_eq!(fs::read(&img_out).unwrap(), img_data);
    assert_eq!(fs::read(&doc_out).unwrap(), doc_data);

    // 7. Metadata preserved
    let m = vault.read_metadata(&img_id, &key).unwrap();
    assert_eq!(m.original_name, "photo.jpg");
    assert_eq!(m.mime_type, "image/jpeg");
}

// ── Password change ───────────────────────────────────────────────────────────

/// Change password → close → reopen → old pw fails, new pw works, files intact
#[test]
fn password_change_files_survive() {
    let dir = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();

    let mut vault = Vault::create(dir.path(), "old_password").unwrap();
    let key = vault.meta.unlock("old_password").unwrap();

    let path = src.path().join("secret.txt");
    fs::write(&path, b"top secret").unwrap();
    let fid = vault.encrypt_file(&path, None, &key, &AtomicBool::new(false)).unwrap();
    drop(key);

    vault.meta.change_password("old_password", "new_password").unwrap();
    vault.save().unwrap();
    drop(vault);

    let vault = Vault::open(dir.path()).unwrap();
    assert!(vault.meta.unlock("old_password").is_err());

    let new_key = vault.meta.unlock("new_password").unwrap();
    let export = TempDir::new().unwrap();
    let out = vault.export_file(&fid, export.path(), &new_key, &AtomicBool::new(false)).unwrap();
    assert_eq!(fs::read(&out).unwrap(), b"top secret");
}

// ── Wrong password ────────────────────────────────────────────────────────────

#[test]
fn wrong_password_returns_invalid_password() {
    let dir = TempDir::new().unwrap();
    let vault = Vault::create(dir.path(), "correct").unwrap();
    assert!(matches!(vault.meta.unlock("wrong"), Err(VaultError::InvalidPassword)));
}

// ── Batch export ──────────────────────────────────────────────────────────────

/// Encrypt several files in a folder, export all, verify each
#[test]
fn batch_encrypt_and_export() {
    let dir = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();

    let mut vault = Vault::create(dir.path(), "pass").unwrap();
    let key = vault.meta.unlock("pass").unwrap();
    let folder_id = vault.create_folder("Batch", None).unwrap();

    let files: &[(&str, &[u8])] = &[
        ("alpha.txt", b"alpha content"),
        ("beta.txt", b"beta content"),
        ("gamma.txt", b"gamma content"),
    ];

    let mut ids = Vec::new();
    for (name, content) in files {
        let p = src.path().join(name);
        fs::write(&p, content).unwrap();
        ids.push(vault.encrypt_file(&p, Some(&folder_id), &key, &AtomicBool::new(false)).unwrap());
    }

    assert_eq!(vault.files_in_folder(Some(&folder_id)).len(), 3);

    let export = TempDir::new().unwrap();
    for (id, (_, expected)) in ids.iter().zip(files) {
        let out = vault.export_file(id, export.path(), &key, &AtomicBool::new(false)).unwrap();
        assert_eq!(&fs::read(&out).unwrap(), expected);
    }
}

// ── Delete ────────────────────────────────────────────────────────────────────

#[test]
fn delete_removes_pv_from_disk() {
    let dir = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();

    let mut vault = Vault::create(dir.path(), "pass").unwrap();
    let key = vault.meta.unlock("pass").unwrap();
    let p = src.path().join("f.txt");
    fs::write(&p, b"bye").unwrap();

    let fid = vault.encrypt_file(&p, None, &key, &AtomicBool::new(false)).unwrap();
    let pv_path = vault.vault_dir().join(&vault.meta.files[0].pv_filename);
    assert!(pv_path.exists());

    vault.delete_file(&fid).unwrap();
    assert!(vault.meta.files.is_empty());
    assert!(!pv_path.exists());
}
