use anyhow::Result;
use colored::{ColoredString, Colorize};
use dialoguer::Select;
use std::fmt::Display;

/// Prints the little banner.
pub fn banner() {
    clear();
    println!();
    println!("{}", "platform: Minecraft Server Manager".bright_green());
    println!();
}

/// Clears the terminal screen and moves the cursor to the top-left.
pub fn clear() {
    print!("\x1B[2J\x1B[1;1H"); // ANSI codes: "\x1B[2J" to clear screen; "\x1B[1;1H" to move cursor
}

/// Prints a success message!
/// msg: Message
pub fn ok(msg: &str) {
    println!(" {} {}", "Ok".bright_green(), msg);
}

/// Prints an error message.
/// msg: Message
pub fn err(msg: &str) {
    eprintln!(" {} {}", "Error".black().bold().on_bright_red(), msg);
}

/// Prints an info message.
/// msg: Message
pub fn info(msg: &str) {
    println!(" {} {}", "*".bright_blue(), msg);
}

/// Prints a warning message.
/// msg: Message
pub fn warn(msg: &str) {
    println!(" {} {}", "!".bright_yellow(), msg);
}

/// Pause until enter is pressed
/// prompt: Message to display before waiting for input
pub fn pause(prompt: impl Display) {
    println!("\n{}", prompt.to_string().dimmed());

    let mut buf = String::new();

    let _ = std::io::stdin().read_line(&mut buf);
}

/// Orders a dialoguer Select menu and returns the selected index.
/// prompt: Message to display;
/// items: Items in the menu (must implement Display);
/// default: Default index to select
pub fn menu<T>(prompt: impl Into<String>, items: &[T], default: usize) -> Result<usize> where T: Display {
    if items.is_empty() { anyhow::bail!("This menu has no items!"); }

    Ok(Select::new().with_prompt(prompt.into()).items(items).default(default.min(items.len().saturating_sub(1))).interact()?)
}

/// Prints a pretty "toggleable" banner.
/// bool: Whether the banner shows ON or OFF
pub fn toggleable(bool: bool) -> ColoredString {
    if bool {
        " ON ".black().on_bright_green().into()
    } else {
        " OFF ".on_bright_red().into()
    }
}