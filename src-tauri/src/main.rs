// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Helper elevato: prima di tutto, senza avviare Tauri.
    let args: Vec<String> = std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect();
    if args.get(1).map(String::as_str) == Some("--elevated-helper") {
        let code = match &args[2..] {
            [pipe, token] => modern_commander_lib::elevation::helper::run(pipe, token),
            _ => 2,
        };
        std::process::exit(code);
    }
    modern_commander_lib::run()
}
