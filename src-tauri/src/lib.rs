pub mod config;
pub mod error;
pub mod fs;
pub mod paths;
pub mod shell;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(fs::watch::Watchers::default())
        .invoke_handler(tauri::generate_handler![
            fs::list_dir,
            fs::nearest_existing_ancestor,
            fs::list_drives,
            fs::watch::watch_dir,
            fs::watch::unwatch_dir,
            shell::context_menu::show_context_menu,
            shell::properties::show_properties,
            shell::drag::start_drag,
            config::read_config,
            config::write_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
