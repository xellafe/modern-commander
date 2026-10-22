//! Comandi Tauri per elenco cartelle e unità. Il lavoro bloccante gira su un thread dedicato.

pub mod columns;
pub mod content;
pub mod drives;
pub mod hash;
pub mod lines;
pub mod listing;
pub mod scan;
pub mod shares;
pub mod size;
pub mod source;
pub mod watch;

use crate::error::{AppError, ErrorCode};
use crate::requests::Requests;
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Provider, Purpose};
use crate::vfs::router::Vfs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::ipc::{Channel, Response};
use tauri::State;

pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError {
        detail: Some(e.to_string()),
        ..AppError::new(ErrorCode::Io, None)
    })
}

/// Percorso locale per i comandi che sulle sorgenti remote non valgono: `UNSUPPORTED` per una posizione remota,
/// che così non arriva mai a `std::fs` né alla shell.
pub(crate) fn local_only(path: &str) -> Result<PathBuf, AppError> {
    match Loc::parse(path)? {
        Loc::Local(p) => Ok(p),
        Loc::Remote { .. } => Err(AppError::new(ErrorCode::Unsupported, None)),
    }
}

/// `f` sul provider di una posizione remota, fuori dal thread principale.
async fn on_remote<T: Send + 'static>(
    vfs: &Arc<Vfs>,
    loc: Loc,
    f: impl FnOnce(&dyn Provider, &str) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let vfs = vfs.clone();
    blocking(move || {
        let (p, path) = vfs.remote(&loc, Purpose::Browse)?;
        f(&*p, &path)
    })
    .await?
}

#[tauri::command]
pub async fn list_dir(vfs: State<'_, Arc<Vfs>>, path: String) -> Result<Vec<listing::Entry>, AppError> {
    match Loc::parse(&path)? {
        Loc::Local(p) => match shares::is_server_root(&p) {
            // Server non SMB (`\\wsl$`, `\\wsl.localhost`): se l'elenco delle condivisioni fallisce vale il listing di sempre,
            // e solo se fallisce anche quello si restituisce l'errore delle condivisioni.
            Some(server) => blocking(move || shares::list_shares(&server).or_else(|e| listing::list_dir(&p).map_err(|_| e))).await?,
            None => blocking(move || listing::list_dir(&p)).await?,
        },
        loc => on_remote(&vfs, loc, |p, path| Ok(p.list(path)?.into_iter().map(listing::Entry::from).collect())).await,
    }
}

#[tauri::command]
pub async fn nearest_existing_ancestor(path: String) -> Result<Option<PathBuf>, AppError> {
    let path = local_only(&path)?;
    blocking(move || listing::nearest_existing_ancestor(&path)).await
}

