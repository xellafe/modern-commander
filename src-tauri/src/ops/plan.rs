//! Espansione di un `JobSpec` nella lista ordinata di elementi da elaborare.
//!
//! Una cartella che qui non si riesce a leggere non fa fallire il job: resta nell'elenco come
//! `unlisted` e il runner la elenca quando ci arriva, dove un errore diventa un prompt
//! (riprova, anche come amministratore, salta, annulla).
//!
//! Le posizioni locali si leggono con `std::fs`, quelle remote con `Provider::stat`/`list`.

use super::model::{FileItem, JobKind, JobSpec, ListEntry};
use crate::error::{AppError, ErrorCode};
use crate::mask::match_mask;
use crate::paths::{normalize_lexical, to_extended};
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Purpose, Stat};
use crate::vfs::router::Vfs;
use std::ffi::{OsStr, OsString};
use std::fs::Metadata;
use std::os::windows::fs::FileTypeExt;
use std::path::Path;

fn invalid(p: Option<&Loc>) -> AppError {
    p.map_or_else(|| AppError::new(ErrorCode::InvalidRequest, None), |l| l.error(ErrorCode::InvalidRequest))
}

fn entry(name: OsString, m: &Metadata) -> ListEntry {
    // `Metadata::is_dir` è falso per giunzioni e symlink: non si seguono mai.
    ListEntry { name, dir: m.is_dir(), dir_link: m.file_type().is_symlink_dir(), size: m.len() }
}

/// `Stat.is_dir` vale anche per un collegamento a cartella: qui, come in locale, `dir` è solo la cartella vera.
fn remote_entry(s: Stat) -> ListEntry {
    ListEntry { name: s.name.into(), dir: s.is_dir && !s.dir_link, dir_link: s.dir_link, size: s.size }
}

fn stat(vfs: &Vfs, l: &Loc) -> Result<ListEntry, AppError> {
    match l {
        Loc::Local(p) => {
            let m = std::fs::symlink_metadata(to_extended(p)).map_err(|e| AppError::from_io(&e, p))?;
            Ok(entry(p.file_name().unwrap_or_default().to_os_string(), &m))
        }
        r => {
            let (prov, path) = vfs.remote(r, Purpose::Transfer)?;
            Ok(remote_entry(prov.stat(&path)?))
        }
    }
}

/// Contenuto diretto di una cartella, senza seguire i collegamenti (`LocalExecutor::list_dir`).
pub fn read_entries(dir: &Path) -> Result<Vec<ListEntry>, AppError> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(to_extended(dir)).map_err(|e| AppError::from_io(&e, dir))? {
        let e = e.map_err(|e| AppError::from_io(&e, dir))?;
        let name = e.file_name();
        // Su Windows `DirEntry::metadata` non segue i collegamenti e non fa chiamate extra.
        let m = e.metadata().map_err(|err| AppError::from_io(&err, &dir.join(&name)))?;
        out.push(entry(name, &m));
    }
    Ok(out)
}

/// Contenuto diretto di una cartella: `read_entries` in locale, `Provider::list` sul remoto.
pub(crate) fn entries(vfs: &Vfs, dir: &Loc) -> Result<Vec<ListEntry>, AppError> {
    if let Loc::Local(p) = dir {
        return read_entries(p);
    }
    let (prov, path) = vfs.remote(dir, Purpose::Transfer)?;
    // Contenuto non fidato: un nome con `/`, `.` o `..` porterebbe fuori dalla cartella.
    let name_ok = |s: &Stat| !matches!(s.name.as_str(), "" | "." | "..") && !s.name.contains('/');
    Ok(prov.list(&path)?.into_iter().filter(name_ok).map(remote_entry).collect())
}

/// Il figlio `name` di `l`; in locale anche un nome non valido in UTF-16 resta intatto.
pub(crate) fn child(l: &Loc, name: &OsStr) -> Loc {
    match l {
        Loc::Local(p) => Loc::Local(p.join(name)),
        r => r.join(&name.to_string_lossy()),
    }
}

/// Forma comune per i confronti e per costruire le destinazioni. In locale `normalize_lexical` (assoluta,
/// senza `\\?\`, `..` risolti: in un percorso `\\?\` Windows non risolve `..`); il remoto è già pulito.
pub(crate) fn norm(l: &Loc) -> Loc {
    match l {
        Loc::Local(p) => Loc::Local(normalize_lexical(p)),
        r => r.clone(),
    }
}

