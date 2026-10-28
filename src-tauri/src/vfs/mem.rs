//! Provider in memoria per i test, con guasti iniettabili: lettura che si interrompe, server senza
//! rinomina atomica, cartelle non scrivibili. I percorsi sono POSIX già puliti (come quelli di `Loc`).

use super::loc::Scheme;
use super::provider::{Caps, Connector, Provider, Purpose, Stat};
use super::router::Vfs;
use crate::error::{AppError, ErrorCode};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Node {
    dir: bool,
    /// Collegamento a cartella (`Stat.dir_link`): si elenca come cartella, senza contenuto.
    link: bool,
    data: Vec<u8>,
    modified_ms: Option<i64>,
    mode: Option<u32>,
}

type Nodes = Arc<Mutex<BTreeMap<String, Node>>>;

pub struct MemProvider {
    nodes: Nodes,
    /// Byte dopo cui `reader` dà `CONNECTION_FAILED` (se il file è più lungo).
    pub fail_read_after: Option<u64>,
    /// `false`: `rename(replace = true)` su un file esistente fallisce senza toccarlo.
    pub atomic_replace: bool,
    /// Se c'è, solo queste cartelle accettano nomi nuovi, rinomine ed eliminazioni; un file esistente si riscrive comunque.
    pub writable_dirs: Option<HashSet<String>>,
    /// `false`: date al minuto, come un server FTP senza `MDTM`.
    pub precise_mtime: bool,
    /// `true`: `list` e `delete` danno `CONNECTION_FAILED`, come una sessione caduta.
    pub offline: AtomicBool,
    /// `true`: `list` e `delete` danno `TIMEOUT`, come una sessione bloccata (VPN caduta).
    pub stalled: AtomicBool,
    /// Byte restituiti da `read_at` e dagli stream di `reader`: quanto si è letto "dalla rete".
    pub bytes_read: Arc<AtomicU64>,
    /// `true`: dopo la prima eliminazione ogni `rename` fallisce (`IO`), come una connessione che cade lì.
    pub fail_rename_after_delete: bool,
    deleted: AtomicBool,
}

/// Le connessioni per id.
pub struct MemConnector(pub HashMap<String, Arc<MemProvider>>);

impl Connector for MemConnector {
    fn provider(&self, _: Scheme, conn: &str, _: Purpose) -> Result<Arc<dyn Provider>, AppError> {
        let p = self.0.get(conn).ok_or_else(|| AppError::new(ErrorCode::NotFound, None))?;
        Ok(p.clone())
    }
}

/// `Vfs` con le connessioni date per id (nessuna: ogni posizione remota dà `NOT_FOUND`).
pub fn vfs(conns: &[(&str, &Arc<MemProvider>)]) -> Vfs {
    Vfs::new(Arc::new(MemConnector(conns.iter().map(|(id, p)| (id.to_string(), Arc::clone(p))).collect())))
}

fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

fn err(code: ErrorCode, path: &str) -> AppError {
    AppError::new(code, Some(Path::new(path)))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// Lettura che si interrompe come una connessione caduta.
struct Broken;

impl Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other(AppError::new(ErrorCode::ConnectionFailed, None)))
    }
}

/// Conta i byte letti in `bytes_read`.
struct Counted<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Scrive direttamente nel nodo: un errore a metà lascia il file incompleto, come su un server vero.
struct MemWriter {
    nodes: Nodes,
    path: String,
}