/// Percorsi in forma lunga (i nomi 8.3 del drop da Explorer); quelli non risolvibili, e le posizioni remote,
/// restano invariati.
#[tauri::command]
pub async fn normalize_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, AppError> {
    blocking(move || {
        paths
            .iter()
            .map(|p| match Loc::parse(p) {
                Ok(Loc::Remote { .. }) => PathBuf::from(p),
                _ => listing::long_path(Path::new(p)),
            })
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn list_drives()-> Result<Vec<drives::Drive>, AppError> {
    blocking(drives::list_drives).await
}

#[tauri::command]
pub async fn file_info(vfs: State<'_, Arc<Vfs>>, path: String) -> Result<content::FileInfo, AppError> {
    match Loc::parse(&path)? {
        Loc::Local(p) => blocking(move || content::file_info(&p)).await?,
        loc => on_remote(&vfs, loc, content::remote_file_info).await,
    }
}

/// I byte arrivano al frontend come `ArrayBuffer`, senza passare da JSON.
#[tauri::command]
pub async fn read_chunk(vfs: State<'_, Arc<Vfs>>, path: String, offset: u64, len: u32) -> Result<Response, AppError> {
    let bytes = match Loc::parse(&path)? {
        Loc::Local(p) => blocking(move || content::read_chunk(&p, offset, len)).await??,
        loc => on_remote(&vfs, loc, move |p, path| content::remote_chunk(p, path, offset, len)).await?,
    };
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn read_file(vfs: State<'_, Arc<Vfs>>, path: String, max: u64) -> Result<Response, AppError> {
    let bytes = match Loc::parse(&path)? {
        Loc::Local(p) => blocking(move || content::read_whole(&p, max)).await??,
        loc => on_remote(&vfs, loc, move |p, path| content::remote_whole(p, path, max)).await?,
    };
    Ok(Response::new(bytes))
}

/// Salvataggio dell'editor (vedi `content::write_text_file`); restituisce il `FileInfo` dopo la scrittura. Sul
/// remoto (`content::remote_write_text_file`) conta anche `expected_size`, e `allow_unsafe` permette la scrittura
/// diretta dove il server non accetta quella sicura.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn write_text_file(
    vfs: State<'_, Arc<Vfs>>,
    path: String,
    text: String,
    encoding: String,
    bom: bool,
    expected_modified_ms: Option<i64>,
    expected_size: Option<u64>,
    allow_unsafe: bool,
) -> Result<content::FileInfo, AppError> {
    match Loc::parse(&path)? {
        Loc::Local(p) => blocking(move || content::write_text_file(&p, &text, &encoding, bom, expected_modified_ms)).await?,
        loc => {
            let vfs = vfs.inner().clone();
            blocking(move || {
                content::remote_write_text_file(&vfs, &loc, &text, &encoding, bom, expected_modified_ms, expected_size, allow_unsafe)
            })
            .await?
        }
    }
}

/// Scarica un file remoto nella cartella temporanea dell'app (vedi `net::temp::fetch_in`) e ne restituisce il
/// percorso locale; `on_progress` riceve i byte scaricati. Si annulla con `cancel_request(request_id)` (errore
/// `CANCELLED`, senza file parziali).
#[tauri::command]
pub async fn fetch_temp(
    vfs: State<'_, Arc<Vfs>>,
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    on_progress: Channel<u64>,
) -> Result<PathBuf, AppError> {
    let (loc, vfs) = (Loc::parse(&path)?, vfs.inner().clone());
    let guard = requests.register(request_id);
    blocking(move || {
        crate::net::temp::fetch_in(&vfs, &loc, &crate::net::temp::dir(), guard.flag(), &mut |n| {
            let _ = on_progress.send(n);
        })
    })
    .await?
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
    let root = local_only(&root)?;
    let guard = requests.register(request_id);
    let limits = scan::ScanLimits { max_depth, max_entries, include_hidden };
    blocking(move || {
        // Un canale chiuso (palette già chiusa) si scopre con l'annullamento: qui l'invio fallito si ignora.
        scan::scan_tree(&root, &limits, guard.flag(), &mut |b| {
            let _ = on_batch.send(b);
        })
    })
    .await?
}

/// Byte dopo il primo LF a `from` o oltre (vedi `lines::next_line_start`); `on_progress` riceve l'offset già cercato.
/// Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn next_line_start(
    vfs: State<'_, Arc<Vfs>>,
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    from: u64,
    unit: u8,
    big_endian: bool,
    on_progress: Channel<u64>,
) -> Result<u64, AppError> {
    let (loc, vfs) = (Loc::parse(&path)?, vfs.inner().clone());
    let guard = requests.register(request_id);
    blocking(move || {
        lines::next_line_start(&mut *source::open_source(&vfs, &loc)?, from, unit, big_endian, guard.flag(), &mut |p| {
            let _ = on_progress.send(p);
        })
    })
    .await?
}

/// Inizio della riga che contiene `at` (vedi `lines::line_start`); `on_progress` riceve l'offset più basso già esaminato.
/// Si annulla con `cancel_request(request_id)` (errore `CANCELLED`).
#[tauri::command]
pub async fn line_start(
    vfs: State<'_, Arc<Vfs>>,
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    at: u64,
    unit: u8,
    big_endian: bool,
    on_progress: Channel<u64>,
) -> Result<u64, AppError> {
    let (loc, vfs) = (Loc::parse(&path)?, vfs.inner().clone());
    let guard = requests.register(request_id);
    blocking(move || {
        lines::line_start(&mut *source::open_source(&vfs, &loc)?, at, unit, big_endian, guard.flag(), &mut |p| {
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
    vfs: State<'_, Arc<Vfs>>,
    requests: State<'_, Requests>,
    request_id: u64,
    path: String,
    encoding: String,
    from: Vec<columns::Mark>,
    target: u64,
    on_progress: Channel<columns::ColumnProgress>,
) -> Result<Vec<columns::ColumnHit>, AppError> {
    let (loc, vfs) = (Loc::parse(&path)?, vfs.inner().clone());
    let guard = requests.register(request_id);
    blocking(move || {
        let mut src = source::open_source(&vfs, &loc)?;
        (0u32..)
            .zip(from)
            .map(|(index, m)| {
                columns::column_offset(&mut *src, &encoding, m, target, guard.flag(), &mut |mark| {
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
    let path = local_only(&path)?;
    let guard = requests.register(request_id);
    blocking(move || hash::hash_file(&path, algo, guard.flag())).await?
}
