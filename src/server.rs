use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::Confirm;
use std::io::{BufRead, BufReader, Write};
use std::path::{PathBuf, Path};
use std::process::{Command, Stdio, Child};
use std::thread;
use std::sync::{LazyLock, OnceLock};
use regex::Regex;

use crate::ui;
use crate::config::{Config, ServerEntry};
use crate::software::Software;

static SERVERSTARTER_PATH: OnceLock<PathBuf> = OnceLock::new();
static TERMINAL_FILTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1B\[[0-?]*[ -/]*[@-~]").unwrap());

pub fn run_server(entry: &ServerEntry, jar_path: &Path) -> Result<()> {
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
    let java = entry.java_path.as_deref().unwrap_or(&config.app.java_path);

    if entry.software.is_installer() {
        let installed = if cfg!(windows) {
            entry.path.join("run.bat").exists()
        } else {
            entry.path.join("run.sh").exists()
        };

        if !installed {
            println!("{}", " Installing... ".black().on_bright_yellow().bold());

            let status = Command::new(java).arg("-jar").arg(jar_path).arg("--installServer").current_dir(&entry.path)
                .status().context("Failed to run installer. Is Java installed?")?;

            if !status.success() {
                return Err(anyhow::anyhow!("Installer exited with: {}", status));
            }
        }

        let mut jvm_args = vec![format!("-Xms{}M", ram / 2), format!("-Xmx{}M", ram)];
        jvm_args.extend(entry.extra_jvm_args.iter().cloned());

        let args_path = entry.path.join("user_jvm_args.txt");
        std::fs::write(&args_path, format!("{}\n", jvm_args.join("\n")))?;

        let starter = SERVERSTARTER_PATH.get().ok_or_else(|| anyhow::anyhow!("ServerStarter not found!"))?;

        let mut process = Command::new(java).arg(format!("@{}", args_path.display())).arg("-jar").arg(starter).arg("--nogui")
            .current_dir(&entry.path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().context("Failed to launch server. Did installation succeed?")?;

        let mut child_stdin = process.stdin.take().unwrap();
        let mut shutdown_pipe = [0i32; 2]; // wake poll() so stdin thread exits on shut

        unsafe { libc::pipe(shutdown_pipe.as_mut_ptr()) };

        let stop_r = shutdown_pipe[0];
        let stop_w = shutdown_pipe[1];

        thread::spawn(move || { // stdin to child; exits on stop signal
            let mut line = String::new();

            loop { // wait for stdin/stop
                let mut poll_set = [
                    libc::pollfd {
                        fd: 0,
                        events: libc::POLLIN,
                        revents: 0,
                    },

                    libc::pollfd {
                        fd: stop_r,
                        events: libc::POLLIN,
                        revents: 0,
                    }
                ];

                let result = unsafe { libc::poll(poll_set.as_mut_ptr(), 2, -1) };

                if result <= 0 { break; }
                if poll_set[1].revents != 0 { break; } // stop signal
                if poll_set[0].revents & libc::POLLIN == 0 { continue; } // no data

                line.clear();

                match std::io::stdin().read_line(&mut line) { // poll already checked
                    Ok(0) | Err(_) => break,

                    Ok(_) => {
                        if child_stdin.write_all(line.as_bytes()).is_err() { break; }
                    }
                }
            }

            unsafe { libc::close(stop_r) };
        });

        println!("{}", " Starting ".black().on_bright_green().bold());

        stream_process(&config, &mut process, &entry.software);
        unsafe { libc::close(stop_w) };

        ui::pause(format!("{}\nPress Enter...", " Server process ended. ".on_black().dimmed().bold()));
        return Ok(());
    }

    let mut jvm = vec![format!("-Xms{}M", ram / 2), format!("-Xmx{}M", ram)];
    jvm.extend(entry.extra_jvm_args.iter().cloned());
    jvm.extend(["-jar".into(), jar_path.to_string_lossy().into_owned(), "--nogui".into()]);

    let java = entry.java_path.as_deref().unwrap_or(&config.app.java_path);

    let mut process = Command::new(java).args(&jvm).current_dir(&entry.path)
        .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().context("Failed to launch Java... Is it installed?")?;

    println!("{}", " Starting ".black().on_bright_green().bold());

    stream_process(&config, &mut process, &entry.software);

    ui::pause(format!("{}\nPress Enter...", " Server process ended. ".on_black().dimmed().bold()));
    Ok(())
}

fn stream_process(config: &Config, process: &mut Child, software: &Software) {
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();
    let regex = Software::log_regex(software).clone();
    let cleaner = config.app.cleaner_log;

    let thread_out = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            let line = remove_term_noise(&line);
            if line.is_empty() { continue; }

            if cleaner {
                println!("{}", format_line(&line, &regex));
            } else {
                println!("{line}");
            }
        }
    });

    let thread_err = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().flatten() {
            eprintln!("{}", line.bright_red());
        }
    });

    let _ = process.wait();
    let _ = thread_out.join();
    let _ = thread_err.join();
}

fn remove_term_noise(line: &str) -> String {
    let mut out = TERMINAL_FILTER.replace_all(line, "").into_owned();

    out.retain(|c| !c.is_control());
    out.trim().to_string()
}

pub fn get_custom_jar(server_path: &Path) -> Result<PathBuf> {
    for entry in std::fs::read_dir(server_path)? {
        let path = entry?.path();

        if path.extension().map_or(false, |e| e == "jar") { return Ok(path); }
    }

    Err(anyhow::anyhow!("No .jar found in \"{}\".", server_path.display()))
}

fn format_line(line: &str, regex: &Regex) -> String {
    if let Some(caps) = regex.captures(line) {
        let level = caps.get(1).map(|it| it.as_str()).unwrap_or("");
        let msg = caps.get(2).map(|it| it.as_str()).unwrap_or("");

        match level {
            "INFO" => format!("{} {msg}", "!".bright_blue()),
            "WARN" => format!("{} {msg}", "⚠".bright_yellow()),
            "ERROR" => format!("{} {msg}", "×".bright_red()),
            "DEBUG" => format!("{} {msg}", "⌕".bright_purple()),
            _ => msg.to_string()
        }
    } else {
        line.to_string()
    }
}

pub fn set_serverstarter(path: PathBuf) {
    let _ = SERVERSTARTER_PATH.set(path);
}