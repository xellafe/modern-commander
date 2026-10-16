//! Esecuzione di un job, un elemento alla volta, nel thread worker. I conflitti si risolvono con
//! la politica del job o chiedendo all'utente; ogni errore diventa un `Prompt::Error`.
//! Le destinazioni si creano sempre senza sostituire nulla: si sovrascrive solo con
//! `copy_file`/`move_path(overwrite = true)` dopo una decisione `Overwrite`.

use super::conflict::{next_free_name, resolve, same_file};
use super::exec::Executor;
use super::model::{ConflictPolicy, Decision, FileItem, JobKind, JobSpec, Meta};
use super::plan::expand;
use super::queue::{ErrorAction, Handle, JobState, Prompt, PromptReply};
use crate::error::{AppError, ErrorCode};
use crate::fs::listing::FILETIME_UNIX_DIFF_MS;
use crate::paths::{normalize_lexical, to_extended};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Il job è stato annullato (dall'utente o con una risposta `Cancel`).
struct Stop;

enum Fail {
    Stop,
    Err(AppError),
}

impl From<AppError> for Fail {
    fn from(e: AppError) -> Self {
        Fail::Err(e)
    }
}

/// Stato di un elemento tra un tentativo e l'altro.
#[derive(Default)]
struct Attempt {
    /// Eliminare in modo definitivo dopo un rifiuto del Cestino.
    permanent: bool,
    /// Prima fase già riuscita (copia del file, creazione della cartella) con la destinazione
    /// effettiva: un nuovo tentativo riprende dalla fase successiva.
    created: Option<PathBuf>,
}

pub(crate) fn run(h: &Handle, exec: &dyn Executor, spec: &JobSpec) -> JobState {
    let items = match expand(spec) {
        Ok(items) => items,
        Err(e) => {
            h.update(|v| v.errors.push(e));
            return JobState::Failed;
        }
    };
    h.update(|v| {
        v.files_total = items.len() as u64;
        v.bytes_total = items.iter().map(|i| i.size).sum();
    });
    let mut r = Runner {
        h,
        exec,
        spec,
        items,
        bytes: 0,
        conflict_all: None,
        skip_all: Vec::new(),
        permanent_all: false,
        skipped: Vec::new(),
        renamed: Vec::new(),
        src_dirs: Vec::new(),
    };
    match r.run() {
        Ok(()) => JobState::Done,
        Err(Stop) => JobState::Cancelled,
    }
}

struct Runner<'a> {
    h: &'a Handle,
    exec: &'a dyn Executor,
    spec: &'a JobSpec,
    /// Le `dst` restano quelle originali: le rinomine si applicano con `rewrite`.
    items: Vec<FileItem>,
    /// Byte degli elementi già elaborati.
    bytes: u64,
    conflict_all: Option<Decision>,
    /// Codici di errore da saltare senza chiedere.
    skip_all: Vec<ErrorCode>,
    permanent_all: bool,
    // ponytail: ricerca lineare, O(elementi × saltati); un albero di prefissi se i salti diventano migliaia.
    /// Sorgenti saltate: si saltano anche i discendenti (Copy/Move) e gli antenati (Delete).
    skipped: Vec<PathBuf>,
    /// Cartelle di destinazione rinominate (vecchia, nuova), nell'ordine in cui sono avvenute.
    renamed: Vec<(PathBuf, PathBuf)>,
    /// Move: cartelle sorgente da rimuovere alla fine, se vuote.
    src_dirs: Vec<PathBuf>,
}

