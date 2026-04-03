mod config;
mod foldersync;
mod server;
mod software;
mod ui;

use anyhow::Result;
use colored::Colorize;
use config::{Config, LinkMode, FolderLinks, ServerEntry, Software};
use dialoguer::{Confirm, Input, Select};
use software::SoftwareManager;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load()?;

    std::fs::create_dir_all(config.software_dir())?;
    std::fs::create_dir_all(config.servers_dir())?;

    loop {
        match main_menu().await {
            Ok(true) => {}

            Ok(false) => break,

            Err(e) => {
                ui::err(&e.to_string());
                ui::pause("Press Enter to continue...");
            }
        }
    }

    Ok(())
}

async fn main_menu() -> Result<bool> {
    let config = Config::load()?;

    ui::banner();

    let mut items: Vec<String> = config.servers.iter().map(|it| {
        format!("• {} {} {}", it.name.bright_green().bold(), format!("[{}]", it.software.as_str()).bright_cyan(), it.mc_version.dimmed())
    }).collect();

    let idx_add = items.len();
    items.push("Add server".into());

    let idx_links = items.len();
    items.push("Folder Sync".into());

    let idx_java = items.len();
    items.push("Java settings".into());

    let idx_quit = items.len();
    items.push("Quit".into());

    let sel = Select::new().with_prompt("Select a server or action").items(&items).default(0).interact()?;

    if sel < idx_add {
        server_menu(sel).await?;
    } else if sel == idx_add {
        add_server_menu().await?;
    } else if sel == idx_links {
        plugin_links_menu()?;
    } else if sel == idx_java {
        java_settings_menu()?;
    } else if sel == idx_quit {
        return Ok(false);
    }

    Ok(true)
}

async fn server_menu(index: usize) -> Result<()> {
    let config = Config::load()?;

    loop {
        ui::banner();

        let server = &config.servers[index];

        println!(" {} {}", "Server:".dimmed(), server.name.bold().bright_white());
        println!(" {} {}", "Software:".dimmed(), server.software.as_str().bright_cyan());
        println!(" {} {}", "Version:".dimmed(), server.mc_version.bright_cyan());
        println!("");

        let items = ["Start", "Check software", "Open folder", "Edit settings", "Remove", "Back"];

        match Select::new().with_prompt(&format!("Actions")).items(&items).default(0).interact()? {
            0 => start_server(index).await?,
            1 => check_update_menu(index).await?,
            2 => open_folder(&config.servers[index].path.to_string_lossy()),
            3 => edit_settings(index)?,
            4 => {
                if remove_server(index)? {
                    return Ok(());
                }
            }
            _ => return Ok(())
        }
    }
}

