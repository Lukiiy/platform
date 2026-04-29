use std::path::Path;
use anyhow::Result;

use crate::ui;

/// Opens a folder in the system's file explorer.
/// path: The path of the folder to open.
pub fn open_folder(path: &str) {
    #[cfg(target_os = "windows")]
    let cmd = ("explorer", path);

    #[cfg(target_os = "macos")]
    let cmd = ("open", path);

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let cmd = ("xdg-open", path);

    if std::process::Command::new(cmd.0).arg(cmd.1).spawn().is_err() {
        ui::warn(&format!("Could not open: {path}"));
    }
}

/// Copies the contents of a folder/file to a dest.
///
/// target: Path to target;
/// dest: Path to end folder.
///
/// Returns an error if the copy fails.
pub fn copy(target: &Path, dest: &Path) -> Result<()> {
    if target.is_dir() {
        std::fs::create_dir_all(dest)?;

        for thing in std::fs::read_dir(target)? {
            let entry = thing?;

            copy(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else { std::fs::copy(target, dest)?; }

    Ok(())
}

/// Creates a symlink from one folder to another!
///
/// source: Path to the source;
/// dest: Path to the end folder.
#[cfg(unix)]
pub fn create_symlink(source: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, dest)
}

/// Creates a symlink from one folder to another!
///
/// source: Path to the source;
/// dest: Path to the end folder.
#[cfg(windows)]
pub fn create_symlink(source: &Path, dest: &Path) -> std::io::Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, dest)
    } else {
        std::os::windows::fs::symlink_file(source, dest)
    }
}

/// Returns whether the given path is a managed symlink.
pub fn is_managed_symlink(path: &Path, group_src: &Path) -> bool {
    path.is_symlink() && std::fs::read_link(path).map(|t| t.starts_with(group_src)).unwrap_or(false)
}