fn name_of(l: &Loc) -> Option<OsString> {
    match l {
        Loc::Local(p) => p.file_name().map(OsStr::to_os_string),
        r => r.file_name().map(OsString::from),
    }
}

/// `unlisted`: cartella vera di cui non si è (ancora) messo il contenuto nell'elenco.
fn item(src: &Loc, dst: Option<Loc>, e: &ListEntry, unlisted: bool) -> FileItem {
    let is_dir = e.dir || e.dir_link;
    FileItem { src: src.clone(), dst, is_dir, size: if is_dir { 0 } else { e.size }, unlisted: unlisted && e.dir }
}

/// Filtro del job; vuoto o di soli spazi = nessuno.
pub fn filter_of(spec: &JobSpec) -> Option<&str> {
    spec.filter.as_deref().filter(|f| !f.trim().is_empty())
}

/// Regola di TC per F5/F6: con una sola sorgente che è un file e una destinazione che non è una
/// cartella esistente né termina con `\`, la destinazione è il nome del file da creare.
pub fn file_target(spec: &JobSpec, vfs: &Vfs) -> bool {
    let is_dir = |l: &Loc| match l {
        Loc::Local(p) => std::fs::metadata(to_extended(p)).is_ok_and(|m| m.is_dir()),
        r => vfs.remote(r, Purpose::Transfer).and_then(|(p, path)| p.stat(&path)).is_ok_and(|s| s.is_dir),
    };
    match (&spec.kind, &spec.dest, spec.sources.as_slice()) {
        (JobKind::Copy | JobKind::Move, Some(dest), [src]) => {
            !dest.to_string().ends_with(['\\', '/']) && !is_dir(dest) && stat(vfs, src).is_ok_and(|e| !e.dir && !e.dir_link)
        }
        _ => false,
    }
}

struct Walker<'a> {
    vfs: &'a Vfs,
    filter: Option<&'a str>,
    /// Delete: contenuto prima della cartella. Copy/Move: cartella prima del contenuto.
    post_order: bool,
    out: Vec<FileItem>,
}

impl Walker<'_> {
    /// `true` se `src` è rimasto nell'elenco (per le cartelle: se non è vuoto dopo il filtro).
    fn visit(&mut self, src: &Loc, dst: Option<Loc>, e: &ListEntry) -> bool {
        // ponytail: un collegamento a cartella è una foglia (is_dir, size 0); Copy/Move lo
        // passano all'executor che decide, Delete elimina il collegamento e mai il contenuto del
        // bersaglio. Seguirlo (con protezione dai cicli) se servirà copiare il contenuto.
        if !e.dir {
            let keep = match self.filter {
                None => true,
                // Il filtro vale per i file; un collegamento a cartella con filtro non si tocca.
                Some(_) if e.dir_link => false,
                Some(mask) => match_mask(&e.name.to_string_lossy(), mask),
            };
            if keep {
                self.out.push(item(src, dst, e, false));
            }
            return keep;
        }
        let Ok(children) = entries(self.vfs, src) else {
            // Illeggibile: si elenca durante l'esecuzione (con il prompt d'errore). Si tiene anche
            // con un filtro: il suo contenuto potrebbe passarlo.
            self.out.push(item(src, dst, e, true));
            return true;
        };
        let at = self.out.len();
        if !self.post_order {
            self.out.push(item(src, dst.clone(), e, false));
        }
        let any = self.children(src, dst.as_ref(), children);
        let keep = self.filter.is_none() || any;
        if self.post_order {
            // Con un filtro le cartelle non si eliminano: conterrebbero i file esclusi.
            if self.filter.is_none() {
                self.out.push(item(src, dst, e, false));
            }
        } else if !keep {
            self.out.truncate(at); // cartella vuota dopo il filtro: non si crea
        }
        keep
    }

    /// Il contenuto `children` della cartella `src`; `true` se ne è rimasto qualcosa.
    fn children(&mut self, src: &Loc, dst: Option<&Loc>, mut children: Vec<ListEntry>) -> bool {
        // Ordine stabile: prima i file, poi le cartelle, per nome.
        children.sort_by_cached_key(|c| (c.dir || c.dir_link, c.name.to_string_lossy().to_lowercase()));
        let mut any = false;
        for c in children {
            any |= self.visit(&child(src, &c.name), dst.map(|d| child(d, &c.name)), &c);
        }
        any
    }
}