impl Write for MemWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut nodes = self.nodes.lock().unwrap();
        let node = nodes.get_mut(&self.path).ok_or_else(|| std::io::Error::other(err(ErrorCode::NotFound, &self.path)))?;
        node.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl MemProvider {
    fn online(&self, path: &str) -> Result<(), AppError> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(err(ErrorCode::ConnectionFailed, path));
        }
        match self.stalled.load(Ordering::SeqCst) {
            true => Err(err(ErrorCode::Timeout, path)),
            false => Ok(()),
        }
    }

    pub fn new() -> Self {
        let nodes = BTreeMap::from([("/".to_owned(), Node { dir: true, ..Node::default() })]);
        Self { nodes: Arc::new(Mutex::new(nodes)), fail_read_after: None, atomic_replace: true, writable_dirs: None, precise_mtime: true, offline: AtomicBool::new(false), stalled: AtomicBool::new(false), bytes_read: Arc::default(), fail_rename_after_delete: false, deleted: AtomicBool::new(false) }
    }

    /// Crea la cartella e quelle sopra che mancano.
    pub fn add_dir(&self, path: &str) {
        let mut nodes = self.nodes.lock().unwrap();
        let mut p = path;
        while p != "/" {
            nodes.entry(p.to_owned()).or_insert(Node { dir: true, ..Node::default() });
            p = parent(p);
        }
    }

    /// Crea (o sostituisce) il file, con le cartelle sopra che mancano.
    pub fn add_file(&self, path: &str, data: &[u8]) {
        self.add_dir(parent(path));
        let node = Node { data: data.to_vec(), modified_ms: Some(now_ms()), ..Node::default() };
        self.nodes.lock().unwrap().insert(path.to_owned(), node);
    }

    /// Crea un collegamento a cartella (come `current -> releases/...`).
    pub fn add_dir_link(&self, path: &str) {
        self.add_dir(parent(path));
        self.nodes.lock().unwrap().insert(path.to_owned(), Node { dir: true, link: true, ..Node::default() });
    }

    /// Contenuto di un file, per le verifiche dei test.
    pub fn data(&self, path: &str) -> Option<Vec<u8>> {
        self.nodes.lock().unwrap().get(path).filter(|n| !n.dir).map(|n| n.data.clone())
    }

    fn writable(&self, dir: &str) -> Result<(), AppError> {
        match &self.writable_dirs {
            Some(dirs) if !dirs.contains(dir) => Err(err(ErrorCode::AccessDenied, dir)),
            _ => Ok(()),
        }
    }

    fn file(&self, path: &str) -> Result<Vec<u8>, AppError> {
        match self.nodes.lock().unwrap().get(path) {
            Some(n) if !n.dir => Ok(n.data.clone()),
            Some(_) => Err(err(ErrorCode::InvalidRequest, path)),
            None => Err(err(ErrorCode::NotFound, path)),
        }
    }
}

fn dir_exists(nodes: &BTreeMap<String, Node>, path: &str) -> Result<(), AppError> {
    match nodes.get(path) {
        Some(n) if n.dir => Ok(()),
        _ => Err(err(ErrorCode::NotFound, path)),
    }
}

impl Provider for MemProvider {
    fn caps(&self) -> Caps {
        Caps { writable: true, atomic_replace: self.atomic_replace, precise_mtime: self.precise_mtime, ranged_read: true }
    }

    fn stat(&self, path: &str) -> Result<Stat, AppError> {
        let nodes = self.nodes.lock().unwrap();
        let n = nodes.get(path).ok_or_else(|| err(ErrorCode::NotFound, path))?;
        let name = path.rsplit('/').next().unwrap_or_default().to_owned();
        Ok(Stat { name, is_dir: n.dir, dir_link: n.link, size: n.data.len() as u64, modified_ms: n.modified_ms, mode: n.mode })
    }

    fn list(&self, path: &str) -> Result<Vec<Stat>, AppError> {
        self.online(path)?;
        let children: Vec<String> = {
            let nodes = self.nodes.lock().unwrap();
            dir_exists(&nodes, path)?;
            nodes.keys().filter(|k| k.as_str() != "/" && parent(k) == path).cloned().collect()
        };
        children.iter().map(|k| self.stat(k)).collect()
    }

    fn read_at(&self, path: &str, offset: u64, len: u32) -> Result<Vec<u8>, AppError> {
        let data = self.file(path)?;
        let start = (offset.min(data.len() as u64)) as usize;
        let end = (start + len as usize).min(data.len());
        self.bytes_read.fetch_add((end - start) as u64, Ordering::Relaxed);
        Ok(data[start..end].to_vec())
    }

    fn reader(&self, path: &str) -> Result<Box<dyn Read + Send>, AppError> {
        let data = self.file(path)?;
        let inner: Box<dyn Read + Send> = match self.fail_read_after {
            Some(n) if n < data.len() as u64 => Box::new(Cursor::new(data[..n as usize].to_vec()).chain(Broken)),
            _ => Box::new(Cursor::new(data)),
        };
        Ok(Box::new(Counted { inner, count: self.bytes_read.clone() }))
    }

