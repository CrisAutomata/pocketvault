use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid password")]
    InvalidPassword,

    #[error("Vault already exists")]
    VaultAlreadyExists,

    #[error("Vault not found")]
    VaultNotFound,

    #[error("Vault is locked")]
    VaultLocked,

    #[error("Encryption failed")]
    EncryptionFailed,

    #[error("Decryption failed — wrong password or corrupted file")]
    DecryptionFailed,

    #[error("Invalid .pv file format")]
    InvalidFileFormat,

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Folder not found: {0}")]
    FolderNotFound(String),

    #[error("Base64 error: {0}")]
    Base64Error(String),
}

pub type Result<T> = std::result::Result<T, VaultError>;
