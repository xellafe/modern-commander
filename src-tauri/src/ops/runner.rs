//! Esecuzione di un job, un elemento alla volta, nel thread worker. I conflitti si risolvono con
//! la politica del job o chiedendo all'utente; ogni errore diventa un `Prompt::Error`.
//! Le destinazioni si creano sempre senza sostituire nulla: si sovrascrive solo con
//! `copy_file`/`move_path(overwrite = true)` dopo una decisione `Overwrite`.
//!
//! Un'operazione su posizioni solo locali passa per l'`Executor` (anche quello elevato); con una posizione
//! remota passa per il provider, e la copia tra provider diversi per `vfs::transfer::copy`.

use super::conflict::{free_name, resolve, same_loc};
use super::exec::Executor;
use super::model::{ConflictPolicy, Decision, FileItem, JobKind, JobSpec, ListEntry, Meta};
use super::plan::{entries, expand, expand_children, file_target, filter_of, norm};
use super::queue::{ErrorAction, Handle, JobState, Prompt, PromptReply};
use super::rooted::{RootedExecutor, job_roots};
use crate::error::{AppError, ErrorCode};
use crate::fs::listing::FILETIME_UNIX_DIFF_MS;
use crate::paths::{invalid_windows_name, to_extended};
use crate::vfs::loc::Loc;
use crate::vfs::provider::{Provider, Purpose, Stat};
use crate::vfs::router::Vfs;
use crate::vfs::transfer;
use std::os::windows::fs::MetadataExt;
use std::sync::Arc;

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
    created: Option<Loc>,
    /// I tentativi successivi passano per l'helper elevato ("Riprova come amministratore").
    elevated: bool,
}

pub(crate) fn run(h: &Handle, exec: Arc<dyn Executor>, vfs: &Vfs, spec: &JobSpec) -> JobState {
    let mut r = Runner {
        h,
        vfs,
        exec: exec.clone(),
        local: exec,
        elevate_all: false,
        rooted: None,
        local_job: spec.sources.iter().chain(&spec.dest).all(|l| !l.is_remote()),
        spec,
        to_file: false,
        items: Vec::new(),
        bytes: 0,
        policy: spec.policy.clone(),
        conflict_all: None,
        unsafe_all: None,
        skip_all: Vec::new(),
        permanent_all: false,
        skipped: Vec::new(),
        renamed: Vec::new(),
        src_dirs: Vec::new(),
        permanent_under: Vec::new(),
        again: false,
    };
    let items = match expand(spec, vfs) {
        Ok(items) => items,
        Err(e) => {
            h.update(|v| v.errors.push(e));
            return JobState::Failed;
        }
    };
    r.to_file = file_target(spec, vfs);
    // Come TC, una cartella di destinazione mancante si crea (con le intermedie) all'avvio; solo
    // dopo `expand`, che rifiuta per esempio la copia dentro se stessa: un job fallito non crea nulla.
    // Se la destinazione è il nome di un file (`file_target`) si crea solo la sua cartella.
    let dest_dir = match (&spec.kind, &spec.dest) {
        (JobKind::Copy | JobKind::Move, Some(dest)) => {
            let dest = norm(dest);
            if r.to_file { dest.parent() } else { Some(dest) }
        }
        _ => None,
    };
    if let Some(dest) = dest_dir.filter(|d| !exists(vfs, d)) {
        if r.attempt(&dest, |r, _| r.mkdir_all(&dest)).is_err() {
            return JobState::Cancelled;
        }
        if !r.skipped.is_empty() {
            return JobState::Done; // saltata: l'errore resta nel riepilogo
        }
    }
    h.update(|v| {
        v.files_total = items.len() as u64;
        v.bytes_total = items.iter().map(|i| i.size).sum();
    });
    r.items = items;
    match r.run() {
        Ok(()) => JobState::Done,
        Err(Stop) => JobState::Cancelled,
    }
}

