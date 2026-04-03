use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub app: Settings,

    #[serde(default)]
    pub servers: Vec<ServerEntry>,

    #[serde(default)]
    pub folder_syncs: Vec<FolderLinks>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub data_dir: PathBuf,

    #[serde(default)]
    pub java_path: String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub software: Software,
    pub mc_version: String,
    pub ram_mb: u32,

    #[serde(default)]
    pub extra_jvm_args: Vec<String>,

    pub jar_name: Option<String>,
    pub java_path: Option<String>
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Software {
    Paper, Vanilla, Fabric, Custom
}

impl Software {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::Vanilla => "vanilla",
            Self::Fabric => "fabric",
            Self::Custom => "custom"
        }
    }

    pub fn auto_download(&self) -> bool {
        !matches!(self, Self::Custom)
    }

    pub fn variants() -> &'static [(&'static str, &'static str)] { &[
        ("vanilla", "Vanilla - Mojang server"),
        ("paper", "Paper - Plugin support"),
        ("fabric", "Fabric - Mod support"),
        ("custom", "Custom - Your own jar")
    ] }

    pub fn from_str(string: &str) -> Self {
        match string {
            "vanilla" => Self::Vanilla,
            "paper" => Self::Paper,
            "fabric" => Self::Fabric,
            _ => Self::Custom
        }
    }
}

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
    #[default] Symlink, Copy
}

impl fmt::Display for LinkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Symlink => "symlink",
            Self::Copy => "copy"
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("platform");
        let java_path = "java".into();

        Config {
            app: Settings {
                data_dir,
                java_path
            },

            servers: vec![],
            folder_syncs: vec![]
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("platform").join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();

        if !path.exists() {
            let def = Self::default();

            def.save()?;

            return Ok(def);
        }

        Ok(toml::from_str(&std::fs::read_to_string(&path)?)?)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();

        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, toml::to_string_pretty(self)?)?;

        Ok(())
    }

    pub fn software_dir(&self) -> PathBuf {
        self.app.data_dir.join("software")
    }

    pub fn servers_dir(&self) -> PathBuf {
        self.app.data_dir.join("servers")
    }

    pub fn synced_folders_dir(&self) -> PathBuf {
        self.app.data_dir.join("syncedFolders")
    }

    pub fn group_dir(&self, group: &FolderLinks) -> PathBuf {
        self.synced_folders_dir().join(&group.name)
    }
}