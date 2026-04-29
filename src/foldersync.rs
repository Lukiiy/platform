use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};

use crate::config::{ServerEntry, Config};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderLinks {
    pub name: String,
    pub servers: Vec<String>,

    #[serde(default)]
    pub mode: LinkMode
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkMode {
    #[default]
    Symlink,

    Copy
}

impl fmt::Display for LinkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Symlink => "symlink",
            Self::Copy => "copy"
        })
    }
}

/// Stores the results of a foldersync sync/unsync call.
#[derive(Default, Debug)]
pub struct SyncReport {
    pub synced: u32,
    pub overridden: u32,
    pub errors: Vec<String>
}

impl std::fmt::Display for SyncReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} synced, {} overridden", self.synced, self.overridden)
    }
}

/// Syncs a folder group to the given servers.
///
/// group: FolderSync to sync;
/// servers: Servers to sync to.
///
/// Returns a SyncReport
pub fn sync(group: &FolderLinks, servers: &[ServerEntry]) -> Result<SyncReport> {
    let source = Config::load()?.foldersync_dir(group);
    let mut report = SyncReport::default();

    std::fs::create_dir_all(&source)?;

    for server in servers.iter().filter(|s| group.servers.contains(&s.id)) {
        std::fs::create_dir_all(&server.path)?;

        for thing in std::fs::read_dir(&source)? {
            let entry = thing?;

            sync_entry(&entry.path(), &server.path.join(entry.file_name()), &source, &group.mode, &mut report)?;
        }
    }

    Ok(report)
}

/// Unsyncs a folder sync group from the servers.
///
/// group: FolderSync to sync;
/// servers: Servers to unsync from.
///
/// Returns the number of files removed
pub fn unsync(group: &FolderLinks, servers: &[ServerEntry]) -> Result<u32> {
    let source = Config::load()?.foldersync_dir(group);
    let mut removed = 0u32;

    for server in servers.iter().filter(|s| group.servers.contains(&s.id)) {
        if !server.path.exists() { continue; }

        unsync_dir(&source, &server.path, &source, &mut removed)?;
    }

    Ok(removed)
}

/// Removes the contents of a group from the server.
///
/// Returns the number of files removed.
fn unsync_dir(group_dir: &Path, server_dir: &Path, group_source: &Path, removed: &mut u32) -> Result<()> {
    for thing in std::fs::read_dir(group_dir)? {
        let entry = thing?;
        let target = server_dir.join(entry.file_name());

        if entry.path().is_dir() && target.is_dir() && !target.is_symlink() {
            unsync_dir(&entry.path(), &target, group_source, removed)?;
        } else if is_managed_symlink(&target, group_source) {
            std::fs::remove_file(&target)?;

            *removed += 1;
        }
    }

    Ok(())
}

/// Syncs a single entry from the given source to the given target, using the given link mode and report.
///
/// Returns an error if the sync fails.
fn sync_entry(source: &Path, target: &Path, group_source: &Path, mode: &LinkMode, report: &mut SyncReport) -> Result<()> {
    if target.is_dir() && !target.is_symlink() {
        for thing in std::fs::read_dir(source)? {
            let entry = thing?;

            sync_entry(&entry.path(), &target.join(entry.file_name()), group_source, mode, report)?;
        }

        return Ok(());
    }

    if (target.exists() || target.is_symlink()) && !is_managed_symlink(target, group_source) {
        report.overridden += 1;

        return Ok(());
    }

    if target.is_symlink() { std::fs::remove_file(target)?; }

    let result = match mode {
        LinkMode::Symlink => create_symlink(source, target).map_err(anyhow::Error::from),
        LinkMode::Copy => copy(source, target)
    };

    match result {
        Ok(()) => report.synced += 1,
        Err(e) => report.errors.push(format!("{}: {e}", target.display()))
    }

    Ok(())
}

/// Returns whether the given path is a managed symlink.
fn is_managed_symlink(path: &Path, group_src: &Path) -> bool {
    path.is_symlink() && std::fs::read_link(path).map(|t| t.starts_with(group_src)).unwrap_or(false)
}

/// Copies the contents of a folder/file to a dest.
///
/// target: Path to target;
/// dest: Path to end folder.
///
/// Returns an error if the copy fails.
fn copy(target: &Path, dest: &Path) -> Result<()> {
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
fn create_symlink(source: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, dest)
}

/// Creates a symlink from one folder to another!
///
/// source: Path to the source;
/// dest: Path to the end folder.
#[cfg(windows)]
fn create_symlink(source: &Path, dest: &Path) -> std::io::Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, dest)
    } else {
        std::os::windows::fs::symlink_file(source, dest)
    }
}