fn open_folder(path: &str) {
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

async fn start_server(index: usize) -> Result<()> {
    let mut config = Config::load()?;
    let entry = config.servers[index].clone();

    let jar_path = if entry.software == Software::Custom {
        server::get_custom_jar(&entry.path)?
    } else {
        println!();

        ui::info("Verifying jar...");

        match SoftwareManager::new(config.software_dir()).ensure_jar(entry.software.as_str(), &entry.mc_version).await {
            Ok((path, jar_name)) => {
                config.servers[index].jar_name = Some(jar_name);
                config.save()?;

                path
            }

            Err(e) => {
                ui::err(&e.to_string());
                ui::pause("Press Enter...");

                return Ok(());
            }
        }
    };

    let servers = config.servers.clone();

    for group in config.folder_syncs.iter().filter(|it| it.servers.contains(&entry.id)) { // sync folder groups
        match foldersync::sync(group, &servers) {
            Ok(r) => ui::ok(&format!("Synced \"{}\"! {r}", group.name)),
            Err(e) => ui::warn(&format!("Failed sync \"{}\": {e}", group.name))
        }
    }

    server::run_server(&config.servers[index], &jar_path)?;

    for group in config.folder_syncs.iter().filter(|it| it.servers.contains(&entry.id)) {
        match foldersync::unsync(group, &servers) {
            Ok(n) => ui::ok(&format!("Unsynced \"{}\"! {n} link(s) removed", group.name)),
            Err(e) => ui::warn(&format!("Failed unsync \"{}\": {e}", group.name))
        }
    }

    Ok(())
}

async fn check_update_menu(index: usize) -> Result<()> {
    let mut config = Config::load()?;

    ui::banner();

    let entry = &config.servers[index];
    if !entry.software.auto_download() {
        ui::warn("Custom software: auto updates are not supported.");
        ui::pause("Press Enter...");

        return Ok(());
    }

    ui::info(&format!("Checking {} {} for updates...", entry.software.as_str(), entry.mc_version));

    let soft_manager = SoftwareManager::new(config.software_dir());

    match soft_manager.check_update(entry.software.as_str(), &entry.mc_version, entry.jar_name.as_deref()).await? {
        None => {
            ui::ok("Already up to date.");
            ui::pause("Press Enter...");
        }

        Some((current, latest)) => {
            if let Some(c) = &current {
                ui::info(&format!("Current: {c}"));
            }

            ui::info(&format!("Latest: {}", latest.bold()));

            if Confirm::new().with_prompt("Download update?").default(true).interact()? {
                match soft_manager.ensure_jar(config.servers[index].software.as_str(), &config.servers[index].mc_version).await {
                    Ok((_, name)) => {
                        config.servers[index].jar_name = Some(name);
                        config.save()?;

                        ui::ok("Updated.");
                    }

                    Err(e) => ui::err(&e.to_string())
                }
            }

            ui::pause("Press Enter...");
        }
    }

    Ok(())
}

fn edit_settings(index: usize) -> Result<()> {
    let mut config = Config::load()?;

    ui::banner();

    let entry = &config.servers[index];
    let ram: String = Input::new().with_prompt("RAM (MB)").default(entry.ram_mb.to_string()).interact_text()?;
    let args: String = Input::new().with_prompt("Extra JVM args (Can leave blank)").default(entry.extra_jvm_args.join(" ")).allow_empty(true).interact_text()?;
    let java: String = Input::new().with_prompt("Java path (Can leave blank)").default(entry.java_path.clone().unwrap_or_default()).allow_empty(true).interact_text()?;

    if let Ok(r) = ram.trim().parse::<u32>() {
        config.servers[index].ram_mb = r;
    } else {
        ui::warn("Invalid RAM, falling back to previous.");
    }

    config.servers[index].extra_jvm_args = args.split_whitespace().map(String::from).collect();
    config.servers[index].java_path = if java.trim().is_empty() { None } else { Some(java.trim().into()) };
    config.save()?;

    ui::ok("Settings saved.");
    ui::pause("Press Enter...");

    Ok(())
}

fn remove_server(index: usize) -> Result<bool> {
    let mut config = Config::load()?;
    let name = config.servers[index].name.clone();

    if Confirm::new().with_prompt(format!("Remove \"{name}\"? (files won't be deleted)")).default(false).interact()? {
        config.servers.remove(index);
        config.save()?;

        ui::ok(&format!("\"{name}\" removed."));
        ui::pause("Press Enter...");

        return Ok(true);
    }

    Ok(false)
}

async fn add_server_menu() -> Result<()> {
    let mut config = Config::load()?;

    ui::banner();

    let action = Select::new().with_prompt("Actions").items(&["Create a new one", "Import existing folder", "Back"]).default(0).interact()?;

    if action == 2 { return Ok(()); }
    let is_new = action == 0;

    let name: String = Input::new().with_prompt("Server name").interact_text()?;
    let slug = slugify(&name);

    let variants = Software::variants();
    let labels: Vec<&str> = variants.iter().map(|(_, l)| *l).collect();
    let sw_sel = Select::new().with_prompt("Software").items(&labels).default(0).interact()?;
    let software = Software::from_str(variants[sw_sel].0);

    let mc_version: String = if software.auto_download() {
        ui::info("Fetching Minecraft versions...");

        match SoftwareManager::new(config.software_dir()).minecraft_releases(30).await {
            Ok(versions) => {
                let idx = Select::new().with_prompt("Minecraft version").items(&versions).default(0).interact()?;

                versions[idx].clone()
            }

            Err(_) => Input::new().with_prompt("Minecraft version").interact_text()?,
        }
    } else {
        Input::new().with_prompt("Minecraft version").interact_text()?
    };

    let ram: String = Input::new().with_prompt("RAM (MB)").default("2048".into()).interact_text()?;
    let ram_mb = ram.trim().parse::<u32>().unwrap_or(2048);

    let server_path = if is_new {
        let p = config.servers_dir().join(&slug);

        std::fs::create_dir_all(&p)?;

        p
    } else {
        let raw: String = Input::new().with_prompt("Path to server folder").interact_text()?;
        let p = std::path::PathBuf::from(raw.trim());

        if !p.exists() {
            ui::err("Path not found.");
            ui::pause("Press Enter...");

            return Ok(());
        }

        p
    };

    config.servers.push(ServerEntry {
        id: slug,
        name: name.clone(),
        path: server_path,
        software,
        mc_version,
        ram_mb,
        extra_jvm_args: vec![],
        jar_name: None,
        java_path: None
    });

    config.save()?;

    ui::ok(&format!("\"{name}\" added!"));
    ui::pause("Press Enter...");

    Ok(())
}

fn plugin_links_menu() -> Result<()> {
    let config = Config::load()?;

    loop {
        ui::banner();

        println!("{}", "Folder Groups".bold().bright_magenta());
        println!("{}\n", "Collections of folders/files that can be shared across servers.".dimmed());

        let mut items: Vec<String> = config.folder_syncs.iter().map(|l| {
            format!("• {}", l.name.bright_green().bold())
        }).collect();

        let idx_new = items.len();

        items.push("Create a new one".into());
        items.push("Back".into());

        let sel = Select::new().with_prompt("Groups").items(&items).default(0).interact()?;
        if sel < idx_new {
            link_action_menu(sel)?;
        } else if sel == idx_new {
            create_link_menu()?;
        } else {
            return Ok(());
        }
    }
}

fn link_action_menu(index: usize) -> Result<()> {
    let mut config = Config::load()?;

    loop {
        ui::banner();

        let link = &config.folder_syncs[index];

        println!("  {} {}", "Group:".dimmed(), link.name.bold().bright_magenta());
        println!("  {} {}", "Mode:".dimmed(), link.mode);
        println!("  {}", "Servers:".dimmed());

        let subscribed: Vec<String> = config.servers.iter().filter(|it| link.servers.contains(&it.id)).map(|it| format!("    ⯁ {}", it.name)).collect();

        if subscribed.is_empty() {
            println!("    (no servers)");
        } else {
            for s in &subscribed {
                println!("{s}");
            }
        }

        let items = ["Open group folder", "Edit toggled servers", "Delete group", "Back"];

        match Select::new().with_prompt("Action").items(&items).default(0).interact()? {
            0 => {
                open_folder(&config.group_dir(&config.folder_syncs[index]).to_string_lossy().into_owned());
            }

            1 => {
                let labels: Vec<String> = config.servers.iter().map(|it| it.name.clone()).collect();
                let current_ids = config.folder_syncs[index].servers.clone();
                let defaults: Vec<bool> = config.servers.iter().map(|it| current_ids.contains(&it.id)).collect();

                let sel = dialoguer::MultiSelect::new().with_prompt("Toggled servers (space to toggle)").items(&labels).defaults(&defaults).interact()?;

                config.folder_syncs[index].servers = sel.into_iter().map(|it| config.servers[it].id.clone()).collect();
                config.save()?;

                ui::ok("Subscriptions updated.");
                ui::pause("Press Enter...");
            }

            2 => {
                if Confirm::new().with_prompt("Delete group? (folder & files will remain)").default(false).interact()? {
                    config.folder_syncs.remove(index);
                    config.save()?;

                    ui::ok("Deleted.");
                    ui::pause("Press Enter...");

                    return Ok(());
                }
            }

            _ => return Ok(())
        }
    }
}

fn create_link_menu() -> Result<()> {
    let mut config = Config::load()?;

    ui::banner();

    println!("  {}", "New Folder group".bold().bright_magenta());
    println!("  {}", "Server files with the same name are never overwritten.".dimmed());
    println!();

    let name: String = Input::new().with_prompt("Group name").interact_text()?;
    let labels: Vec<String> = config.servers.iter().map(|it| it.name.clone()).collect();
    let sel = dialoguer::MultiSelect::new().with_prompt("Toggle for servers [SPACE to toggle, ENTER to confirm]").items(&labels).interact()?;
    let server_ids: Vec<String> = sel.into_iter().map(|it| config.servers[it].id.clone()).collect();

    let mode_sel = Select::new().with_prompt("Sync mode").items(&["Symlink (recommended)", "Copy"]).default(0).interact()?;
    let mode = if mode_sel == 0 { LinkMode::Symlink } else { LinkMode::Copy };

    let link = FolderLinks {
        name: name.clone(),
        servers: server_ids,
        mode,
    };

    let group_dir = config.group_dir(&link);

    std::fs::create_dir_all(&group_dir)?;
    ui::ok(&format!("Group folder created: {}", group_dir.display()));

    open_folder(&group_dir.to_string_lossy());

    config.folder_syncs.push(link);
    config.save()?;

    ui::info("Drop folders and/or into the group folder!");
    ui::pause("Press Enter...");

    Ok(())
}

fn java_settings_menu() -> Result<()> {
    let mut config = Config::load()?;

    ui::banner();

    println!("  {} {}", "Java Path:".dimmed(), config.app.java_path.bright_cyan());

    config.app.java_path = Input::new().with_prompt("Java path").default(config.app.java_path.clone()).interact_text()?.trim().to_string();
    config.save()?;

    ui::ok("Java path updated.");
    ui::pause("Press Enter...");

    Ok(())
}

fn slugify(string: &str) -> String {
    string.to_lowercase().chars().map(|char| if char.is_alphanumeric() { char } else { '_' }).collect::<String>().split('_').filter(|it| !it.is_empty()).collect::<Vec<_>>().join("_")
}