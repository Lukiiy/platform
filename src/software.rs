use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// todo: snapshots & more modded servers?

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Software {
    Vanilla,
    Paper,
    Fabric,
    Custom
}

impl Software {
    pub const EVERYTHING: &'static [(Self, &'static str, &'static str, &'static str)] = &[
        (Self::Vanilla, "vanilla", "Vanilla", "Mojang server"),
        (Self::Paper, "paper", "Paper", "Plugin support"),
        (Self::Fabric, "fabric", "Fabric", "Mod support"),
        (Self::Custom, "custom", "Custom", "Your own jar")
    ];

    fn entry(&self) -> &'static (Self, &'static str, &'static str, &'static str) {
        Self::EVERYTHING.iter().find(|(s, ..)| s == self).unwrap()
    }

    pub fn as_str(&self) -> &'static str { self.entry().2 }

    pub fn from_str(string: &str) -> Self {
        Self::EVERYTHING.iter().find(|(_, id, ..)| *id == string).map_or(Self::Custom, |(v, ..)| *v)
    }

    pub fn auto_download(&self) -> bool { !matches!(self, Self::Custom) }

    pub fn log_regex(software: &Software) -> Regex {
        match software {
            Self::Paper => Regex::new(r"^\[\d{2}:\d{2}:\d{2}\s+([A-Z]+)\]:\s*(.*)$").unwrap(),
            _ => Regex::new(r"^\[\d{2}:\d{2}:\d{2}\]\s+\[[^/\]]+/([A-Z]+)\]:\s*(.*)$").unwrap()
        }
    }
}

pub struct SoftwareManager {
    pub software_dir: PathBuf,

    client: Client
}

impl SoftwareManager {
    pub fn new(software_dir: PathBuf) -> Self {
        Self {
            software_dir,
            client: Client::builder().build().unwrap()
        }
    }

    pub async fn ensure_jar(&self, software: Software, mc_version: &str) -> Result<(PathBuf, String)> {
        let (url, jar_name) = self.resolve(software, mc_version).await?;

        let dest = self.software_dir.join(software.as_str().to_lowercase()).join(&jar_name);
        if !dest.exists() {
            std::fs::create_dir_all(dest.parent().unwrap())?;

            self.download(&url, &dest, &jar_name).await?;
        }

        Ok((dest, jar_name))
    }

    /// Returns Some((current, latest)) when an update is available, None if up to date.
    pub async fn check_update(&self, software: Software, mc_version: &str, current: Option<&str>) -> Result<Option<(Option<String>, String)>> {
        let (_, jar_name) = self.resolve(software, mc_version).await?;
        let cached = self.software_dir.join(software.as_str()).join(&jar_name).exists();

        if cached && current.map_or(false, |c| c == jar_name) {
            Ok(None)
        } else {
            Ok(Some((current.map(String::from), jar_name)))
        }
    }

    pub async fn minecraft_releases(&self, limit: usize) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Manifest {
            versions: Vec<Version>
        }

        #[derive(Deserialize)]
        struct Version {
            id: String,

            #[serde(rename = "type")]
            kind: String
        }

        let manifest: Manifest = self.get_json("https://launchermeta.mojang.com/mc/game/version_manifest.json").await?;

        Ok(manifest.versions.into_iter().filter(|v| v.kind == "release").take(limit).map(|v| v.id).collect())
    }

    async fn resolve(&self, software: Software, mc_version: &str) -> Result<(String, String)> {
        match software {
            Software::Vanilla => {
                #[derive(Deserialize)]
                struct Manifest {
                    versions: Vec<ManifestEntry>
                }

                #[derive(Deserialize)]
                struct ManifestEntry {
                    id: String,
                    url: String
                }

                #[derive(Deserialize)]
                struct Meta {
                    downloads: Downloads
                }

                #[derive(Deserialize)]
                struct Downloads {
                    server: Asset
                }

                #[derive(Deserialize)]
                struct Asset {
                    url: String
                }

                let manifest: Manifest = self.get_json("https://launchermeta.mojang.com/mc/game/version_manifest.json").await?;

                let entry = manifest.versions.iter().find(|v| v.id == mc_version).ok_or_else(|| anyhow::anyhow!("Version {mc_version} not in Mojang manifest"))?;
                let meta: Meta = self.get_json(&entry.url).await?;
                let name = format!("minecraft_server.{mc_version}.jar");

                Ok((meta.downloads.server.url, name))
            }

            Software::Paper => {
                let url = format!("https://fill.papermc.io/v3/projects/paper/versions/{mc_version}/builds");

                let answer: serde_json::Value = self.get_json(&url).await.context("Paper API error")?;

                let build = answer.as_array()
                    .and_then(|arr| arr.iter().find(|b| b["channel"].as_str() == Some("STABLE")))
                    .ok_or_else(|| anyhow::anyhow!("No stable Paper build for {mc_version}"))?;

                let name = build["downloads"]["server:default"]["name"].as_str().ok_or_else(|| anyhow::anyhow!("Paper: missing download name"))?.to_string();
                let dl = build["downloads"]["server:default"]["url"].as_str().ok_or_else(|| anyhow::anyhow!("Paper: missing download url"))?.to_string();

                Ok((dl, name))
            }

            Software::Fabric => {
                #[derive(Deserialize)]
                struct Entry {
                    version: String
                }

                let loaders: Vec<Entry> = self.get_json("https://meta.fabricmc.net/v2/versions/loader").await?;
                let installers: Vec<Entry> = self.get_json("https://meta.fabricmc.net/v2/versions/installer").await?;

                let loader = &loaders.first().ok_or_else(|| anyhow::anyhow!("No Fabric loaders"))?.version;
                let installer = &installers.first().ok_or_else(|| anyhow::anyhow!("No Fabric installers"))?.version;

                Ok((format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader}/{installer}/server/jar"), format!("fabric-server-mc.{mc_version}-loader.{loader}-launcher.{installer}.jar")))
            }

            _ => Err(anyhow::anyhow!("Unknown software!"))
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        Ok(self.client.get(url).send().await?.error_for_status()?.json::<T>().await?)
    }

    async fn download(&self, url: &str, dest: &Path, label: &str) -> Result<()> {
        println!("Downloading {label}...");

        let bytes = self.client.get(url).send().await.with_context(|| format!("GET {url}"))?.bytes().await?;

        std::fs::write(dest, &bytes)?;
        println!("Downloaded {label}");

        Ok(())
    }
}