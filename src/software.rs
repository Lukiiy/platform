use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::LazyLock};

// todo: snapshots & more modded servers?

pub const SERVERSTARTER_JAR: &str = "server_starter.jar";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Software {
    Vanilla,
    Paper,
    Folia,
    Fabric,
    NeoForge,
    Forge,
    Custom
}

impl Software {
    pub const EVERYTHING: &'static [(Self, &'static str, &'static str, &'static str)] = &[
        (Self::Vanilla, "vanilla", "Vanilla", "Mojang server"),
        (Self::Paper, "paper", "Paper", "Plugin support"),
        (Self::Folia, "folia", "Folia", "Multithreaded & Plugin support"),
        (Self::Fabric, "fabric", "Fabric", "Mod support"),
        (Self::NeoForge, "neoforge", "NeoForge", "Mod support"),
        (Self::Forge, "forge", "Forge", "Mod support"),
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

    pub fn is_installer(&self) -> bool { matches!(self, Self::Forge | Self::NeoForge) }

    pub fn menu_labels() -> Vec<String> { Self::EVERYTHING.iter().map(|(_, _, name, desc)| format!("{name} - {desc}")).collect() }

    pub fn log_regex(software: &Software) -> &'static Regex {
        static PAPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[\d{2}:\d{2}:\d{2}\s+([A-Z]+)\]:\s*(.*)$").unwrap());
        static OTHER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[\d{2}:\d{2}:\d{2}\]\s+\[[^/\]]+/([A-Z]+)\]:\s*(.*)$").unwrap());
        static FORGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\[\d{2}:\d{2}:\d{2}\]\s+\[[^/\]]+/([A-Z]+)\](?:\s+\[[^\]]+\])?:\s*(.*)$").unwrap());

        match software {
            Self::Paper | Self::Folia => &PAPER,
            Self::Forge | Self::NeoForge => &FORGE,
            _ => &OTHER
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
        let (_, latest) = self.resolve(software, mc_version).await?;

        if current.map_or(false, |c| c == latest) { return Ok(None); }

        Ok(Some((current.map(String::from), latest)))
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

            Software::Paper => self.resolve_papermc_dls("paper", mc_version, false).await,

            Software::Folia => self.resolve_papermc_dls("folia", mc_version, false).await,

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

            Software::NeoForge => { // why
                let parts: Vec<&str> = mc_version.trim_start_matches("1.").splitn(2, '.').collect();

                let neo_prefix = match parts.as_slice() {
                    [minor] => format!("{minor}.0."),
                    [minor, patch] => format!("{minor}.{patch}."),
                    _ => anyhow::bail!("Unrecognised MC version format: {mc_version}")
                };

                let xml = self.get_text("https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml").await?;
                let version = Self::resolve_maven_version(&xml, |v| v.starts_with(&neo_prefix) && !v.ends_with("-beta")).or_else(|| Self::resolve_maven_version(&xml, |v| v.starts_with(&neo_prefix))).ok_or_else(|| anyhow::anyhow!("No NeoForge version for {mc_version}"))?;
                let jar_name = format!("neoforge-{version}-installer.jar");

                Ok((format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/{jar_name}"), jar_name))
            }

            Software::Forge => {
                let xml = self.get_text("https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml").await?;
                let prefix = format!("{mc_version}-");
                let version = Self::resolve_maven_version(&xml, |v| v.starts_with(&prefix)).ok_or_else(|| anyhow::anyhow!("No Forge version for {mc_version}"))?;
                let jar_name = format!("forge-{version}-installer.jar");

                Ok((format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{version}/{jar_name}"), jar_name))
            }

            _ => Err(anyhow::anyhow!("Unknown software!"))
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        Ok(self.client.get(url).send().await?.error_for_status()?.json::<T>().await?)
    }

    async fn get_text(&self, url: &str) -> Result<String> {
        Ok(self.client.get(url).send().await?.error_for_status()?.text().await?)
    }

    async fn download(&self, url: &str, dest: &Path, label: &str) -> Result<()> {
        println!("Downloading {label}...");

        let bytes = self.client.get(url).send().await.with_context(|| format!("GET {url}"))?.bytes().await?;

        std::fs::write(dest, &bytes)?;
        println!("Downloaded {label}");

        Ok(())
    }

    fn resolve_maven_version(xml: &str, predicate: impl Fn(&str) -> bool) -> Option<String> {
        xml.lines().filter_map(|line| {
            let line = line.trim();

            line.strip_prefix("<version>")?.strip_suffix("</version>")
        }).filter(|it| predicate(it)).last().map(String::from)
    }

    async fn resolve_papermc_dls(&self, project: &str, mc_version: &str, nonstable_fallback: bool) -> Result<(String, String)> { // todo
        #[derive(Deserialize)]
        struct Build {
            channel: Option<String>,

            #[serde(rename = "downloads")]
            downloads: serde_json::Value
        }

        let url = format!("https://fill.papermc.io/v3/projects/{project}/versions/{mc_version}/builds");
        let builds: Vec<Build> = self.get_json(&url).await.with_context(|| format!("{project} API error"))?;

        let build = builds.iter().find(|b| b.channel.as_deref() == Some("STABLE"))
            .or_else(|| {
                if nonstable_fallback { builds.first() } else { None }
            })
            .ok_or_else(|| {
                if nonstable_fallback { anyhow::anyhow!("No Folia build for {mc_version}") } else { anyhow::anyhow!("No stable Paper build for {mc_version}") }
            })?;

        let asset = &build.downloads["server:default"];

        let name = asset["name"].as_str().ok_or_else(|| anyhow::anyhow!("Missing name"))?.to_string();
        let url = asset["url"].as_str().ok_or_else(|| anyhow::anyhow!("Missing url"))?.to_string();

        Ok((url, name))
    }

    pub async fn use_serverstarter(&self) -> Result<PathBuf> {
        let dest = self.software_dir.join(SERVERSTARTER_JAR);

        if !dest.exists() { self.download("https://github.com/NeoForged/ServerStarterJar/releases/latest/download/server.jar", &dest, "ServerStarterJar").await?; }

        Ok(dest)
    }
}