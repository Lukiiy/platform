mod config;
mod foldersync;
mod server;
mod software;
mod ui;

use anyhow::Result;
use colored::Colorize;
use config::{Config, ServerEntry};
use dialoguer::{Select, Confirm, Input};
use software::{Software, SoftwareManager};
use foldersync::{FolderLinks, LinkMode};

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = Config::load()?;

    std::fs::create_dir_all(config.software_dir())?;
    std::fs::create_dir_all(config.servers_dir())?;

    loop {
        match main_menu(&mut config).await {
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

async fn main_menu(config: &mut Config) -> Result<bool> {
    ui::banner();

    let mut items: Vec<String> = config.servers.iter().map(|it| {
        format!("• {} {} {}", it.name.bright_green().bold(), format!("[{}]", it.software.as_str()).bright_cyan(), it.mc_version.dimmed())
    }).collect();

    let act = items.len();

    items.push("Add server".into());
    items.push("Folder Sync".into());
    items.push("Global Settings".into());
    items.push("Quit".into());

    match ui::menu("Select a server or action", &items, 0)? {
        i if i < act => server_menu(config, i).await?,
        i if i == act => add_server_menu(config).await?,
        i if i == act + 1 => folder_sync_menu(config)?,
        i if i == act + 2 => global_settings(config)?,
        _ => return Ok(false)
    }

    Ok(true)
}

async fn server_menu(config: &mut Config, index: usize) -> Result<()> {
    loop {
        ui::banner();

        let server = &config.servers[index];

        println!(" {} {}", "Server:".dimmed(), server.name.bold().bright_white());
        println!(" {} {}", "Software:".dimmed(), server.software.as_str().bright_cyan());
        println!(" {} {}", "Version:".dimmed(), server.mc_version.bright_cyan());
        println!();

        match ui::menu("Actions", &["Start", "Software Menu", "Open folder", "Edit settings", "Remove", "Back"], 0)? {
            0 => start_server(index).await?,
            1 => software_menu(config, index).await?,
            2 => open_folder(&config.servers[index].path.to_string_lossy()),
            3 => server_settings(config, index)?,
            4 => {
                if remove_server(config, index)? {
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
    let software = entry.software;
    let manager = SoftwareManager::new(config.software_dir());

    let jar_path = if software == Software::Custom {
        server::get_custom_jar(&entry.path)?
    } else {
        println!();

        ui::info("Verifying jar...");

        match manager.ensure_jar(software, &entry.mc_version).await {
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

    if software.is_installer() { server::set_serverstarter(manager.use_serverstarter().await?); }

    let servers = config.servers.clone();

    for group in config.folder_syncs.iter().filter(|it| it.servers.contains(&entry.id)) { // sync folder groups
        match foldersync::sync(group, &servers) {
            Ok(r) => ui::ok(&format!("Synced {}! {r}", group.name)),
            Err(e) => ui::warn(&format!("Failed sync {}: {e}", group.name))
        }
    }

    server::run_server(&config.servers[index], &jar_path)?;

    for group in config.folder_syncs.iter().filter(|it| it.servers.contains(&entry.id)) {
        match foldersync::unsync(group, &servers) {
            Ok(n) => ui::ok(&format!("Unsynced {}! {n} link(s) removed", group.name)),
            Err(e) => ui::warn(&format!("Failed unsync {}: {e}", group.name))
        }
    }

    Ok(())
}

async fn software_menu(config: &mut Config, index: usize) -> Result<()> {
    let soft_manager = SoftwareManager::new(config.software_dir());

    loop {
        ui::banner();

        println!("{}", "Software Menu".bold().bright_magenta());
        println!("{}", "Change software and/or version for this server.".dimmed());
        println!();

        let entry = &config.servers[index];
        let current_software = entry.software;
        let current_version = entry.mc_version.clone();
        let current_jar = entry.jar_name.clone();

        println!(" {} {} {} {}", "Current:".dimmed(), current_software.as_str().bright_green(), current_version.bright_cyan(), current_jar.as_deref().unwrap_or("???").white().italic());
        println!();

        match ui::menu("Actions", &["Check for update", "Change software/version", "Back"], 0)? {
            0 => {
                if !current_software.auto_download() {
                    ui::warn("Custom software: Auto updates are not supported.");
                    ui::pause("Press Enter...");

                    continue;
                }

                ui::info(&format!("Checking {} {} for updates...", current_software.as_str(), current_version));

                match soft_manager.check_update(current_software, &current_version, current_jar.as_deref()).await? {
                    None => {
                        ui::ok("Already up to date.");
                        ui::pause("Press Enter...");
                    }

                    Some((current, latest)) => {
                        if let Some(c) = current { ui::info(&format!("Current: {c}")); }

                        ui::info(&format!("Latest: {}", latest.bold()));

                        if Confirm::new().with_prompt("Download update?").default(true).interact()? {
                            match soft_manager.ensure_jar(current_software, &current_version).await {
                                Ok((_, name)) => {
                                    let changed = current_jar.as_deref() != Some(name.as_str());

                                    config.servers[index].jar_name = Some(name);
                                    config.save()?;

                                    if changed { ui::ok("Updated."); } else { ui::info("Nothing changed."); }
                                }

                                Err(e) => ui::err(&e.to_string())
                            }
                        }

                        ui::pause("Press Enter...");
                    }
                }
            }

            1 => { // OH MY AAAAAAAAAAAAAAAAAAAAAAAA
                let soft_manager = SoftwareManager::new(config.software_dir());
                let (target_software, target_version) = select_software(&soft_manager).await?;

                if target_software != entry.software { ui::warn("This will change software! May require some reconfiguration."); }
                if target_version != entry.mc_version { ui::warn("This will change version!"); }

                if Confirm::new().with_prompt("Proceed?").default(false).interact()? {
                    match soft_manager.ensure_jar(target_software, &target_version).await {
                        Ok((_, name)) => {
                            let changed = target_software != entry.software || target_version != entry.mc_version || entry.jar_name.as_deref() != Some(name.as_str());

                            config.servers[index].software = target_software;
                            config.servers[index].mc_version = target_version;
                            config.servers[index].jar_name = Some(name);

                            config.save()?;

                            if changed { ui::ok("Changed."); } else { ui::info("Nothing changed."); }
                        }

                        Err(e) => ui::err(&e.to_string())
                    }

                    ui::pause("Press Enter...");
                }
            }

            _ => return Ok(())
        }
    }
}

fn server_settings(config: &mut Config, index: usize) -> Result<()> {
    let mut selected = 0;

    loop {
        ui::banner();

        println!("{}", "Server Settings".bold().bright_magenta());
        println!("{}", "Options that apply only to this server.".dimmed());
        println!();

        let entry = &config.servers[index];

        let ram = format!("RAM: {}", format!("{} MB", entry.ram_mb).bright_cyan().to_string());

        let extra_args = format!("Extra JVM args: {}",
            if entry.extra_jvm_args.is_empty() {
                "(not set)".dimmed().to_string()
            } else {
                entry.extra_jvm_args.join(" ").bright_cyan().to_string()
            }
        );

        let java_path = format!("Java path: {}",
            match entry.java_path.as_deref() {
                Some(path) if !path.trim().is_empty() => path.bright_cyan().to_string(),

                _ => "(Default)".dimmed().to_string()
            }
        );

        let select = Select::new().with_prompt("Settings").items(&[ram, extra_args, java_path, "Back".into()]).default(selected).interact()?;

        selected = select;

        match select {
            0 => {
                let current = config.servers[index].ram_mb.to_string();
                let ram = Input::<String>::new().with_prompt("RAM (MB)").default(current).interact_text()?;

                match ram.trim().parse::<u32>() {
                    Ok(value) => {
                        config.servers[index].ram_mb = value;
                        config.save()?;

                        ui::ok("RAM updated!");
                    }

                    Err(_) => ui::warn("Invalid value, keeping the previous setting.")
                }

                ui::pause("Press Enter...");
            }

            1 => {
                let current = config.servers[index].extra_jvm_args.join(" ");
                let args = Input::<String>::new().with_prompt("Extra JVM args").allow_empty(true).default(current).interact_text()?;

                config.servers[index].extra_jvm_args = args.split_whitespace().map(String::from).collect();
                config.save()?;

                ui::ok("Extra JVM args updated!");
                ui::pause("Press Enter...");
            }

            2 => {
                let current = config.servers[index].java_path.clone().unwrap_or_default();
                let java = Input::<String>::new().with_prompt("Java path").allow_empty(true).default(current).interact_text()?;
                let trimmed = java.trim();

                config.servers[index].java_path = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

                config.save()?;

                ui::ok("Java path updated!");
                ui::pause("Press Enter...");
            }

            _ => return Ok(())
        }
    }
}

fn remove_server(config: &mut Config, index: usize) -> Result<bool> {
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

async fn add_server_menu(config: &mut Config) -> Result<()> {
    ui::banner();

    let action = ui::menu("Actions", &["Create a new one", "Import existing folder", "Back"], 0)?;

    if action == 2 { return Ok(()); }
    let is_new = action == 0;

    let name: String = Input::new().with_prompt("Server name").interact_text()?;
    let id = slugify(&name);

    let (software, mc_version) = select_software(&SoftwareManager::new(config.software_dir())).await?;
    let ram_mb: u32 = 2048;

    let server_path = if is_new {
        let p = config.servers_dir().join(&id);

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
        id,
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

fn folder_sync_menu(config: &mut Config) -> Result<()> {
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

        let sel = ui::menu("Groups", &items, 0)?;

        if sel < idx_new {
            link_action_menu(config, sel)?;
        } else if sel == idx_new {
            create_link_menu(config)?;
        } else {
            return Ok(());
        }
    }
}

fn link_action_menu(config: &mut Config, index: usize) -> Result<()> {
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

        println!();

        match ui::menu("Actions", &["Open group folder", "Edit toggled servers", "Delete group", "Back"], 0)? {
            0 => open_folder(&config.group_dir(&config.folder_syncs[index]).to_string_lossy().into_owned()),

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

fn create_link_menu(config: &mut Config) -> Result<()> {
    ui::banner();

    println!("  {}", "New Folder group".bold().bright_magenta());
    println!("  {}", "Server files with the same name are never overwritten.".dimmed());
    println!();

    let name: String = Input::new().with_prompt("Group name").interact_text()?;
    let labels: Vec<String> = config.servers.iter().map(|it| it.name.clone()).collect();
    let sel = dialoguer::MultiSelect::new().with_prompt("Toggle for servers [SPACE to toggle, ENTER to confirm]").items(&labels).interact()?;
    let server_ids: Vec<String> = sel.into_iter().map(|it| config.servers[it].id.clone()).collect();

    let mode_sel = ui::menu("Sync mode", &["Symlink (recommended)", "Copy"], 0)?;
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

    ui::info("Drop folders and/or files into the group folder!");
    ui::pause("Press Enter...");

    Ok(())
}

fn global_settings(config: &mut Config) -> Result<()> {
    let mut selected = 0;

    loop {
        ui::banner();

        println!("{}", "Global Settings".bold().bright_magenta());
        println!("{}", "Options that apply globally to the app and all servers.".dimmed());
        println!();

        let java_path = format!("Java path: {}",
            if config.app.java_path.trim().is_empty() {
                "(not set)".dimmed().to_string()
            } else {
                config.app.java_path.bright_cyan().to_string()
            }
        );

        let cleaner_log = format!("Cleaner logs: {}", ui::toggleable(config.app.cleaner_log));

        let select = Select::new().with_prompt("Settings").items(&[java_path, cleaner_log, "Back".into()]).default(selected).interact()?;

        selected = select;

        match select {
            0 => {
                config.app.java_path = dialoguer::Input::new().with_prompt("Java path").default(config.app.java_path.clone()).interact_text()?.trim().to_string();
                config.save()?;

                ui::ok("Java path updated.");
                ui::pause("Press Enter...");
            }

            1 => {
                config.app.cleaner_log = !config.app.cleaner_log;
                config.save()?;
            }

            _ => return Ok(())
        }
    }
}

async fn select_software(soft_manager: &SoftwareManager) -> Result<(Software, String)> {
    let labels = Software::menu_labels();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let software = Software::EVERYTHING[ui::menu("Software", &label_refs, 0)?].0;

    let mc_version = if software.auto_download() {
        ui::info("Fetching Minecraft versions...");

        match soft_manager.minecraft_releases(60).await {
            Ok(versions) => versions[ui::menu("Minecraft version", &versions, 0)?].clone(),

            Err(_) => Input::new().with_prompt("Minecraft version").interact_text()?
        }
    } else {
        Input::new().with_prompt("Minecraft version").interact_text()?
    };

    Ok((software, mc_version))
}

fn slugify(string: &str) -> String {
    string.to_lowercase().chars().map(|char| if char.is_alphanumeric() { char } else { '_' }).collect::<String>().split('_').filter(|it| !it.is_empty()).collect::<Vec<_>>().join("_")
}