pub mod archive;
pub mod config;
pub mod elevation;
pub mod error;
pub mod fs;
pub mod mask;
pub mod ops;
pub mod paths;
pub mod requests;
pub mod shell;
pub mod terminal;

use ops::queue::{Event, Queue};
use std::sync::Arc;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(fs::watch::Watchers::default())
        .manage(requests::Requests::default())
        .manage(terminal::Terminals::default())
        // Un ricaricamento della pagina (anche il reload di Vite) perde i canali e non esegue la pulizia dei
        // componenti: i terminali aperti si chiudono qui. All'avvio non ce n'è nessuno: nessun effetto.
        // ponytail: vale con la finestra unica; con più finestre servirebbe la proprietà della sessione per webview.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                webview.state::<terminal::Terminals>().kill_all();
            }
        })
        .setup(|app| {
            if let Some(Ok(hwnd)) = app.get_webview_window("main").map(|w| w.hwnd()) {
                elevation::set_owner_window(hwnd);
            }
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
            fs::size::calc_dir_size,
            fs::scan_tree,
            fs::next_line_start,
            fs::line_start,
            fs::column_offsets,
            fs::calc_hash,
            archive::list_archive,
            requests::cancel_request,
            fs::file_info,
            fs::read_chunk,
            fs::read_file,
            fs::write_text_file,
            fs::ansi_encoding,
            shell::context_menu::show_context_menu,
            shell::properties::show_properties,
            shell::drag::start_drag,
            shell::open::open_path,
            shell::open::open_with,
            shell::thumbnail::get_thumbnail,
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
            terminal::term_shells,
            terminal::term_spawn,
            terminal::term_write,
            terminal::term_resize,
            terminal::term_kill,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Nessun processo orfano: i terminali aperti si fermano con l'app.
            if let tauri::RunEvent::Exit = event {
                app.state::<terminal::Terminals>().kill_all();
            }
        });
}
