use anyhow::Result;
use std::path::Path;
use crate::config::{LinkMode, FolderLinks, ServerEntry, Config};

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

pub fn sync(group: &FolderLinks, servers: &[ServerEntry]) -> Result<SyncReport> {
    let source = Config::load()?.group_dir(group);
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

pub fn unsync(group: &FolderLinks, servers: &[ServerEntry]) -> Result<u32> {
    let source = Config::load()?.group_dir(group);
    let mut removed = 0u32;

    for server in servers.iter().filter(|s| group.servers.contains(&s.id)) {
        if !server.path.exists() { continue; }

        unsync_dir(&source, &server.path, &source, &mut removed)?;
    }

    Ok(removed)
}

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

fn is_managed_symlink(path: &Path, group_src: &Path) -> bool {
    path.is_symlink() && std::fs::read_link(path).map(|t| t.starts_with(group_src)).unwrap_or(false)
}

fn copy(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;

        for thing in std::fs::read_dir(src)? {
            let entry = thing?;

            copy(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }

    Ok(())
}

#[cfg(unix)]
fn create_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}
#[cfg(windows)]
fn create_symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::os::windows::fs::symlink_dir(src, dst)
    } else {
        std::os::windows::fs::symlink_file(src, dst)
    }
}