/// Elementi del contenuto (`children`, appena elencato) della cartella `dir`, senza la cartella
/// stessa, con le destinazioni costruite dalla `dst` originale di `dir`.
pub fn expand_children(spec: &JobSpec, vfs: &Vfs, dir: &FileItem, children: Vec<ListEntry>) -> Vec<FileItem> {
    let post_order = matches!(spec.kind, JobKind::Delete { .. });
    let mut walker = Walker { vfs, filter: filter_of(spec), post_order, out: Vec::new() };
    walker.children(&dir.src, dir.dst.as_ref(), children);
    walker.out
}

/// Posizione del file `l` nel flusso del suo archivio (solido 7z, TAR.GZ): `(contenitore, ordine)`.
fn stream_key(vfs: &Vfs, l: &Loc) -> Option<(String, u64)> {
    if !matches!(l, Loc::Archive { .. }) {
        return None;
    }
    let (prov, path) = vfs.remote(l, Purpose::Transfer).ok()?;
    Some((l.volume_key(), prov.stream_order(&path)?))
}

/// Se una sorgente è in un archivio a flusso, i file si estraggono nell'ordine del flusso, così lo si decomprime
/// una volta sola: prima le cartelle (nell'ordine attuale, i genitori restano prima dei figli), poi i file per
/// `(contenitore, ordine)`, e in fondo, nell'ordine attuale, quelli senza ordine. L'ordinamento è stabile.
pub(crate) fn stream_sort(items: &mut Vec<FileItem>, vfs: &Vfs) {
    let keys: Vec<_> = items.iter().map(|i| if i.is_dir { None } else { stream_key(vfs, &i.src) }).collect();
    if keys.iter().all(Option::is_none) {
        return;
    }
    let mut keyed: Vec<_> = std::mem::take(items).into_iter().zip(keys).collect();
    // `false < true`: cartelle, poi file con ordine, poi file senza.
    keyed.sort_by(|(a, ka), (b, kb)| (!a.is_dir, ka.is_none(), ka).cmp(&(!b.is_dir, kb.is_none(), kb)));
    items.extend(keyed.into_iter().map(|(i, _)| i));
}

