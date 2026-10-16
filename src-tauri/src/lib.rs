pub mod config;
pub mod elevation;
pub mod error;
pub mod fs;
pub mod mask;
pub mod ops;
pub mod paths;
pub mod shell;

use ops::queue::{Event, Queue};
use std::sync::Arc;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(fs::watch::Watchers::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let sink = Arc::new(move |e: Event| {
                let _ = match e {
                    Event::Updated(v) => handle.emit("job-updated", v),
                    Event::Prompt(p) => handle.emit("job-prompt", p),
                };
            });
            app.manage(Queue::new(Arc::new(ops::exec::LocalExecutor), Arc::new(elevation::shared), sink));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fs::list_dir,
            fs::nearest_existing_ancestor,
            fs::list_drives,
            fs::normalize_paths,
            fs::watch::watch_dir,
            fs::watch::unwatch_dir,
            shell::context_menu::show_context_menu,
            shell::properties::show_properties,
            shell::drag::start_drag,
            config::read_config,
            config::write_config,
            ops::queue::enqueue_job,
            ops::queue::pause_job,
            ops::queue::resume_job,
            ops::queue::cancel_job,
            ops::queue::move_job,
            ops::queue::reply_prompt,
            ops::queue::list_jobs,
            ops::queue::clear_finished,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
