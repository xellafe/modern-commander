pub mod error;
pub mod fs;
pub mod paths;
pub mod shell;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            fs::list_dir,
            fs::nearest_existing_ancestor,
            fs::list_drives,
            shell::context_menu::show_context_menu,
            shell::properties::show_properties,
            shell::drag::start_drag,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
