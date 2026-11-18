//! Coloured console: each part of the log gets its colour, warnings are yellow, errors red, and
//! Minecraft colour codes (§a, §c...) in messages become real colours. Only on a terminal:
//! redirected output (files, tests) stays plain.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static COLOURS: AtomicBool = AtomicBool::new(false);

const RESET: &str = "\x1b[0m";

/// Turns colours on when stdout is a terminal (and enables ANSI on the Windows console).
pub fn init() {
    if std::env::var_os("NO_COLOR").is_some() || !std::io::stdout().is_terminal() {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
            SetConsoleMode,
        };
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = GetStdHandle(which);
            let mut mode = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
    COLOURS.store(true, Ordering::Relaxed);
}

/// ANSI code of a Minecraft colour/format code.
fn minecraft_code(c: char) -> Option<&'static str> {
    Some(match c.to_ascii_lowercase() {
        '0' => "\x1b[30m",
        '1' => "\x1b[34m",
        '2' => "\x1b[32m",
        '3' => "\x1b[36m",
        '4' => "\x1b[31m",
        '5' => "\x1b[35m",
        '6' => "\x1b[33m",
        '7' => "\x1b[37m",
        '8' => "\x1b[90m",
        '9' => "\x1b[94m",
        'a' => "\x1b[92m",
        'b' => "\x1b[96m",
        'c' => "\x1b[91m",
        'd' => "\x1b[95m",
        'e' => "\x1b[93m",
        'f' => "\x1b[97m",
        'l' => "\x1b[1m",
        'o' => "\x1b[3m",
        'n' => "\x1b[4m",
        'm' => "\x1b[9m",
        'r' => RESET,
        'k' => "",
        _ => return None,
    })
}

/// Colour of a log line by its tag.
fn tag_colour(line: &str, error: bool) -> &'static str {
    let tag = line.split(']').next().unwrap_or("");
    let lower = line.to_ascii_lowercase();
    if lower.contains("warning") {
        return "\x1b[93m";
    }
    if error && (lower.contains("fail") || lower.contains("error") || lower.contains("refused") || lower.contains("panick")) {
        return "\x1b[91m";
    }
    match tag {
        "[Net" | "[RakNet" => "\x1b[36m",
        "[NetherNet" => "\x1b[96m",
        "[Game" => "\x1b[92m",
        "[Auth" => "\x1b[93m",
        "[Command" => "\x1b[95m",
        "[Server" => "\x1b[94m",
        "[Debug" => "\x1b[90m",
        "[Data" => "\x1b[90m",
        t if t.starts_with("[World") => "\x1b[34m",
        _ if line.starts_with("Done (") => "\x1b[1;92m",
        _ if line.starts_with("CeresMC Server") => "\x1b[1;96m",
        _ if error => "\x1b[33m",
        _ => "",
    }
}

/// The line with ANSI colours (or plain, without Minecraft codes, when colours are off).
fn render(line: &str, error: bool) -> String {
    let colours = COLOURS.load(Ordering::Relaxed);
    let base = if colours { tag_colour(line, error) } else { "" };
    let mut out = String::with_capacity(line.len() + 16);
    out.push_str(base);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§' {
            if let Some(code) = chars.peek().copied().and_then(minecraft_code) {
                chars.next();
                if colours {
                    out.push_str(if code == RESET { base_or_reset(base) } else { code });
                }
                continue;
            }
        }
        out.push(c);
    }
    if colours {
        out.push_str(RESET);
    }
    out
}

fn base_or_reset(base: &'static str) -> &'static str {
    if base.is_empty() { RESET } else { base }
}

/// The console sink of every crate.
pub fn print(line: &str, error: bool) {
    let text = render(line, error);
    if error {
        let _ = writeln!(std::io::stderr(), "{text}");
    } else {
        let _ = writeln!(std::io::stdout(), "{text}");
    }
}
