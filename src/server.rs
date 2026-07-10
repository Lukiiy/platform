use anyhow::{Context, Result};
use colored::Colorize;
use dialoguer::Confirm;
use std::io::{BufRead, BufReader, Write, stdin};
use std::path::{PathBuf, Path};
use std::process::{Command, Stdio, Child};
use std::thread;
use std::fs;
use std::sync::{LazyLock, OnceLock};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::time::Duration;
use regex::Regex;

use crate::ui;
use crate::config::{Config, ServerEntry};
use crate::software::Software;

static SERVERSTARTER_PATH: OnceLock<PathBuf> = OnceLock::new();
static TERMINAL_FILTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1B\[[0-?]*[ -/]*[@-~]").unwrap());
const THREAD_STACK_SIZE: usize = 262144; // 256 * 1024

pub fn run_server(entry: &mut ServerEntry, jar_path: &Path) -> Result<()> {
    let config = Config::load()?;

    const INSTALLER_ERR: &str = "Failed to launch the installer!";
    const SERVER_ERR: &str = "Failed to start the server... Is it installed? Is Java also installed?";

    if entry.software != Software::Custom {
        let eula = entry.path.join("eula.txt");

        if !eula.exists() {
            if !Confirm::new().with_prompt("Accept the Minecraft EULA? (https://aka.ms/MinecraftEULA)").default(false).interact()? {
                ui::warn("EULA not accepted, cancelling.");

                return Ok(());
            }

            fs::write(&eula, "eula=true")?;
        }
    }

    let ram = entry.ram_mb;
    let java = entry.java_path.as_deref().unwrap_or(&config.app.java_path);

    if entry.software.is_installer() {
        if !entry.installed {
            println!("{}", " Installing... ".black().on_bright_yellow().bold());

            let status = Command::new(java).arg("-jar").arg(jar_path).arg("--installServer")
                .current_dir(&entry.path).status().context(INSTALLER_ERR)?;

            if !status.success() {
                return Err(anyhow::anyhow!("Installer exited with: {}", status));
            }

            entry.installed = true;
        }

        let mut jvm_args = vec![format!("-Xms{}M", ram / 2), format!("-Xmx{}M", ram)];
        jvm_args.extend(entry.extra_jvm_args.iter().cloned());

        let args_path = entry.path.join("user_jvm_args.txt");
        fs::write(&args_path, format!("{}\n", jvm_args.join("\n")))?;

        let has_script = if cfg!(windows) {
            entry.path.join("run.bat").exists()
        } else {
            entry.path.join("run.sh").exists()
        };

        let mut process = if has_script { // modern Forge/NeoForge
            let starter = SERVERSTARTER_PATH.get().ok_or_else(|| anyhow::anyhow!("ServerStarter not found!"))?;

            Command::new(java).arg(format!("@{}", args_path.display())).arg("-jar").arg(starter).arg("--nogui")
                .current_dir(&entry.path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                .spawn().context(SERVER_ERR)?
        } else { // old Forge
            let server_jar = fs::read_dir(&entry.path)?
                .filter_map(|e| e.ok().map(|e| e.path())).find(|p| { p.extension().map_or(false, |e| e == "jar") && p.file_name().map(|n| n.to_string_lossy().contains("server")).unwrap_or(false) }).ok_or_else(|| anyhow::anyhow!(SERVER_ERR))?;

            let mut jvm = vec![format!("-Xms{}M", ram / 2), format!("-Xmx{}M", ram)];

            jvm.extend(entry.extra_jvm_args.iter().cloned());
            jvm.extend(["-jar".into(), server_jar.to_string_lossy().into_owned(), "--nogui".into()]);

            Command::new(java).args(&jvm).current_dir(&entry.path)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                .spawn().context(SERVER_ERR)?
        };

        let stop = run_stdin_relay(&mut process);

        println!("{}", " Starting ".black().on_bright_green().bold());
        stream_process(&config, &mut process, &entry.software);
        stop_relay(stop);

        ui::pause(format!("{}\nPress Enter...", " Server process ended. ".on_black().dimmed().bold()));
        return Ok(());
    }

    let mut jvm = vec![format!("-Xms{}M", ram / 2), format!("-Xmx{}M", ram)];
    jvm.extend(entry.extra_jvm_args.iter().cloned());
    jvm.extend(["-jar".into(), jar_path.to_string_lossy().into_owned(), "--nogui".into()]);

    let java = entry.java_path.as_deref().unwrap_or(&config.app.java_path);

    let mut process = Command::new(java).args(&jvm).current_dir(&entry.path)
        .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().context(SERVER_ERR)?;

    println!("{}", " Starting ".black().on_bright_green().bold());

    stream_process(&config, &mut process, &entry.software);

    ui::pause(format!("{}\nPress Enter...", " Server process ended. ".on_black().dimmed().bold()));
    Ok(())
}

#[cfg(unix)]
/// Relays stdin to the child process, exiting on stop signal
fn run_stdin_relay(process: &mut Child) -> i32 {
    let mut child_stdin = process.stdin.take().unwrap();
    let mut shutdown_pipe = [0i32; 2]; // wake poll() so stdin thread exits on shut

    unsafe { libc::pipe(shutdown_pipe.as_mut_ptr()) }; // pipe used as stop signal

    let stop_r = shutdown_pipe[0];
    let stop_w = shutdown_pipe[1];

    let _ = thread::Builder::new().stack_size(THREAD_STACK_SIZE)
        .spawn(move || { // stdin to child; exits on stop signal
            let mut line = String::new();

            loop { // wait for stdin/stop
                let mut poll_set = [
                    libc::pollfd {
                        fd: 0,
                        events: libc::POLLIN,
                        revents: 0
                    },

                    libc::pollfd {
                        fd: stop_r,
                        events: libc::POLLIN,
                        revents: 0
                    }
                ];

                let result = unsafe { libc::poll(poll_set.as_mut_ptr(), 2, -1) };

                if result <= 0 { break; }
                if poll_set[1].revents != 0 { break; } // stop signal
                if poll_set[0].revents & libc::POLLIN == 0 { continue; } // no data

                line.clear();

                match stdin().read_line(&mut line) { // poll already checked
                    Ok(0) | Err(_) => break,

                    Ok(_) => {
                        if child_stdin.write_all(line.as_bytes()).is_err() { break; }
                    }
                }
            }

        stop_relay(stop_r);
    }).expect("Failed to run relay thread!");

    stop_w // caller closes this to quit
}

#[cfg(unix)]
/// Signals relay thread to quit
fn stop_relay(stop_w: i32) {
    unsafe { libc::close(stop_w) };
}

#[cfg(windows)]
/// Relays stdin to the child process, exiting on stop signal
fn run_stdin_relay(process: &mut Child) -> Arc<AtomicBool> {
    use windows_sys::Win32::System::Console::*;
    use windows_sys::Win32::Foundation::*;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let mut child_stdin = process.stdin.take().unwrap();

    thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        let stdin_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut line = String::new();

        loop {
            if stop_clone.load(Ordering::Relaxed) { break; } // server stopped, quit

            // poll before read_line to avoid blocking after server exits
            let mut count = 0u32;

            unsafe { GetNumberOfConsoleInputEvents(stdin_handle, &mut count) };

            if count == 0 { // no input pending; sleep briefly to avoid spinning
                thread::sleep(Duration::from_millis(20));

                continue;
            }

            line.clear();

            match stdin().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => if child_stdin.write_all(line.as_bytes()).is_err() { break }
            }
        }
    }).unwrap();

    stop
}

