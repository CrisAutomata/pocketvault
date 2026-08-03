//! Decrypts an item to a temp file and hands it to the OS's default app for
//! that file type — shared by both front ends' "Preview" action. Neither
//! terminal UI can render arbitrary file types itself (unlike the desktop
//! app's in-window image/text preview), so this delegates to whatever the
//! OS already knows how to open — works for any file type, not just
//! images/text.
//!
//! The temp file is real plaintext on disk, if only briefly: any previous
//! preview's temp file is removed first so at most one lingers at a time,
//! but it isn't wiped the instant the viewer opens it (that process needs
//! the file to still exist), so this is a deliberate, minor exposure versus
//! a fully in-memory preview.

use std::io;
use std::path::{Path, PathBuf};

pub fn write_temp_file(name: &str, data: &[u8]) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("pocketvault-preview");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, data)?;
    Ok(path)
}

pub fn open_with_os_default(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
    }
    Ok(())
}