pub fn expand(spec: &JobSpec, vfs: &Vfs) -> Result<Vec<FileItem>, AppError> {
    let first = spec.sources.first().ok_or_else(|| invalid(None))?;
    let filter = filter_of(spec);
    let mut walker = Walker { vfs, filter, post_order: false, out: Vec::new() };
    match &spec.kind {
        JobKind::Mkdir => Ok(vec![FileItem { src: first.clone(), dst: None, is_dir: true, size: 0, unlisted: false }]),
        JobKind::Rename => {
            let dest = spec.dest.clone().ok_or_else(|| invalid(Some(first)))?;
            if spec.sources.len() != 1 {
                return Err(invalid(Some(first)));
            }
            let e = stat(vfs, first)?;
            let (ns, nd) = (norm(first), norm(&dest));
            // Sul remoto si rinomina solo nella stessa connessione.
            if (ns.is_remote() || nd.is_remote()) && ns.volume_key() != nd.volume_key() {
                return Err(invalid(Some(first)));
            }
            // Cambiare solo le maiuscole (`a` → `A`) è lecito; finire dentro se stesso no.
            if nd.is_within(&ns) && !nd.same(&ns) {
                return Err(first.error(ErrorCode::IntoItself));
            }
            Ok(vec![item(first, Some(nd), &e, false)])
        }
        JobKind::Delete { permanent } => {
            walker.post_order = true;
            // Nel Cestino una cartella va intera, come con la Shell: un elemento per sorgente. Il
            // runner la elenca solo se il Cestino la rifiuta e si sceglie l'eliminazione definitiva.
            // Un archivio non ha Cestino: le cartelle si elencano subito.
            let whole = !permanent && filter.is_none();
            for s in &spec.sources {
                // Mai un'intera unità o condivisione.
                if norm(s).file_name().is_none() {
                    return Err(invalid(Some(s)));
                }
                let e = stat(vfs, s)?;
                if whole && !matches!(s, Loc::Archive { .. }) {
                    walker.out.push(item(s, None, &e, true));
                } else {
                    walker.visit(s, None, &e);
                }
            }
            Ok(walker.out)
        }
        // Pack è una Copy verso la radice di uno ZIP (che può non esistere).
        JobKind::Copy | JobKind::Move | JobKind::Pack { .. } => {
            let is_move = spec.kind == JobKind::Move;
            let dest = spec.dest.as_ref().ok_or_else(|| invalid(Some(first)))?;
            if matches!(spec.kind, JobKind::Pack { .. }) && !matches!(dest, Loc::Archive { path, .. } if path == "/") {
                return Err(invalid(Some(dest)));
            }
            // Forma comune (`norm`) per i confronti e per costruire le destinazioni.
            let ndest = norm(dest);
            let to_file = file_target(spec, vfs);
            let mut checked = Vec::with_capacity(spec.sources.len());
            for s in &spec.sources {
                let e = stat(vfs, s)?;
                let ns = norm(s);
                let name = name_of(&ns).ok_or_else(|| invalid(Some(s)))?;
                // Un file come destinazione è il nome nuovo: lo stesso file dà il prompt di conflitto.
                let dst = if to_file { ndest.clone() } else { child(&ndest, &name) };
                if !to_file && ndest.is_within(&ns) {
                    return Err(s.error(ErrorCode::IntoItself));
                }
                if is_move && !to_file && ns.parent().is_some_and(|p| p.same(&ndest)) {
                    return Err(s.error(ErrorCode::SameFile));
                }
                // ponytail: stessa unità = stessa lettera/condivisione; un volume montato in una
                // cartella o un'unità `subst` danno ERROR_NOT_SAME_DEVICE e l'executor ripiega su copia.
                // Sul remoto la chiave è la connessione: lo spostamento nella stessa è un `Provider::rename`; in un
                // archivio è il contenitore, e lo spostamento nello stesso è una rinomina nella sua riscrittura.
                let vol = ns.volume_key();
                let rename = is_move && filter.is_none() && !vol.is_empty() && vol == ndest.volume_key();
                checked.push((s, e, dst, rename));
            }
            for (s, e, dst, rename) in checked {
                if rename {
                    walker.out.push(item(s, Some(dst), &e, true)); // rename: nessuna espansione
                } else {
                    walker.visit(s, Some(dst), &e);
                }
            }
            stream_sort(&mut walker.out, vfs);
            Ok(walker.out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::ops::model::{ConflictPolicy, JobKind};
    use crate::paths::to_extended;
    use crate::vfs::mem;
    use std::path::{Path, PathBuf};

    // I test locali passano dal `Vfs` senza connessioni.
    fn expand(spec: &JobSpec) -> Result<Vec<FileItem>, AppError> {
        super::expand(spec, &mem::vfs(&[]))
    }

    fn file_target(spec: &JobSpec) -> bool {
        super::file_target(spec, &mem::vfs(&[]))
    }

    fn spec(kind: JobKind, sources: Vec<PathBuf>, dest: Option<PathBuf>, filter: Option<&str>) -> JobSpec {
        let sources = sources.into_iter().map(Loc::Local).collect();
        JobSpec { kind, sources, dest: dest.map(Loc::Local), filter: filter.map(str::to_string), policy: ConflictPolicy::Ask }
    }

    fn srcs(items: &[FileItem]) -> Vec<PathBuf> {
        items.iter().map(|i| i.src.as_local().unwrap().to_path_buf()).collect()
    }

    fn local(p: impl Into<PathBuf>) -> Option<Loc> {
        Some(Loc::Local(p.into()))
    }

    /// a/{x.txt (3 byte), s/y.txt}
    fn tree(root: &Path) -> PathBuf {
        let a = root.join("a");
        std::fs::create_dir_all(a.join("s")).unwrap();
        std::fs::write(a.join("x.txt"), b"abc").unwrap();
        std::fs::write(a.join("s").join("y.txt"), b"").unwrap();
        a
    }

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok);
    }

    #[test]
    fn copy_into_itself_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("b")).unwrap();
        let s = spec(JobKind::Copy, vec![a.clone()], Some(a.join("b")), None);
        assert_eq!(expand(&s).unwrap_err().code, ErrorCode::IntoItself);
    }

    #[test]
    fn into_itself_detected_across_path_forms() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("b")).unwrap();
        for dest in [to_extended(&a.join("b")), a.clone(), a.join("b").join("..").join("B"), d.path().join("A\\")] {
            let s = spec(JobKind::Move, vec![a.clone()], Some(dest.clone()), None);
            assert_eq!(expand(&s).unwrap_err().code, ErrorCode::IntoItself, "{dest:?}");
        }
        let s = spec(JobKind::Copy, vec![to_extended(&a)], Some(a.join("b")), None);
        assert_eq!(expand(&s).unwrap_err().code, ErrorCode::IntoItself);
    }

    #[test]
    fn copy_into_own_parent_is_allowed() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().to_path_buf()), None)).unwrap();
        assert_eq!(items[0].dst, local(&a));
    }

    #[test]
    fn destinations_are_built_from_normalized_dest() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = to_extended(d.path()).join("x").join("..").join("D");
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(dest), None)).unwrap();
        let da = d.path().join("D").join("a");
        assert_eq!(items[0].dst, local(&da));
        assert_eq!(items[3].dst, local(da.join("s").join("y.txt")));
        let renamed = d.path().join("x").join("..").join("b");
        let items = expand(&spec(JobKind::Rename, vec![a], Some(renamed), None)).unwrap();
        assert_eq!(items[0].dst, local(d.path().join("b")));
    }

    #[test]
    fn move_into_same_parent_is_same_file() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = to_extended(d.path()).join(".");
        assert_eq!(expand(&spec(JobKind::Move, vec![a], Some(dest), None)).unwrap_err().code, ErrorCode::SameFile);
    }

    #[test]
    fn copy_expands_dirs_before_children_and_maps_dest() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = d.path().join("D");
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt"), a.join("s"), a.join("s").join("y.txt")]);
        let dsts: Vec<_> = items.iter().map(|i| i.dst.as_ref().unwrap().as_local().unwrap().to_path_buf()).collect();
        let da = dest.join("a");
        assert_eq!(dsts, vec![da.clone(), da.join("x.txt"), da.join("s"), da.join("s").join("y.txt")]);
        assert_eq!(items.iter().map(|i| (i.is_dir, i.size)).collect::<Vec<_>>(), vec![(true, 0), (false, 3), (true, 0), (false, 0)]);
    }

    #[test]
    fn copy_keeps_empty_dirs_without_filter_and_accepts_files() {
        let d = tempfile::tempdir().unwrap();
        let e = d.path().join("empty");
        std::fs::create_dir(&e).unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"12").unwrap();
        let dest = d.path().join("D");
        let items = expand(&spec(JobKind::Copy, vec![e.clone(), f.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(
            items,
            vec![
                FileItem { src: Loc::Local(e), dst: local(dest.join("empty")), is_dir: true, size: 0, unlisted: false },
                FileItem { src: Loc::Local(f), dst: local(dest.join("f.txt")), is_dir: false, size: 2, unlisted: false },
            ]
        );
    }

    #[test]
    fn delete_expands_children_before_dirs() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let items = expand(&spec(JobKind::Delete { permanent: true }, vec![a.clone()], None, None)).unwrap();
        assert_eq!(srcs(&items), vec![a.join("x.txt"), a.join("s").join("y.txt"), a.join("s"), a.clone()]);
        assert!(items.iter().all(|i| i.dst.is_none()));
    }

    #[test]
    fn recycle_delete_is_one_item_per_source() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"12").unwrap();
        // La cartella va nel Cestino intera, come con la Shell: si espande solo se il Cestino la rifiuta.
        let items = expand(&spec(JobKind::Delete { permanent: false }, vec![a.clone(), f.clone()], None, None)).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), f.clone()]);
        assert_eq!((items[0].is_dir, items[0].unlisted, items[1].size), (true, true, 2));
        // Con un filtro si cestinano solo i file che passano: si espande.
        let items = expand(&spec(JobKind::Delete { permanent: false }, vec![a.clone()], None, Some("*.txt"))).unwrap();
        assert_eq!(srcs(&items), vec![a.join("x.txt"), a.join("s").join("y.txt")]);
    }

    #[test]
    fn unreadable_subfolder_is_left_to_list_at_run_time() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let _deny = crate::ops::DenyList::new(&a.join("s"));
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().join("D")), None)).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt"), a.join("s")]);
        assert_eq!(items.iter().map(|i| i.unlisted).collect::<Vec<_>>(), vec![false, false, true]);
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![a.clone()], None, None)).unwrap();
        assert_eq!(srcs(&del), vec![a.join("x.txt"), a.join("s"), a.clone()]);
        // Anche con un filtro: il contenuto della cartella illeggibile potrebbe passarlo.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![a.clone()], None, Some("*.md"))).unwrap();
        assert_eq!(srcs(&del), vec![a.join("s")]);
    }

    #[test]
    fn single_file_to_a_missing_name_is_the_target_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let to = d.path().join("D").join("g.txt");
        for kind in [JobKind::Copy, JobKind::Move] {
            let items = expand(&spec(kind, vec![f.clone()], Some(to.clone()), None)).unwrap();
            assert_eq!(items[0].dst, local(&to));
        }
        // Un file esistente come destinazione: lo stesso nome (poi il prompt di conflitto).
        let items = expand(&spec(JobKind::Copy, vec![f.clone()], Some(f.clone()), None)).unwrap();
        assert_eq!(items[0].dst, local(&f));
        // Con `\` finale, una cartella esistente, una cartella o più sorgenti: la destinazione è una cartella.
        let mut slash = to.clone().into_os_string();
        slash.push("\\");
        let into = |sources: Vec<PathBuf>, dest: PathBuf| expand(&spec(JobKind::Copy, sources, Some(dest), None)).unwrap()[0].dst.clone();
        assert_eq!(into(vec![f.clone()], PathBuf::from(slash)), local(to.join("f.txt")));
        assert_eq!(into(vec![f.clone()], d.path().to_path_buf()), local(d.path().join("f.txt")));
        let a = tree(d.path());
        assert_eq!(into(vec![a.clone()], to.clone()), local(to.join("a")));
        assert_eq!(into(vec![f.clone(), a.join("x.txt")], to.clone()), local(to.join("f.txt")));
        assert_eq!(file_target(&spec(JobKind::Copy, vec![f.clone()], Some(to.clone()), None)), true);
        assert_eq!(file_target(&spec(JobKind::Delete { permanent: true }, vec![f.clone()], Some(to), None)), false);
    }

    #[test]
    fn filter_applies_to_files_only() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("s")).unwrap();
        for f in ["x.txt", "y.md", "s/z.md"] {
            std::fs::write(a.join(f), b"").unwrap();
        }
        let items = expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().join("D")), Some("*.txt"))).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt")]);
        // Nessun file che passa il filtro: niente da creare.
        assert!(expand(&spec(JobKind::Copy, vec![a.clone()], Some(d.path().join("D")), Some("*.zip"))).unwrap().is_empty());
        // Con filtro, Delete non elimina cartelle: conterrebbero file esclusi.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![a.clone()], None, Some("*.md"))).unwrap();
        assert_eq!(srcs(&del), vec![a.join("y.md"), a.join("s").join("z.md")]);
    }

    #[test]
    fn move_on_same_volume_is_one_item_per_source() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let dest = d.path().join("D");
        let items = expand(&spec(JobKind::Move, vec![a.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(items, vec![FileItem { src: Loc::Local(a.clone()), dst: local(dest.join("a")), is_dir: true, size: 0, unlisted: true }]);
        // Con un filtro il rename in blocco sposterebbe anche i file esclusi: si espande.
        let items = expand(&spec(JobKind::Move, vec![a.clone()], Some(dest), Some("x.*"))).unwrap();
        assert_eq!(srcs(&items), vec![a.clone(), a.join("x.txt")]);
    }

    #[test]
    fn move_across_volumes_is_expanded() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let other = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
        if crate::paths::volume_key(d.path()) == crate::paths::volume_key(other.path()) {
            return; // temp e sorgenti sulla stessa unità: caso non riproducibile qui.
        }
        let items = expand(&spec(JobKind::Move, vec![a.clone()], Some(other.path().to_path_buf()), None)).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[3].dst, local(other.path().join("a").join("s").join("y.txt")));
    }

    #[test]
    fn missing_source_is_not_found_with_path() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone");
        for kind in [JobKind::Copy, JobKind::Move, JobKind::Delete { permanent: false }, JobKind::Rename] {
            let e = expand(&spec(kind, vec![gone.clone()], Some(d.path().join("D")), None)).unwrap_err();
            assert_eq!(e.code, ErrorCode::NotFound);
            assert_eq!(e.path.as_deref(), Some(gone.to_str().unwrap()));
        }
    }

    #[test]
    fn invalid_requests() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let bad = [
            spec(JobKind::Copy, vec![a.clone()], None, None),
            spec(JobKind::Copy, vec![], Some(d.path().join("D")), None),
            spec(JobKind::Rename, vec![a.clone(), a.clone()], Some(d.path().join("b")), None),
            spec(JobKind::Mkdir, vec![], None, None),
            spec(JobKind::Copy, vec![PathBuf::from(r"C:\")], Some(d.path().join("D")), None),
            spec(JobKind::Delete { permanent: true }, vec![PathBuf::from(r"C:\")], None, None),
            spec(JobKind::Delete { permanent: false }, vec![PathBuf::from(r"\\?\C:\")], None, None),
            spec(JobKind::Delete { permanent: true }, vec![PathBuf::from(r"\\srv\sh\")], None, None),
        ];
        for s in bad {
            assert_eq!(expand(&s).unwrap_err().code, ErrorCode::InvalidRequest, "{s:?}");
        }
    }

    #[test]
    fn rename_and_mkdir_are_single_items() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let b = d.path().join("b");
        let items = expand(&spec(JobKind::Rename, vec![a.clone()], Some(b.clone()), None)).unwrap();
        assert_eq!(items, vec![FileItem { src: Loc::Local(a.clone()), dst: local(b), is_dir: true, size: 0, unlisted: false }]);
        let upper = d.path().join("A");
        assert_eq!(expand(&spec(JobKind::Rename, vec![a.clone()], Some(upper.clone()), None)).unwrap()[0].dst, local(upper));
        let inside = spec(JobKind::Rename, vec![a.clone()], Some(a.join("s").join("a")), None);
        assert_eq!(expand(&inside).unwrap_err().code, ErrorCode::IntoItself);
        let n = d.path().join("new");
        let items = expand(&spec(JobKind::Mkdir, vec![n.clone()], None, None)).unwrap();
        assert_eq!(items, vec![FileItem { src: Loc::Local(n), dst: None, is_dir: true, size: 0, unlisted: false }]);
    }

    #[test]
    fn junctions_are_leaves() {
        let d = tempfile::tempdir().unwrap();
        let target = tree(d.path());
        let p = d.path().join("p");
        std::fs::create_dir(&p).unwrap();
        let link = p.join("link");
        junction(&link, &target);

        // Eliminare la cartella che contiene la giunzione non tocca il contenuto del bersaglio.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![p.clone()], None, None)).unwrap();
        assert_eq!(srcs(&del), vec![link.clone(), p.clone()]);
        // Eliminare la giunzione stessa: un solo elemento, il collegamento.
        let del = expand(&spec(JobKind::Delete { permanent: true }, vec![link.clone()], None, None)).unwrap();
        assert_eq!(del, vec![FileItem { src: Loc::Local(link.clone()), dst: None, is_dir: true, size: 0, unlisted: false }]);
        // Nel Cestino il collegamento va da solo, e non è mai una cartella da elencare.
        let del = expand(&spec(JobKind::Delete { permanent: false }, vec![link.clone()], None, None)).unwrap();
        assert!(!del[0].unlisted);

        let dest = d.path().join("D");
        let cp = expand(&spec(JobKind::Copy, vec![p.clone()], Some(dest.clone()), None)).unwrap();
        assert_eq!(cp[1], FileItem { src: Loc::Local(link), dst: local(dest.join("p").join("link")), is_dir: true, size: 0, unlisted: false });
        assert_eq!(cp.len(), 2);
    }

    #[test]
    fn stream_sort_orders_files_by_archive() {
        use std::io::Write;
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in [("b/2", &b"22"[..]), ("a/1", &b"1"[..])] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            b.append_data(&mut h, name, data).unwrap();
        }
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&b.into_inner().unwrap()).unwrap();
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("t.tar.gz");
        std::fs::write(&g, enc.finish().unwrap()).unwrap();
        let at = |p: &str| Loc::archive(Loc::Local(g.clone()), p);
        let it = |p: &str, is_dir: bool| FileItem { src: at(p), dst: None, is_dir, size: 0, unlisted: false };
        let loose = FileItem { src: Loc::Local(d.path().join("z.txt")), dst: None, is_dir: false, size: 0, unlisted: false };
        // Prima a, poi b: l'ordine d'archivio è `b/2`, `a/1`. I file fuori dall'archivio restano dopo.
        let mut items = vec![loose.clone(), it("/b", true), it("/a", true), it("/a/1", false), it("/b/2", false)];
        stream_sort(&mut items, &mem::vfs(&[]));
        let got: Vec<_> = items.iter().map(|i| i.src.to_string()).collect();
        let want: Vec<_> = [at("/b"), at("/a"), at("/b/2"), at("/a/1"), loose.src.clone()].iter().map(Loc::to_string).collect();
        assert_eq!(got, want);
        // Senza voci a flusso l'elenco non cambia.
        let mut plain = vec![loose.clone(), it("/b", true)];
        stream_sort(&mut plain, &mem::vfs(&[]));
        assert_eq!(plain[0], loose);
    }

    /// ZIP con `conf/a.xml` e `conf/sub/b.xml`; ritorna il contenitore.
    fn zip_with_conf(dir: &Path) -> Loc {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for name in ["conf/a.xml", "conf/sub/b.xml"] {
            w.start_file(name, o).unwrap();
            w.write_all(b"x").unwrap();
        }
        let z = dir.join("a.zip");
        std::fs::write(&z, w.finish().unwrap().into_inner()).unwrap();
        Loc::Local(z)
    }

    fn arc_spec(kind: JobKind, src: Loc, dest: Option<Loc>) -> JobSpec {
        JobSpec { kind, sources: vec![src], dest, filter: None, policy: ConflictPolicy::Ask }
    }

    fn names_of(items: &[FileItem]) -> Vec<String> {
        items.iter().map(|i| i.src.file_name().unwrap()).collect()
    }

    #[test]
    fn move_of_an_archive_dir_is_renamed_only_inside_its_archive() {
        let d = tempfile::tempdir().unwrap();
        let z = zip_with_conf(d.path());
        let conf = Loc::archive(z.clone(), "/conf");
        let out = Loc::Local(d.path().join("out"));
        let items = expand(&arc_spec(JobKind::Move, conf.clone(), Some(out))).unwrap();
        assert_eq!(names_of(&items), ["conf", "a.xml", "sub", "b.xml"]);
        assert!(items.iter().all(|i| !i.unlisted), "{items:?}");
        // Dentro lo stesso archivio (anche scritto in un'altra forma) la cartella si rinomina in blocco.
        let upper = Loc::Local(PathBuf::from(z.to_string().to_uppercase()));
        let items = expand(&arc_spec(JobKind::Move, conf.clone(), Some(Loc::archive(upper, "/altro")))).unwrap();
        assert_eq!((names_of(&items), items[0].unlisted), (vec!["conf".to_owned()], true));
    }

    #[test]
    fn pack_expands_like_copy_into_an_archive_root() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let root = Loc::archive(Loc::Local(d.path().join("n.zip")), "/");
        let pack = |dest: Loc| JobSpec { dest: Some(dest), ..spec(JobKind::Pack { replace: true }, vec![a.clone()], None, None) };
        let items = expand(&pack(root.clone())).unwrap();
        assert_eq!(names_of(&items), ["a", "x.txt", "s", "y.txt"]);
        assert_eq!(items[1].dst, Some(root.join("a").join("x.txt")));
        assert!(!file_target(&pack(root.clone())));
        // Solo verso la radice di un archivio.
        for bad in [root.join("a"), Loc::Local(d.path().join("D"))] {
            assert_eq!(expand(&pack(bad.clone())).unwrap_err().code, ErrorCode::InvalidRequest, "{bad}");
        }
    }

    #[test]
    fn recycle_delete_of_an_archive_dir_is_listed() {
        let d = tempfile::tempdir().unwrap();
        let conf = Loc::archive(zip_with_conf(d.path()), "/conf");
        // Un archivio non ha Cestino: niente elemento unico da elencare dopo, il contenuto è già nell'elenco.
        let items = expand(&arc_spec(JobKind::Delete { permanent: false }, conf, None)).unwrap();
        assert_eq!(names_of(&items), ["a.xml", "b.xml", "sub", "conf"]);
        assert!(items.iter().all(|i| !i.unlisted), "{items:?}");
    }
}
