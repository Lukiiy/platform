use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::Confirm;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use crate::ui;
use crate::config::{Config, ServerEntry};

pub fn run_server(entry: &ServerEntry, jar_path: &PathBuf) -> Result<()> {
    let config = Config::load()?;

    let eula = entry.path.join("eula.txt");
    if !eula.exists() {
        if !Confirm::new().with_prompt("Accept the Minecraft EULA? (https://aka.ms/MinecraftEULA)").default(false).interact()? {
            ui::warn("EULA not accepted, cancelling.");
            return Ok(());
        }

        std::fs::write(&eula, "eula=true")?;
    }

    let ram = entry.ram_mb;
    let mut jvm = vec![format!("-Xms{}M", ram / 2), format!("-Xmx{}M", ram)];

    jvm.extend(entry.extra_jvm_args.iter().cloned());
    jvm.extend(["-jar".into(), jar_path.to_string_lossy().into_owned(), "--nogui".into()]);

    println!("{}", " Starting ".on_bright_green().bold());

    let java = entry.java_path.as_deref().unwrap_or(&config.app.java_path);

    let mut process = Command::new(java).args(&jvm).current_dir(&entry.path)
        .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().context("Failed to launch Java... Is it installed?")?;

    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();

    let thread_out = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if let Ok(l) = line { println!("{l}"); }
        }
    });

    let thread_error = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if let Ok(l) = line { eprintln!("{}", l.bright_red()); }
        }
    });

    let _ = process.wait();

    println!("{}", " Server process ended. ".on_black().dimmed().bold());

    let _ = thread_out.join();
    let _ = thread_error.join();

    Ok(())
}

pub fn get_custom_jar(server_path: &PathBuf) -> Result<PathBuf> {
    for entry in std::fs::read_dir(server_path)? {
        let path = entry?.path();

        if path.extension().map_or(false, |e| e == "jar") {
            return Ok(path);
        }
    }

    Err(anyhow::anyhow!("No .jar found in \"{}\".", server_path.display()))
}