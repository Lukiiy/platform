use chrono::Local;
use std::fs::{self, File};
use std::io::{self, Result, Error, ErrorKind, Write, Seek};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};
use crate::config::{Config, ServerEntry};
use crate::file_utils;

const EXCLUDED_DIRS: &[&str] = &["logs", "crash-reports", "cache", "libraries", "versions"];
const EXCLUDED_FILES: &[&str] = &[];

fn log_error(msg: impl Into<String>) -> Error {
    Error::new(ErrorKind::Other, msg.into())
}

#[derive(Debug, Clone)]
pub struct Backup {
    pub path: PathBuf,
    pub name: String,
    pub size: u64
}

pub fn server_backup_dir(config: &Config, server: &ServerEntry) -> PathBuf {
    config.backups_dir().join(&server.id)
}

/// Creates a backup!
pub fn create(config: &Config, server: &ServerEntry) -> Result<PathBuf> {
    if !server.path.exists() {
        return Err(log_error(format!("Server folder does not exist: {}", server.path.display())));
    }

    let dir = server_backup_dir(config, server);

    fs::create_dir_all(&dir)?;

    let path = dir.join(&format!("{}.zip", Local::now().format("%Y-%m-%d_%H-%M-%S")));
    let tmp = path.with_extension("zip.tmp");
    let mut zip = ZipWriter::new(File::create(&tmp)?);

    add_directory(&mut zip, &server.path, &server.path, SimpleFileOptions::default().compression_method(CompressionMethod::ZSTD).compression_level(Some(6)))?;
    zip.finish()?;
    fs::rename(&tmp, &path)?;

    Ok(path)
}

fn add_directory<W: Write + Seek>(zip: &mut ZipWriter<W>, source: &Path, current: &Path, options: SimpleFileOptions) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let path = entry?.path();

        if should_exclude(&path) {
            continue;
        }

        if path.is_dir() {
            add_directory(zip, source, &path, options)?;
        } else {
            let relative = path.strip_prefix(source).map_err(|error| log_error(error.to_string()))?.to_string_lossy().replace('\\', "/");

            zip.start_file(relative, options)?;
            io::copy(&mut File::open(&path)?, zip)?;
        }
    }

    Ok(())
}

fn should_exclude(target: &Path) -> bool {
    let Some(name) = target.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    if target.is_dir() {
        EXCLUDED_DIRS.contains(&name)
    } else {
        EXCLUDED_FILES.contains(&name)
    }
}

pub fn list(config: &Config, server: &ServerEntry) -> Result<Vec<Backup>> {
    let dir = server_backup_dir(config, server);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut backups: Vec<Backup> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok()).filter(|entry| entry.path().extension().and_then(|x| x.to_str()) == Some("zip"))
        .filter_map(|e| {
            let metadata = e.metadata().ok()?;

            metadata.is_file().then(|| Backup {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
                size: metadata.len()
            })
        }).collect();

    backups.sort_by(|a, b| b.name.cmp(&a.name));

    Ok(backups)
}

pub fn remove(backup: &Backup) -> Result<()> {
    fs::remove_file(&backup.path)
}

pub fn restore_backup(server: &ServerEntry, backup: &Backup) -> Result<()> {
    let tmp = server.path.with_extension("restore.tmp");
    let _ = file_utils::remove_dir(&tmp);

    fs::create_dir_all(&tmp)?;

    let result = extract(&backup.path, &tmp)
        .and_then(|_| file_utils::remove_dir(&server.path).map_err(|err| log_error(err.to_string())))
        .and_then(|_| file_utils::copy(&tmp, &server.path).map_err(|err| log_error(err.to_string())));

    let _ = file_utils::remove_dir(&tmp);
    result?;

    Ok(())
}

fn extract(source: &Path, target: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(source)?)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let enclosed = entry.enclosed_name().ok_or_else(|| log_error("unsafe path"))?;
        let output = target.join(enclosed);

        if entry.is_dir() {
            fs::create_dir_all(&output)?;

            continue;
        }

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }

        io::copy(&mut entry, &mut File::create(&output)?)?;
    }

    Ok(())
}