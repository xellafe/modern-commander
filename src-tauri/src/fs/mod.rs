//! Comandi Tauri per elenco cartelle e unità. Il lavoro bloccante gira su un thread dedicato.

pub mod columns;
pub mod content;
pub mod drives;
pub mod hash;
pub mod lines;
pub mod listing;
pub mod scan;
pub mod size;
pub mod watch;

use crate::error::{AppError, ErrorCode};
use crate::requests::Requests;
use std::path::{Path, PathBuf};
use tauri::ipc::{Channel, Response};
use tauri::State;

pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, None)
    })
}

#[tauri::command]
pub async fn list_dir(path: String) -> Result<Vec<listing::Entry>, AppError> {
    blocking(move || listing::list_dir(Path::new(&path))).await?
}

#[tauri::command]
pub async fn nearest_existing_ancestor(path: String) -> Result<Option<PathBuf>, AppError> {
    blocking(move || listing::nearest_existing_ancestor(Path::new(&path))).await
}

/// Percorsi in forma lunga (i nomi 8.3 del drop da Explorer); quelli non risolvibili restano invariati.
#[tauri::command]
pub async fn normalize_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, AppError> {
    blocking(move || paths.iter().map(|p| listing::long_path(Path::new(p))).collect()).await
}

#[tauri::command]
pub async fn list_drives()-> Result<Vec<drives::Drive>, AppError> {
    blocking(drives::list_drives).await
}

#[tauri::command]
pub async fn file_info(path: String) -> Result<content::FileInfo, AppError> {
    blocking(move || content::file_info(Path::new(&path))).await?
}

/// I byte arrivano al frontend come `ArrayBuffer`, senza passare da JSON.
#[tauri::command]
pub async fn read_chunk(path: String, offset: u64, len: u32) -> Result<Response, AppError> {
    let bytes = blocking(move || content::read_chunk(Path::new(&path), offset, len)).await??;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn read_file(path: String, max: u64) -> Result<Response, AppError> {
    let bytes = blocking(move || content::read_whole(Path::new(&path), max)).await??;
    Ok(Response::new(bytes))
}

/// Salvataggio dell'editor (vedi `content::write_text_file`); restituisce il `FileInfo` dopo la scrittura.
#[tauri::command]
pub async fn write_text_file(
    path: String,
    text: String,
    encoding: String,
    bom: bool,
    expected_modified_ms: Option<i64>,
) -> Result<content::FileInfo, AppError> {
    blocking(move || content::write_text_file(Path::new(&path), &text, &encoding, bom, expected_modified_ms)).await?
}

/// Etichetta della codifica ANSI di sistema (code page `GetACP`) nel formato di `TextDecoder`.
#[tauri::command]
pub fn ansi_encoding() -> String {
    content::encoding_for_acp(content::ansi_code_page()).to_string()
}

/// Scansione dell'albero sotto `root` per il salto rapido: le voci arrivano a lotti su `on_batch`, il risultato
/// è `true` se si è fermata al limite di voci. Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn scan_tree(
    requests: State<'_, Requests>,
    request_id: u64,
    root: String,
    max_depth: u32,
    max_entries: u32,
    include_hidden: bool,
    on_batch: Channel<Vec<scan::ScanEntry>>,
) -> Result<bool, AppError> {
    let guard = requests.register(request_id);
    let limits = scan::ScanLimits { max_depth, max_entries, include_hidden };
    blocking(move || {
        // Un canale chiuso (palette già chiusa) si scopre con l'annullamento: qui l'invio fallito si ignora.
        scan::scan_tree(Path::new(&root), &limits, guard.flag(), &mut |b| {
            let _ = on_batch.send(b);
        })
    })
    .await?
}

/// Byte dopo il primo LF a `from` o oltre (vedi `lines::next_line_start`); `on_progress` riceve l'offset già cercato.
/// Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn next_line_start(
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    from: u64,
    unit: u8,
    big_endian: bool,
    on_progress: Channel<u64>,
) -> Result<u64, AppError> {
    let guard = requests.register(request_id);
    blocking(move || {
        lines::next_line_start(Path::new(&path), from, unit, big_endian, guard.flag(), &mut |p| {
            let _ = on_progress.send(p);
        })
    })
    .await?
}

/// Inizio della riga che contiene `at` (vedi `lines::line_start`); `on_progress` riceve l'offset più basso già esaminato.
/// Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn line_start(
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    at: u64,
    unit: u8,
    big_endian: bool,
    on_progress: Channel<u64>,
) -> Result<u64, AppError> {
    let guard = requests.register(request_id);
    blocking(move || {
        lines::line_start(Path::new(&path), at, unit, big_endian, guard.flag(), &mut |p| {
            let _ = on_progress.send(p);
        })
    })
    .await?
}

/// Per ogni punto di `from`, in sequenza, il carattere che occupa la colonna `target` o la fine riga (vedi
/// `columns::column_offset`); `on_progress` riceve i punti di controllo con la posizione in `from`.
/// Si annulla con `cancel_request(request_id)` (errore `CANCELLED`), per tutta la richiesta.
#[tauri::command]
pub async fn column_offsets(
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    encoding: String,
    from: Vec<columns::Mark>,
    target: u64,
    on_progress: Channel<columns::ColumnProgress>,
) -> Result<Vec<columns::ColumnHit>, AppError> {
    let guard = requests.register(request_id);
    blocking(move || {
        (0u32..)
            .zip(from)
            .map(|(index, m)| {
                columns::column_offset(Path::new(&path), &encoding, m, target, guard.flag(), &mut |mark| {
                    let _ = on_progress.send(columns::ColumnProgress { index, mark });
                })
            })
            .collect()
    })
    .await?
}

/// Hash del file (esadecimale minuscolo). Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn calc_hash(
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    algo: hash::HashAlgo,
) -> Result<String, AppError> {
    let guard = requests.register(request_id);
    blocking(move || hash::hash_file(Path::new(&path), algo, guard.flag())).await?
}
