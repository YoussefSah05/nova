// Prevents a console window from appearing on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use nova_transcribe::streaming::worker;

fn main() {
    // The same binary serves as Nova's ASR worker process (ADR-0011): no
    // window, no Tauri, just whisper.cpp behind a pipe protocol.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(worker::WORKER_FLAG) {
        std::process::exit(worker::run(&args[1..]));
    }
    nova_desktop_lib::run();
}
