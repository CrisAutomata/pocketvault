use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::crypto::{decrypt, derive_key, encrypt, generate_vault_key, KdfParams, VaultKey};
use crate::error::{Result, VaultError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Argon2Settings {
    pub salt_b64: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultFileEntry {
    pub id: String,
    pub pv_filename: String,
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultMeta {
    pub version: u8,
    pub argon2: Argon2Settings,
    pub enc_vault_key_b64: String,
    pub vault_key_nonce_b64: String,
    pub folders: Vec<VaultFolder>,
    pub files: Vec<VaultFileEntry>,
    pub metadata_preview: bool,
}

impl VaultMeta {
    pub fn create_new(password: &str) -> Result<(VaultMeta, VaultKey)> {
        let kdf = KdfParams::default_secure();
        let kek = derive_key(password, &kdf)?;
        let dek = generate_vault_key();

        let (enc_dek, nonce) = encrypt(&kek, &dek.0)?;

        let meta = VaultMeta {
            version: 1,
            argon2: Argon2Settings {
                salt_b64: B64.encode(&kdf.salt),
                m_cost: kdf.m_cost,
                t_cost: kdf.t_cost,
                p_cost: kdf.p_cost,
            },
            enc_vault_key_b64: B64.encode(&enc_dek),
            vault_key_nonce_b64: B64.encode(nonce),
            folders: Vec::new(),
            files: Vec::new(),
            metadata_preview: true,
        };

        Ok((meta, dek))
    }

    pub fn unlock(&self, password: &str) -> Result<VaultKey> {
        let salt = B64
            .decode(&self.argon2.salt_b64)
            .map_err(|e| VaultError::Base64Error(e.to_string()))?;

        let kek = derive_key(
            password,
            &KdfParams {
                salt,
                m_cost: self.argon2.m_cost,
                t_cost: self.argon2.t_cost,
                p_cost: self.argon2.p_cost,
            },
        )?;

        let enc_dek = B64
            .decode(&self.enc_vault_key_b64)
            .map_err(|e| VaultError::Base64Error(e.to_string()))?;

        let nonce_bytes = B64
            .decode(&self.vault_key_nonce_b64)
            .map_err(|e| VaultError::Base64Error(e.to_string()))?;
        let mut nonce = [0u8; 12];
        if nonce_bytes.len() != 12 {
            return Err(VaultError::InvalidFileFormat);
        }
        nonce.copy_from_slice(&nonce_bytes);

        let dek_bytes = decrypt(&kek, &enc_dek, &nonce).map_err(|_| VaultError::InvalidPassword)?;

        if dek_bytes.len() != 32 {
            return Err(VaultError::InvalidPassword);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&dek_bytes);
        Ok(VaultKey(arr))
    }

    pub fn change_password(&mut self, old_password: &str, new_password: &str) -> Result<()> {
        let dek = self.unlock(old_password)?;

        let new_kdf = KdfParams::default_secure();
        let new_kek = derive_key(new_password, &new_kdf)?;
        let (enc_dek, nonce) = encrypt(&new_kek, &dek.0)?;

        self.argon2.salt_b64 = B64.encode(&new_kdf.salt);
        self.enc_vault_key_b64 = B64.encode(&enc_dek);
        self.vault_key_nonce_b64 = B64.encode(nonce);

        Ok(())
    }

    pub fn add_folder(&mut self, name: &str, parent_id: Option<&str>) -> String {
        let id = Uuid::new_v4().to_string();
        self.folders.push(VaultFolder {
            id: id.clone(),
            name: name.to_string(),
            parent_id: parent_id.map(str::to_string),
        });
        id
    }

    pub fn add_file(&mut self, pv_filename: &str, folder_id: Option<&str>) -> String {
        let id = Uuid::new_v4().to_string();
        self.files.push(VaultFileEntry {
            id: id.clone(),
            pv_filename: pv_filename.to_string(),
            folder_id: folder_id.map(str::to_string),
        });
        id
    }

    pub fn remove_file(&mut self, file_id: &str) -> Option<VaultFileEntry> {
        if let Some(pos) = self.files.iter().position(|f| f.id == file_id) {
            Some(self.files.remove(pos))
        } else {
            None
        }
    }

    pub fn rename_folder(&mut self, folder_id: &str, new_name: &str) -> bool {
        if let Some(f) = self.folders.iter_mut().find(|f| f.id == folder_id) {
            f.name = new_name.to_string();
            true
        } else {
            false
        }
    }

    /// Removes `folder_id` and its entire subtree — every nested subfolder and
    /// every file anywhere in that subtree, not just direct children.
    pub fn remove_folder(&mut self, folder_id: &str) -> Vec<VaultFileEntry> {
        let mut ids_to_remove = vec![folder_id.to_string()];
        let mut i = 0;
        while i < ids_to_remove.len() {
            let current = ids_to_remove[i].clone();
            for sub in self.subfolders(Some(&current)) {
                ids_to_remove.push(sub.id.clone());
            }
            i += 1;
        }

        let in_subtree = |fid: &str| ids_to_remove.iter().any(|id| id == fid);

        let removed_files: Vec<VaultFileEntry> = self
            .files
            .iter()
            .filter(|f| f.folder_id.as_deref().is_some_and(in_subtree))
            .cloned()
            .collect();

        self.files
            .retain(|f| !f.folder_id.as_deref().is_some_and(in_subtree));
        self.folders.retain(|f| !ids_to_remove.contains(&f.id));

        removed_files
    }

    pub fn files_in_folder<'a>(&'a self, folder_id: Option<&str>) -> Vec<&'a VaultFileEntry> {
        self.files
            .iter()
            .filter(|f| f.folder_id.as_deref() == folder_id)
            .collect()
    }

    pub fn subfolders<'a>(&'a self, parent_id: Option<&str>) -> Vec<&'a VaultFolder> {
        self.folders
            .iter()
            .filter(|f| f.parent_id.as_deref() == parent_id)
            .collect()
    }

    /// The chain of folders from the vault root down to `folder_id`, root first.
    /// Empty if `folder_id` isn't found.
    pub fn folder_path<'a>(&'a self, folder_id: &str) -> Vec<&'a VaultFolder> {
        let mut chain = Vec::new();
        let mut current = self.folders.iter().find(|f| f.id == folder_id);
        while let Some(f) = current {
            chain.push(f);
            current = f
                .parent_id
                .as_deref()
                .and_then(|pid| self.folders.iter().find(|x| x.id == pid));
        }
        chain.reverse();
        chain
    }

    pub fn folder_file_count(&self, folder_id: &str) -> usize {
        self.files
            .iter()
            .filter(|f| f.folder_id.as_deref() == Some(folder_id))
            .count()
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_unlock() {
        let (meta, key) = VaultMeta::create_new("mypassword").unwrap();
        let unlocked = meta.unlock("mypassword").unwrap();
        assert_eq!(key.0, unlocked.0);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let (meta, _) = VaultMeta::create_new("correct").unwrap();
        assert!(meta.unlock("wrong").is_err());
    }

    #[test]
    fn change_password_preserves_vault_key() {
        let (mut meta, original_key) = VaultMeta::create_new("old").unwrap();
        meta.change_password("old", "new").unwrap();

        // old password must fail
        assert!(meta.unlock("old").is_err());

        // new password must work and yield the same vault key
        let new_key = meta.unlock("new").unwrap();
        assert_eq!(original_key.0, new_key.0);
    }

    #[test]
    fn change_password_wrong_old_fails() {
        let (mut meta, _) = VaultMeta::create_new("pass").unwrap();
        assert!(meta.change_password("wrong", "new").is_err());
    }

    #[test]
    fn add_and_remove_folder() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let id = meta.add_folder("Photos", None);
        assert_eq!(meta.folders.len(), 1);
        assert_eq!(meta.folders[0].name, "Photos");

        meta.remove_folder(&id);
        assert!(meta.folders.is_empty());
    }

    #[test]
    fn nested_folders() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let parent = meta.add_folder("Root", None);
        let child = meta.add_folder("Child", Some(&parent));

        let roots = meta.subfolders(None);
        assert_eq!(roots.len(), 1);

        let children = meta.subfolders(Some(&parent));
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child);
    }

    #[test]
    fn folder_path_walks_root_to_leaf() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let root = meta.add_folder("asdasdad", None);
        let mid = meta.add_folder(".astro", Some(&root));
        let leaf = meta.add_folder("collections", Some(&mid));

        let path = meta.folder_path(&leaf);
        let names: Vec<&str> = path.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["asdasdad", ".astro", "collections"]);

        assert_eq!(meta.folder_path(&root).len(), 1);
        assert!(meta.folder_path("missing").is_empty());
    }

    #[test]
    fn add_and_remove_file() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let id = meta.add_file("abc.pv", None);
        assert_eq!(meta.files.len(), 1);

        let removed = meta.remove_file(&id).unwrap();
        assert_eq!(removed.pv_filename, "abc.pv");
        assert!(meta.files.is_empty());
    }

    #[test]
    fn remove_folder_cascades_to_files() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let folder_id = meta.add_folder("Photos", None);
        meta.add_file("a.pv", Some(&folder_id));
        meta.add_file("b.pv", Some(&folder_id));
        meta.add_file("c.pv", None); // root file

        let removed = meta.remove_folder(&folder_id);
        assert_eq!(removed.len(), 2);
        assert_eq!(meta.files.len(), 1); // root file remains
    }

    #[test]
    fn remove_folder_cascades_to_nested_subfolders() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let root = meta.add_folder("asdasdad", None);
        let mid = meta.add_folder(".astro", Some(&root));
        let leaf = meta.add_folder("collections", Some(&mid));
        meta.add_file("mid.pv", Some(&mid));
        meta.add_file("leaf.pv", Some(&leaf));
        meta.add_file("root_file.pv", None);

        let removed = meta.remove_folder(&root);
        let mut removed_names: Vec<&str> = removed.iter().map(|f| f.pv_filename.as_str()).collect();
        removed_names.sort();
        assert_eq!(removed_names, vec!["leaf.pv", "mid.pv"]);

        // root, mid, and leaf are all gone; unrelated root file survives
        assert!(meta.folders.is_empty());
        assert_eq!(meta.files.len(), 1);
        assert_eq!(meta.files[0].pv_filename, "root_file.pv");
    }

    #[test]
    fn folder_file_count() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let id = meta.add_folder("Docs", None);
        meta.add_file("x.pv", Some(&id));
        meta.add_file("y.pv", Some(&id));
        meta.add_file("z.pv", None);

        assert_eq!(meta.folder_file_count(&id), 2);
    }

    #[test]
    fn files_in_folder_root() {
        let (mut meta, _) = VaultMeta::create_new("p").unwrap();
        let id = meta.add_folder("F", None);
        meta.add_file("a.pv", None);
        meta.add_file("b.pv", Some(&id));

        assert_eq!(meta.files_in_folder(None).len(), 1);
        assert_eq!(meta.files_in_folder(Some(&id)).len(), 1);
    }
}