struct Runner<'a> {
    h: &'a Handle,
    vfs: &'a Vfs,
    /// Esecutore del tentativo in corso: `local` oppure quello elevato.
    exec: Arc<dyn Executor>,
    local: Arc<dyn Executor>,
    /// "Riprova come amministratore" per tutti: il resto del job passa per l'helper elevato.
    elevate_all: bool,
    /// Radici del job risolte alla prima elevazione e riusate fino alla fine (vedi `rooted`).
    rooted: Option<RootedExecutor>,
    /// Job su sole posizioni locali: l'unico in cui l'helper elevato può servire.
    local_job: bool,
    spec: &'a JobSpec,
    /// La destinazione è il nome del file da creare (`file_target`).
    to_file: bool,
    /// Le `dst` restano quelle originali: le rinomine si applicano con `rewrite`.
    items: Vec<FileItem>,
    /// Byte degli elementi già elaborati.
    bytes: u64,
    /// Politica dei conflitti: quella del job, o quella scelta dall'utente "per tutti".
    policy: ConflictPolicy,
    conflict_all: Option<Decision>,
    /// Risposta "per tutti" all'avviso della scrittura non sicura.
    unsafe_all: Option<bool>,
    /// Codici di errore da saltare senza chiedere.
    skip_all: Vec<ErrorCode>,
    permanent_all: bool,
    // ponytail: ricerca lineare, O(elementi × saltati); un albero di prefissi se i salti diventano migliaia.
    /// Sorgenti saltate: si saltano anche i discendenti (Copy/Move) e gli antenati (Delete).
    skipped: Vec<Loc>,
    /// Cartelle di destinazione rinominate (vecchia, nuova), nell'ordine in cui sono avvenute.
    renamed: Vec<(Loc, Loc)>,
    /// Move: cartelle sorgente da rimuovere alla fine, se vuote.
    src_dirs: Vec<Loc>,
    /// Delete: cartelle rifiutate dal Cestino ed eliminate in modo definitivo, contenuto compreso.
    permanent_under: Vec<Loc>,
    /// L'elemento corrente è stato elencato e il suo contenuto messo al suo posto (Delete): si
    /// riparte dallo stesso indice senza contarlo.
    again: bool,
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
            let current = item.src.to_string();
            self.h.update(|v| v.current = Some(current));
            if !self.is_skipped(&item.src) {
                self.attempt(&item.src, |r, a| r.item(i, &item, a))?;
                if std::mem::take(&mut self.again) {
                    continue;
                }
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
            if is_empty_dir(self.vfs, dir) {
                self.attempt(dir, |r, _| Ok(r.delete_at(dir, true, true)?))?;
            }
        }
        Ok(())
    }

    fn rewrite(&self, mut d: Loc) -> Loc {
        for (old, new) in &self.renamed {
            if let Some(r) = rebase(&d, old, new) {
                d = r;
            }
        }
        d
    }

    fn is_skipped(&self, src: &Loc) -> bool {
        self.skipped.iter().any(|s| under(src, s) || under(s, src))
    }

    /// Esegue `op` finché riesce, chiedendo all'utente cosa fare a ogni errore.
    fn attempt(&mut self, src: &Loc, mut op: impl FnMut(&mut Self, &mut Attempt) -> Result<(), Fail>) -> Result<(), Stop> {
        let mut a = Attempt::default();
        loop {
            let res = match self.executor(&mut a) {
                Ok(x) => {
                    self.exec = x;
                    op(self, &mut a)
                }
                Err(e) => Err(Fail::Err(e)), // per esempio UAC rifiutato: prompt d'errore normale
            };
            let e = match res {
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
            } else if self.skip_all.contains(&e.code) || e.code == ErrorCode::UnsafeWrite {
                // `UNSAFE_WRITE`: l'utente ha appena scelto di non scrivere senza la scrittura sicura.
                ErrorAction::Skip
            } else {
                self.ask_error(&e, deletable)?
            };
            match action {
                // Retry ripete nello stesso modo, elevato compreso.
                ErrorAction::Retry => {}
                ErrorAction::RetryElevated => a.elevated = true,
                ErrorAction::DeletePermanent => a.permanent = deletable,
                ErrorAction::Skip => {
                    self.skipped.push(src.clone());
                    self.h.update(|v| v.errors.push(e));
                    return Ok(());
                }
                ErrorAction::Cancel => return Err(Stop),
            }
        }
    }

    /// Esecutore per il prossimo tentativo. Se l'helper non parte (UAC rifiutato) si torna
    /// all'esecutore locale: un semplice Riprova non ripropone il prompt UAC.
    fn executor(&mut self, a: &mut Attempt) -> Result<Arc<dyn Executor>, AppError> {
        if !(a.elevated || self.elevate_all) {
            return Ok(self.local.clone());
        }
        let inner = self.h.elevated().inspect_err(|_| {
            a.elevated = false;
            self.elevate_all = false;
        })?;
        // L'esecutore elevato cambia se l'helper è uscito ed è stato riavviato: le radici restano.
        let x = match self.rooted.take() {
            Some(r) => r.with_inner(inner),
            None => RootedExecutor::new(inner, job_roots(self.spec, self.vfs)),
        };
        self.rooted = Some(x.clone());
        Ok(Arc::new(x))
    }

    fn ask_error(&mut self, e: &AppError, deletable: bool) -> Result<ErrorAction, Stop> {
        let job_id = self.h.job_id();
        let can_elevate = e.code == ErrorCode::AccessDenied && self.local_job;
        let reply = self.h.ask(|prompt_id| Prompt::Error {
            prompt_id,
            job_id,
            error: e.clone(),
            can_elevate,
            can_delete_permanently: deletable,
        });
        let Some(PromptReply::Error { action, apply_to_all }) = reply else { return Err(Stop) };
        if apply_to_all {
            match action {
                ErrorAction::Skip => self.skip_all.push(e.code),
                ErrorAction::DeletePermanent if deletable => self.permanent_all = true,
                ErrorAction::RetryElevated => self.elevate_all = true,
                _ => {}
            }
        }
        Ok(action)
    }

    /// Avviso della scrittura non sicura (`transfer::copy`): una risposta "per tutti" vale per il resto del job.
    fn confirm_unsafe(&mut self, dst: &Loc) -> bool {
        if let Some(proceed) = self.unsafe_all {
            return proceed;
        }
        let job_id = self.h.job_id();
        let reply = self.h.ask(|prompt_id| Prompt::UnsafeWrite { prompt_id, job_id, path: dst.to_string() });
        let Some(PromptReply::UnsafeWrite { proceed, apply_to_all }) = reply else { return false };
        if apply_to_all {
            self.unsafe_all = Some(proceed);
        }
        proceed
    }

    fn item(&mut self, i: usize, item: &FileItem, a: &mut Attempt) -> Result<(), Fail> {
        let src = &item.src;
        match &self.spec.kind {
            JobKind::Delete { permanent } => self.delete(i, item, *permanent || a.permanent),
            JobKind::Mkdir => self.mkdir_all(&norm(src)),
            JobKind::Rename => Ok(self.rename(src, &dst_of(item)?)?),
            JobKind::Copy | JobKind::Move => {
                self.check_name(item)?;
                if item.is_dir { self.dir(i, item, a) } else { self.file(item, a) }
            }
        }
    }

    /// Un nome remoto che Windows non accetta blocca il download di quell'elemento: mai rinominato in silenzio.
    /// Si guarda il nome della sorgente, perché unito alla cartella locale un `\` o un `:` porterebbe altrove;
    /// con `to_file` il nome locale è quello scelto dall'utente.
    fn check_name(&self, item: &FileItem) -> Result<(), AppError> {
        let download = item.src.is_remote() && matches!(item.dst, Some(Loc::Local(_)));
        if download && !self.to_file && item.src.file_name().is_some_and(|n| invalid_windows_name(&n)) {
            return Err(item.src.error(ErrorCode::InvalidName));
        }
        Ok(())
    }

    /// Una cartella non elencata va nel Cestino intera; in modo definitivo (anche dopo un rifiuto
    /// del Cestino) o con un filtro prima si elenca, e il suo contenuto passa prima di lei.
    fn delete(&mut self, i: usize, item: &FileItem, permanent: bool) -> Result<(), Fail> {
        let permanent = permanent || self.permanent_under.iter().any(|p| under(&item.src, p));
        let filtered = filter_of(self.spec).is_some();
        if self.items[i].unlisted && (permanent || filtered) {
            self.list(i)?;
            if permanent {
                self.permanent_under.push(item.src.clone());
            }
            return Ok(());
        }
        if item.is_dir && filtered {
            return Ok(()); // con un filtro le cartelle restano: conterrebbero i file esclusi
        }
        Ok(self.delete_at(&item.src, item.is_dir, permanent)?)
    }

    fn file(&mut self, item: &FileItem, a: &mut Attempt) -> Result<(), Fail> {
        let (src, is_move) = (&item.src, self.spec.kind == JobKind::Move);
        if a.created.is_none() {
            let mut dst = dst_of(item)?;
            let mut overwrite = false;
            if exists(self.vfs, &dst) {
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
                match self.move_path(src, &dst, overwrite) {
                    Err(e) if e.code == ErrorCode::NotSameDevice => {}
                    r => return Ok(r?),
                }
            }
            let (h, base) = (self.h, self.bytes);
            // Move: la copia deve essere su disco prima che si elimini il sorgente.
            self.copy_file(src, &dst, overwrite, is_move, &mut |n| {
                h.update(|v| v.bytes_done = base + n);
                h.checkpoint()
            })?;
            a.created = Some(dst);
        }
        if is_move {
            // Il sorgente si elimina solo dopo che la copia è riuscita.
            self.delete_at(src, false, true)?;
        }
        Ok(())
    }

    /// Cartella di Copy/Move. Se il suo contenuto è già nell'elenco basta crearla; un Move senza
    /// filtro di una cartella non elencata prova a spostarla in blocco con `move_path`, altrimenti
    /// (altro volume, destinazione da unire, cartella illeggibile in `expand`) la si elenca qui.
    /// Un collegamento a cartella (giunzione, symlink, `dir_link` remoto) si può solo spostare in blocco:
    /// copiarlo darebbe una cartella vuota, quindi diventa un errore `UNSUPPORTED` (detail `reparse_point`).
    fn dir(&mut self, i: usize, item: &FileItem, a: &mut Attempt) -> Result<(), Fail> {
        let src = &item.src;
        let is_move = self.spec.kind == JobKind::Move;
        let link = is_dir_link(self.vfs, src);
        if link && !is_move {
            return Err(unsupported_link(src).into());
        }
        if a.created.is_none() {
            let mut dst = dst_of(item)?;
            let mut merge = false;
            if exists(self.vfs, &dst) && same_loc(src, &dst) {
                // Copia nella propria cartella madre: si può solo rinominare o saltare.
                match self.decide(src, &dst)? {
                    Decision::Skip => {
                        self.skipped.push(src.clone());
                        return Ok(());
                    }
                    Decision::Rename(new) => dst = new,
                    Decision::Overwrite => return Err(src.error(ErrorCode::SameFile).into()),
                    Decision::Cancel => return Err(Fail::Stop),
                }
            } else {
                // Una cartella esistente si unisce; un file al suo posto fa fallire mkdir/move_path.
                merge = is_real_dir(self.vfs, &dst);
            }
            if is_move && (item.unlisted || link) && !merge && filter_of(self.spec).is_none() {
                match self.move_path(src, &dst, false) {
                    Err(e) if e.code == ErrorCode::NotSameDevice => {}
                    r => return Ok(r?),
                }
            }
            if link {
                return Err(unsupported_link(src).into());
            }
            // Prima di creare la cartella: se non si riesce a elencarla e la si salta, non resta vuota.
            if self.items[i].unlisted {
                self.list(i)?;
            }
            if !merge {
                self.mkdir_at(&dst)?;
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
        Ok(())
    }

    /// Elenca la cartella `i` con l'esecutore del tentativo (anche quello elevato) o con il provider, e ne
    /// mette il contenuto nell'elenco: subito dopo di lei per Copy/Move, prima di lei per Delete, che poi
    /// riparte da lì (`again`). Le `dst` dei figli sono quelle originali, come le altre.
    fn list(&mut self, i: usize) -> Result<(), Fail> {
        let dir = self.items[i].clone();
        let children = expand_children(self.spec, self.vfs, &dir, self.list_at(&dir.src)?);
        let (n, bytes) = (children.len() as u64, children.iter().map(|c| c.size).sum::<u64>());
        self.h.update(|v| {
            v.files_total += n;
            v.bytes_total += bytes;
        });
        self.items[i].unlisted = false;
        if matches!(self.spec.kind, JobKind::Delete { .. }) {
            self.items.splice(i..i, children);
            self.again = true;
        } else {
            self.items.splice(i + 1..i + 1, children);
        }
        Ok(())
    }

    /// Decisione per una destinazione esistente. Una sovrascrittura (dalla politica, dall'utente
    /// o "per tutti") di un file su se stesso diventa un errore `SAME_FILE`.
    fn decide(&mut self, src: &Loc, dst: &Loc) -> Result<Decision, Fail> {
        let (sm, dm) = (meta(self.vfs, src)?, meta(self.vfs, dst)?);
        let d = match self.conflict_all.clone() {
            Some(Decision::Rename(_)) => Decision::Rename(free_name(self.vfs, dst)),
            Some(d) => d,
            None => match resolve(self.vfs, &self.policy, src, dst, &sm, &dm) {
                Ok(Some(d)) => d,
                Ok(None) => self.ask_conflict(src, dst, sm, dm)?,
                // Stesso file con Ask: l'utente può ancora rinominare o saltare.
                Err(e) if e.code == ErrorCode::SameFile && self.policy == ConflictPolicy::Ask => {
                    self.ask_conflict(src, dst, sm, dm)?
                }
                Err(e) => return Err(e.into()),
            },
        };
        match d {
            Decision::Rename(p) => Ok(Decision::Rename(norm(&p))),
            Decision::Overwrite if same_loc(src, dst) => Err(src.error(ErrorCode::SameFile).into()),
            d => Ok(d),
        }
    }

    fn ask_conflict(&mut self, src: &Loc, dst: &Loc, src_meta: Meta, dst_meta: Meta) -> Result<Decision, Fail> {
        let job_id = self.h.job_id();
        let same = same_loc(src, dst);
        let suggested = free_name(self.vfs, dst);
        let reply = self.h.ask(|prompt_id| Prompt::Conflict {
            prompt_id,
            job_id,
            src: src.to_string(),
            dst: dst.to_string(),
            src_meta,
            dst_meta,
            same_file: same,
            suggested_name: suggested.to_string(),
        });
        match reply {
            Some(PromptReply::Conflict { decision, apply_to_all }) => {
                if apply_to_all && decision != Decision::Cancel {
                    self.conflict_all = Some(decision.clone());
                }
                Ok(decision)
            }
            Some(PromptReply::ConflictPolicy { policy, apply_to_all }) => {
                // Prima di `resolve`: anche se questo conflitto dà `SAME_FILE`, vale per i successivi.
                if apply_to_all {
                    self.policy = policy.clone();
                }
                // `reply` rifiuta `Ask`: con le altre politiche `resolve` decide sempre.
                Ok(resolve(self.vfs, &policy, src, dst, &src_meta, &dst_meta)?.unwrap_or(Decision::Skip))
            }
            _ => Err(Fail::Stop),
        }
    }

    /// Crea `path` e le cartelle intermedie mancanti; se esiste già dà `ALREADY_EXISTS`.
    fn mkdir_all(&self, path: &Loc) -> Result<(), Fail> {
        let ancestors = std::iter::successors(Some(path.clone()), Loc::parent);
        let missing: Vec<Loc> = ancestors.take_while(|p| !exists(self.vfs, p)).collect();
        if missing.is_empty() {
            self.mkdir_at(path)?;
        }
        for d in missing.iter().rev() {
            self.mkdir_at(d)?;
        }
        Ok(())
    }

    // Operazioni elementari: in locale con l'esecutore del tentativo, sul remoto con il provider.

    fn remote(&self, l: &Loc) -> Result<(Arc<dyn Provider>, String), AppError> {
        self.vfs.remote(l, Purpose::Transfer)
    }

    fn mkdir_at(&self, d: &Loc) -> Result<(), AppError> {
        match d {
            Loc::Local(p) => self.exec.mkdir(p),
            r => {
                let (p, path) = self.remote(r)?;
                p.mkdir(&path)
            }
        }
    }

    /// Sul remoto non c'è Cestino: come sulle unità di rete, `NO_RECYCLE_BIN` e l'utente sceglie se
    /// eliminare in modo definitivo.
    fn delete_at(&self, l: &Loc, is_dir: bool, permanent: bool) -> Result<(), AppError> {
        match l {
            Loc::Local(p) => self.exec.delete(p, is_dir, permanent),
            r if !permanent => Err(r.error(ErrorCode::NoRecycleBin)),
            r => {
                let (p, path) = self.remote(r)?;
                p.delete(&path, is_dir)
            }
        }
    }

    fn list_at(&self, dir: &Loc) -> Result<Vec<ListEntry>, AppError> {
        match dir {
            Loc::Local(p) => self.exec.list_dir(p),
            r => entries(self.vfs, r),
        }
    }

    /// Spostamento in blocco: `move_path` in locale, `Provider::rename` nella stessa connessione; tra
    /// provider diversi `NOT_SAME_DEVICE`, e chi chiama ripiega su copia ed eliminazione.
    fn move_path(&self, src: &Loc, dst: &Loc, overwrite: bool) -> Result<(), AppError> {
        match (src, dst) {
            (Loc::Local(s), Loc::Local(d)) => self.exec.move_path(s, d, overwrite),
            _ if src.is_remote() && src.volume_key() == dst.volume_key() => {
                let ((p, from), (_, to)) = (self.remote(src)?, self.remote(dst)?);
                p.rename(&from, &to, overwrite)
            }
            _ => Err(src.error(ErrorCode::NotSameDevice)),
        }
    }

    fn rename(&self, src: &Loc, dst: &Loc) -> Result<(), AppError> {
        match (src, dst) {
            (Loc::Local(s), Loc::Local(d)) => self.exec.rename(s, d),
            _ => self.move_path(src, dst, false),
        }
    }

    /// Copia di un file: tra due posizioni locali con l'esecutore, altrimenti con `transfer::copy`. Un
    /// "Annulla" sull'avviso della scrittura non sicura diventa `UNSAFE_WRITE`, che salta l'elemento.
    fn copy_file(&mut self, src: &Loc, dst: &Loc, overwrite: bool, flush: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError> {
        if let (Loc::Local(s), Loc::Local(d)) = (src, dst) {
            return self.exec.copy_file(s, d, overwrite, flush, progress);
        }
        let vfs = self.vfs;
        let mut declined = false;
        let mut confirm = |d: &Loc| {
            let ok = self.confirm_unsafe(d);
            declined = !ok;
            ok
        };
        match transfer::copy(vfs, src, dst, overwrite, &mut confirm, progress) {
            Err(e) if declined && e.code == ErrorCode::Cancelled => Err(dst.error(ErrorCode::UnsafeWrite)),
            r => r,
        }
    }
}

fn dst_of(item: &FileItem) -> Result<Loc, AppError> {
    item.dst.clone().ok_or_else(|| item.src.error(ErrorCode::InvalidRequest))
}

/// `a` coincide con `b` o sta sotto di lei; in locale per componenti, come `Path::starts_with`.
fn under(a: &Loc, b: &Loc) -> bool {
    match (a, b) {
        (Loc::Local(a), Loc::Local(b)) => a.starts_with(b),
        _ => a.is_within(b),
    }
}

/// `d` con il prefisso `old` sostituito da `new`; `None` se `d` non sta sotto `old`.
fn rebase(d: &Loc, old: &Loc, new: &Loc) -> Option<Loc> {
    match (d, old, new) {
        (Loc::Local(d), Loc::Local(old), Loc::Local(new)) => {
            let rest = d.strip_prefix(old).ok()?;
            Some(Loc::Local(if rest.as_os_str().is_empty() { new.clone() } else { new.join(rest) }))
        }
        (Loc::Remote { path, .. }, Loc::Remote { path: o, .. }, _) if d.is_within(old) => Some(new.join(&path[o.len()..])),
        _ => None,
    }
}

/// Il Cestino ha rifiutato l'elemento: l'utente può scegliere l'eliminazione definitiva.
fn can_delete_permanently(e: &AppError) -> bool {
    e.code == ErrorCode::NoRecycleBin
        || (e.code == ErrorCode::InvalidName && e.detail.as_deref() == Some("recycle_bin_unsupported_name"))
}

// Controlli sulle posizioni: `std::fs` in locale, `Provider::stat`/`list` sul remoto.

fn remote_stat(vfs: &Vfs, l: &Loc) -> Result<Stat, AppError> {
    let (p, path) = vfs.remote(l, Purpose::Transfer)?;
    p.stat(&path)
}

/// `symlink_metadata`: anche un collegamento rotto occupa il nome.
fn exists(vfs: &Vfs, l: &Loc) -> bool {
    match l {
        Loc::Local(p) => std::fs::symlink_metadata(to_extended(p)).is_ok(),
        r => remote_stat(vfs, r).is_ok(),
    }
}

/// Cartella vera: una giunzione o un symlink non si attraversano per unire.
fn is_real_dir(vfs: &Vfs, l: &Loc) -> bool {
    match l {
        Loc::Local(p) => std::fs::symlink_metadata(to_extended(p)).is_ok_and(|m| m.is_dir()),
        r => remote_stat(vfs, r).is_ok_and(|s| s.is_dir && !s.dir_link),
    }
}

/// Giunzione o collegamento simbolico (nella std di Windows `is_symlink` copre entrambi); sul remoto `dir_link`.
fn is_dir_link(vfs: &Vfs, l: &Loc) -> bool {
    match l {
        Loc::Local(p) => std::fs::symlink_metadata(to_extended(p)).is_ok_and(|m| m.file_type().is_symlink()),
        r => remote_stat(vfs, r).is_ok_and(|s| s.dir_link),
    }
}

fn unsupported_link(l: &Loc) -> AppError {
    AppError { detail: Some("reparse_point".into()), ..l.error(ErrorCode::Unsupported) }
}

fn is_empty_dir(vfs: &Vfs, l: &Loc) -> bool {
    match l {
        Loc::Local(p) => std::fs::read_dir(to_extended(p)).is_ok_and(|mut r| r.next().is_none()),
        r => entries(vfs, r).is_ok_and(|e| e.is_empty()),
    }
}

/// Sul remoto una data mancante vale 0.
fn meta(vfs: &Vfs, l: &Loc) -> Result<Meta, AppError> {
    match l {
        Loc::Local(p) => {
            let m = std::fs::symlink_metadata(to_extended(p)).map_err(|e| AppError::from_io(&e, p))?;
            Ok(Meta { size: m.len(), modified_ms: (m.last_write_time() / 10_000) as i64 - FILETIME_UNIX_DIFF_MS })
        }
        r => remote_stat(vfs, r).map(|s| Meta { size: s.size, modified_ms: s.modified_ms.unwrap_or(0) }),
    }
}
