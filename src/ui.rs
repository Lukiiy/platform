use colored::Colorize;

pub fn banner() {
    clear();
    println!();
    println!("{}", "platform: Minecraft Server Manager".bright_green());
    println!();
}

pub fn clear() {
    print!("\x1B[2J\x1B[1;1H"); // clear screen + move cursor to top-left
}

pub fn ok(msg: &str) {
    println!(" {} {}", "Ok".bright_green(), msg);
}

pub fn err(msg: &str) {
    eprintln!(" {} {}", "Error".bright_red(), msg);
}

pub fn info(msg: &str) {
    println!(" {} {}", "*".bright_blue(), msg);
}

pub fn warn(msg: &str) {
    println!(" {} {}", "!".bright_yellow(), msg);
}

/// Pause until enter is pressed
pub fn pause(prompt: &str) {
    println!("\n{}", prompt.dimmed());

    let mut buf = String::new();

    let _ = std::io::stdin().read_line(&mut buf);
}
