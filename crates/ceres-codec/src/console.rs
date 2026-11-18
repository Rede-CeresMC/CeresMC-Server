//! Console output for every crate: lines go to one sink (the server colours them and copies the
//! important ones to its log file). Each crate's `println!`/`eprintln!` are redirected here.

use std::sync::OnceLock;

static SINK: OnceLock<fn(&str, bool)> = OnceLock::new();

/// Where the console lines go (set once by the server at start-up).
pub fn set_sink(sink: fn(&str, bool)) {
    let _ = SINK.set(sink);
}

/// One console line (`error` = it was an eprintln).
pub fn line(text: &str, error: bool) {
    match SINK.get() {
        Some(sink) => sink(text, error),
        None if error => std::eprintln!("{text}"),
        None => std::println!("{text}"),
    }
}