    fn writer(&self, path: &str, create_new: bool) -> Result<Box<dyn Write + Send>, AppError> {
        let mut nodes = self.nodes.lock().unwrap();
        dir_exists(&nodes, parent(path))?;
        match nodes.get_mut(path) {
            Some(_) if create_new => return Err(err(ErrorCode::AlreadyExists, path)),
            Some(n) if n.dir => return Err(err(ErrorCode::InvalidRequest, path)),
            Some(n) => {
                n.data.clear();
                n.modified_ms = Some(now_ms());
            }
            None => {
                self.writable(parent(path))?;
                nodes.insert(path.to_owned(), Node { modified_ms: Some(now_ms()), ..Node::default() });
            }
        }
        Ok(Box::new(MemWriter { nodes: self.nodes.clone(), path: path.to_owned() }))
    }

    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), AppError> {
        if self.fail_rename_after_delete && self.deleted.load(Ordering::SeqCst) {
            return Err(err(ErrorCode::Io, to));
        }
        let mut nodes = self.nodes.lock().unwrap();
        if !nodes.contains_key(from) {
            return Err(err(ErrorCode::NotFound, from));
        }
        dir_exists(&nodes, parent(to))?;
        self.writable(parent(from))?;
        self.writable(parent(to))?;
        if from == to {
            return Ok(());
        }
        if let Some(existing) = nodes.get(to) {
            if !replace || existing.dir {
                return Err(err(ErrorCode::AlreadyExists, to));
            }
            if !self.atomic_replace {
                // Come un server SFTP senza `posix-rename`: `SSH_FX_FAILURE`.
                return Err(err(ErrorCode::Io, to));
            }
        }
        let prefix = format!("{from}/");
        let moved: Vec<String> = nodes.keys().filter(|k| k.as_str() == from || k.starts_with(&prefix)).cloned().collect();
        for k in moved {
            let n = nodes.remove(&k).unwrap();
            nodes.insert(format!("{to}{}", &k[from.len()..]), n);
        }
        Ok(())
    }

    fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        self.online(path)?;
        let mut nodes = self.nodes.lock().unwrap();
        let n = nodes.get(path).ok_or_else(|| err(ErrorCode::NotFound, path))?;
        if n.dir != is_dir || path == "/" {
            return Err(err(ErrorCode::InvalidRequest, path));
        }
        self.writable(parent(path))?;
        if is_dir && nodes.keys().any(|k| k.as_str() != "/" && parent(k) == path) {
            return Err(err(ErrorCode::Io, path)); // cartella non vuota
        }
        nodes.remove(path);
        self.deleted.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn mkdir(&self, path: &str) -> Result<(), AppError> {
        let mut nodes = self.nodes.lock().unwrap();
        if nodes.contains_key(path) {
            return Err(err(ErrorCode::AlreadyExists, path));
        }
        dir_exists(&nodes, parent(path))?;
        self.writable(parent(path))?;
        nodes.insert(path.to_owned(), Node { dir: true, modified_ms: Some(now_ms()), ..Node::default() });
        Ok(())
    }

    fn set_meta(&self, path: &str, modified_ms: Option<i64>, mode: Option<u32>) -> Result<(), AppError> {
        let mut nodes = self.nodes.lock().unwrap();
        let n = nodes.get_mut(path).ok_or_else(|| err(ErrorCode::NotFound, path))?;
        n.modified_ms = modified_ms.or(n.modified_ms);
        n.mode = mode.or(n.mode);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::provider::stream_error;

    #[test]
    fn mem_provider_faults() {
        let mut m = MemProvider::new();
        m.add_file("/d/a.txt", b"0123456789");
        m.fail_read_after = Some(4);
        let mut buf = vec![];
        let e = m.reader("/d/a.txt").unwrap().read_to_end(&mut buf).unwrap_err();
        assert_eq!((stream_error(e, "/d/a.txt").code, buf.as_slice()), (ErrorCode::ConnectionFailed, &b"0123"[..]));

        m.atomic_replace = false;
        m.writer("/d/b", true).unwrap().write_all(b"nuovo").unwrap();
        assert_eq!(m.writer("/d/b", true).err().unwrap().code, ErrorCode::AlreadyExists);
        assert_eq!(m.rename("/d/b", "/d/a.txt", false).unwrap_err().code, ErrorCode::AlreadyExists);
        assert_eq!(m.rename("/d/b", "/d/a.txt", true).unwrap_err().code, ErrorCode::Io);
        assert_eq!(m.data("/d/a.txt").unwrap(), b"0123456789");
        assert_eq!(m.data("/d/b").unwrap(), b"nuovo");
        m.atomic_replace = true;
        m.rename("/d/b", "/d/a.txt", true).unwrap();
        assert_eq!((m.data("/d/a.txt").unwrap(), m.data("/d/b")), (b"nuovo".to_vec(), None));

        m.writable_dirs = Some(HashSet::from(["/w".to_owned()]));
        assert_eq!(m.writer("/d/c", true).err().unwrap().code, ErrorCode::AccessDenied);
        m.writer("/d/a.txt", false).unwrap().write_all(b"diretto").unwrap();
        assert_eq!(m.data("/d/a.txt").unwrap(), b"diretto");

        m.writable_dirs = None;
        m.mkdir("/d/e").unwrap();
        m.set_meta("/d/a.txt", Some(5), Some(0o600)).unwrap();
        let names: Vec<_> = m.list("/d").unwrap().into_iter().map(|s| (s.name, s.is_dir, s.modified_ms)).collect();
        assert_eq!(names[0], ("a.txt".into(), false, Some(5)));
        assert_eq!(names[1].0, "e");
        assert_eq!(m.delete("/d", true).unwrap_err().code, ErrorCode::Io);
        m.rename("/d", "/x", false).unwrap();
        assert_eq!(m.read_at("/x/a.txt", 2, 100).unwrap(), b"retto");
    }
}