impl Runner<'_> {
    fn run(&mut self) -> Result<(), Stop> {
        let mut i = 0;
        while i < self.items.len() {
            if !self.h.checkpoint() {
                return Err(Stop);
            }
            let mut item = self.items[i].clone();
            item.dst = item.dst.map(|d| self.rewrite(d));
            let current = item.src.to_string_lossy().into_owned();
            self.h.update(|v| v.current = Some(current));
            if !self.is_skipped(&item.src) {
                self.attempt(&item.src, |r, a| r.item(i, &item, a))?;
            }
            self.bytes += item.size;
            let bytes = self.bytes;
            self.h.update(|v| {
                v.files_done += 1;
                v.bytes_done = bytes;
            });
            i += 1;
        }
        // Figli prima dei genitori (le cartelle sono in pre-ordine); mai ricorsivo: si rimuovono
        // solo le cartelle rimaste vuote, quelle con elementi saltati o esclusi restano.
        for dir in std::mem::take(&mut self.src_dirs).iter().rev() {
            if !self.h.checkpoint() {
                return Err(Stop);
            }
            if is_empty_dir(dir) {
                self.attempt(dir, |r, _| Ok(r.exec.delete(dir, true, true)?))?;
            }
        }
        Ok(())
    }

    fn rewrite(&self, mut d: PathBuf) -> PathBuf {
        for (old, new) in &self.renamed {
            if let Ok(rest) = d.strip_prefix(old) {
                d = if rest.as_os_str().is_empty() { new.clone() } else { new.join(rest) };
            }
        }
        d
    }

    fn is_skipped(&self, src: &Path) -> bool {
        self.skipped.iter().any(|s| src.starts_with(s) || s.starts_with(src))
    }

    /// Esegue `op` finché riesce, chiedendo all'utente cosa fare a ogni errore.
    fn attempt(&mut self, src: &Path, mut op: impl FnMut(&mut Self, &mut Attempt) -> Result<(), Fail>) -> Result<(), Stop> {
        let mut a = Attempt::default();
        loop {
            let e = match op(self, &mut a) {
                Ok(()) => return Ok(()),
                Err(Fail::Stop) => return Err(Stop),
                Err(Fail::Err(e)) => e,
            };
            // Compresa la copia interrotta dal callback per l'annullamento.
            if self.h.cancelled() {
                return Err(Stop);
            }
            let deletable = can_delete_permanently(&e);
            let action = if deletable && self.permanent_all {
                ErrorAction::DeletePermanent
            } else if self.skip_all.contains(&e.code) {
                ErrorAction::Skip
            } else {
                self.ask_error(&e, deletable)?
            };
            match action {
                // ponytail: RetryElevated vale come Retry finché il Task 17 non aggiunge l'helper elevato.
                ErrorAction::Retry | ErrorAction::RetryElevated => {}
                ErrorAction::DeletePermanent => a.permanent = deletable,
                ErrorAction::Skip => {
                    self.skipped.push(src.to_path_buf());
                    self.h.update(|v| v.errors.push(e));
                    return Ok(());
                }
                ErrorAction::Cancel => return Err(Stop),
            }
        }
    }

    fn ask_error(&mut self, e: &AppError, deletable: bool) -> Result<ErrorAction, Stop> {
        let job_id = self.h.job_id();
        let reply = self.h.ask(|prompt_id| Prompt::Error {
            prompt_id,
            job_id,
            error: e.clone(),
            can_elevate: e.code == ErrorCode::AccessDenied,
            can_delete_permanently: deletable,
        });
        let Some(PromptReply::Error { action, apply_to_all }) = reply else { return Err(Stop) };
        if apply_to_all {
            match action {
                ErrorAction::Skip => self.skip_all.push(e.code),
                ErrorAction::DeletePermanent if deletable => self.permanent_all = true,
                _ => {}
            }
        }
        Ok(action)
    }

    fn item(&mut self, i: usize, item: &FileItem, a: &mut Attempt) -> Result<(), Fail> {
        let src = &item.src;
        match &self.spec.kind {
            JobKind::Delete { permanent } => Ok(self.exec.delete(src, item.is_dir, *permanent || a.permanent)?),
            JobKind::Mkdir => self.mkdir_all(&normalize_lexical(src)),
            JobKind::Rename => Ok(self.exec.rename(src, &dst_of(item)?)?),
            JobKind::Copy | JobKind::Move if item.is_dir => self.dir(i, item, a),
            JobKind::Copy | JobKind::Move => self.file(item, a),
        }
    }

    fn file(&mut self, item: &FileItem, a: &mut Attempt) -> Result<(), Fail> {
        let (src, is_move) = (&item.src, self.spec.kind == JobKind::Move);
        if a.created.is_none() {
            let mut dst = dst_of(item)?;
            let mut overwrite = false;
            if exists(&dst) {
                match self.decide(src, &dst)? {
                    Decision::Skip => return Ok(()),
                    Decision::Rename(new) => dst = new,
                    Decision::Overwrite => overwrite = true,
                    Decision::Cancel => return Err(Fail::Stop),
                }
            }
            if is_move {
                // Anche con unità diverse si prova: un'unità `subst` o un volume montato possono
                // stare sullo stesso volume fisico (e viceversa dare NOT_SAME_DEVICE).
                match self.exec.move_path(src, &dst, overwrite) {
                    Err(e) if e.code == ErrorCode::NotSameDevice => {}
                    r => return Ok(r?),
                }
            }
            let (h, base) = (self.h, self.bytes);
            self.exec.copy_file(src, &dst, overwrite, &mut |n| {
                h.update(|v| v.bytes_done = base + n);
                h.checkpoint()
            })?;
            a.created = Some(dst);
        }
        if is_move {
            // Il sorgente si elimina solo dopo che la copia è riuscita.
            self.exec.delete(src, false, true)?;
        }
        Ok(())
    }

    /// Cartella di Copy/Move. Se è seguita dal proprio contenuto (espansa da `expand`) basta
    /// crearla; altrimenti (Move sulla stessa unità) si sposta in blocco con `move_path`.
    fn dir(&mut self, i: usize, item: &FileItem, a: &mut Attempt) -> Result<(), Fail> {
        let src = &item.src;
        let is_move = self.spec.kind == JobKind::Move;
        let expanded = self.items.get(i + 1).is_some_and(|n| n.src.starts_with(src));
        if a.created.is_none() {
            let mut dst = dst_of(item)?;
            let mut merge = false;
            if exists(&dst) && same_file(src, &dst) {
                // Copia nella propria cartella madre: si può solo rinominare o saltare.
                match self.decide(src, &dst)? {
                    Decision::Skip => {
                        self.skipped.push(src.clone());
                        return Ok(());
                    }
                    Decision::Rename(new) => dst = new,
                    Decision::Overwrite => return Err(AppError::new(ErrorCode::SameFile, Some(src)).into()),
                    Decision::Cancel => return Err(Fail::Stop),
                }
            } else {
                // Una cartella esistente si unisce; un file al suo posto fa fallire mkdir/move_path.
                merge = is_real_dir(&dst);
            }
            if is_move && !expanded && !merge {
                match self.exec.move_path(src, &dst, false) {
                    Err(e) if e.code == ErrorCode::NotSameDevice => {}
                    r => return Ok(r?),
                }
            }
            if !merge {
                self.exec.mkdir(&dst)?;
            }
            if let Some(orig) = &item.dst
                && *orig != dst
            {
                self.renamed.push((orig.clone(), dst.clone()));
            }
            if is_move {
                self.src_dirs.push(src.clone());
            }
            a.created = Some(dst);
        }
        if is_move && !expanded {
            self.splice_children(i)?;
        }
        Ok(())
    }

    /// Move di una cartella non espansa che non si è potuta spostare in blocco (altro volume o
    /// destinazione da unire): il suo contenuto entra nella coda subito dopo di lei.
    fn splice_children(&mut self, i: usize) -> Result<(), AppError> {
        let item = &self.items[i];
        let parent = item.dst.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        let copy = JobSpec {
            kind: JobKind::Copy,
            sources: vec![item.src.clone()],
            dest: parent,
            filter: None,
            policy: self.spec.policy.clone(),
        };
        // Le `dst` così costruite sono quelle originali dei figli (`dst` della cartella + nome).
        let mut children = expand(&copy)?;
        children.remove(0); // la cartella stessa
        let (n, bytes) = (children.len() as u64, children.iter().map(|c| c.size).sum::<u64>());
        self.h.update(|v| {
            v.files_total += n;
            v.bytes_total += bytes;
        });
        self.items.splice(i + 1..i + 1, children);
        Ok(())
    }

    /// Decisione per una destinazione esistente. Una sovrascrittura (dalla politica, dall'utente
    /// o "per tutti") di un file su se stesso diventa un errore `SAME_FILE`.
    fn decide(&mut self, src: &Path, dst: &Path) -> Result<Decision, Fail> {
        let (sm, dm) = (meta(src)?, meta(dst)?);
        let d = match self.conflict_all.clone() {
            Some(Decision::Rename(_)) => Decision::Rename(next_free_name(dst)),
            Some(d) => d,
            None => match resolve(&self.spec.policy, src, dst, &sm, &dm) {
                Ok(Some(d)) => d,
                Ok(None) => self.ask_conflict(src, dst, sm, dm)?,
                // Stesso file con Ask: l'utente può ancora rinominare o saltare.
                Err(e) if e.code == ErrorCode::SameFile && self.spec.policy == ConflictPolicy::Ask => {
                    self.ask_conflict(src, dst, sm, dm)?
                }
                Err(e) => return Err(e.into()),
            },
        };
        match d {
            Decision::Rename(p) => Ok(Decision::Rename(normalize_lexical(&p))),
            Decision::Overwrite if same_file(src, dst) => Err(AppError::new(ErrorCode::SameFile, Some(src)).into()),
            d => Ok(d),
        }
    }

    fn ask_conflict(&mut self, src: &Path, dst: &Path, src_meta: Meta, dst_meta: Meta) -> Result<Decision, Fail> {
        let job_id = self.h.job_id();
        let reply = self.h.ask(|prompt_id| Prompt::Conflict {
            prompt_id,
            job_id,
            src: src.to_string_lossy().into_owned(),
            dst: dst.to_string_lossy().into_owned(),
            src_meta,
            dst_meta,
        });
        let Some(PromptReply::Conflict { decision, apply_to_all }) = reply else { return Err(Fail::Stop) };
        if apply_to_all && decision != Decision::Cancel {
            self.conflict_all = Some(decision.clone());
        }
        Ok(decision)
    }

    /// Crea `path` e le cartelle intermedie mancanti; se esiste già dà `ALREADY_EXISTS`.
    fn mkdir_all(&self, path: &Path) -> Result<(), Fail> {
        let missing: Vec<&Path> = path.ancestors().take_while(|p| !exists(p)).collect();
        if missing.is_empty() {
            self.exec.mkdir(path)?;
        }
        for d in missing.iter().rev() {
            self.exec.mkdir(d)?;
        }
        Ok(())
    }
}

fn dst_of(item: &FileItem) -> Result<PathBuf, AppError> {
    item.dst.clone().ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, Some(&item.src)))
}

/// Il Cestino ha rifiutato l'elemento: l'utente può scegliere l'eliminazione definitiva.
fn can_delete_permanently(e: &AppError) -> bool {
    e.code == ErrorCode::NoRecycleBin
        || (e.code == ErrorCode::InvalidName && e.detail.as_deref() == Some("recycle_bin_unsupported_name"))
}

/// `symlink_metadata`: anche un collegamento rotto occupa il nome.
fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(to_extended(p)).is_ok()
}

/// Cartella vera: una giunzione o un symlink non si attraversano per unire.
fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(to_extended(p)).is_ok_and(|m| m.is_dir())
}

fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(to_extended(p)).is_ok_and(|mut r| r.next().is_none())
}

fn meta(p: &Path) -> Result<Meta, AppError> {
    let m = std::fs::symlink_metadata(to_extended(p)).map_err(|e| AppError::from_io(&e, p))?;
    Ok(Meta { size: m.len(), modified_ms: (m.last_write_time() / 10_000) as i64 - FILETIME_UNIX_DIFF_MS })
}