#[cfg(windows)]
/// Signals relay thread to quit
fn stop_relay(stop: Arc<AtomicBool>) {
    stop.store(true, Ordering::Relaxed);
}

fn stream_process(config: &Config, process: &mut Child, software: &Software) {
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();
    let regex = Software::log_regex(software).clone();
    let log_cleaner = config.app.cleaner_log;

    let thread_out = thread::Builder::new().stack_size(THREAD_STACK_SIZE)
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();

            while reader.read_line(&mut line).unwrap_or(0) != 0 {
                let cleaned = remove_term_noise(&line);

                line.clear();

                if cleaned.is_empty() {
                    continue;
                }

                if !log_cleaner {
                    println!("{cleaned}");
                    continue;
                }

                println!("{}", format_line(&cleaned, &regex));
            }
        }).unwrap();

    let thread_err = thread::Builder::new().stack_size(THREAD_STACK_SIZE)
        .spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();

            while reader.read_line(&mut line).unwrap_or(0) != 0 {
                eprintln!("{}", line.trim_end_matches(['\n', '\r']).bright_red());

                line.clear();
            }
        }).unwrap();

    let _ = process.wait();
    let _ = thread_out.join();
    let _ = thread_err.join();
}

fn remove_term_noise(line: &str) -> String {
    let replaced = TERMINAL_FILTER.replace_all(line, "");
    let trimmed = replaced.trim();

    if trimmed.chars().any(|c| c.is_control()) {
        let mut out = trimmed.to_string();

        out.retain(|c| !c.is_control());

        out
    } else {
        trimmed.to_string()
    }
}

pub fn get_custom_jar(server_path: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(server_path)? {
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