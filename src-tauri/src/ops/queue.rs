//! Coda delle operazioni. Un thread scheduler avvia i job nell'ordine della coda, al massimo un
//! job alla volta per volume (per connessione sul remoto); ogni job avviato gira in un proprio thread
//! worker (`runner`).
//!
//! Stati finali: `Done` anche se alcuni elementi sono stati saltati dopo un errore (gli errori
//! restano in `JobView.errors`); `Cancelled` se annullato dall'utente; `Failed` solo per un errore
//! fuori dai prompt per elemento (per esempio `expand` che fallisce).
//! I job finiti restano in elenco finché il frontend non chiama `clear_finished`.

use super::exec::Executor;
use super::model::{ConflictPolicy, Decision, JobKind, JobSpec, Meta};
use super::plan::norm;
use super::runner;
use crate::error::{AppError, ErrorCode};
use crate::paths::{normalize_lexical, same_path};
use crate::vfs::loc::Loc;
use crate::vfs::router::Vfs;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub type JobId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Running,
    Paused,
    /// In attesa della risposta a un prompt.
    Waiting,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_finished(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed | JobState::Cancelled)
    }
}

/// `files_*` contano gli elementi di `expand` (cartelle comprese), anche quelli saltati.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: JobId,
    pub kind: JobKind,
    pub state: JobState,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: Option<String>,
    /// Media mobile sugli ultimi 2 s.
    pub speed_bps: u64,
    /// Chiavi di scheduling (`Loc::volume_key`): unità o condivisione in locale, connessione sul remoto.
    pub volumes: Vec<String>,
    /// Cartella di destinazione di Copy/Move (per la barra di stato); `None` per le altre operazioni.
    pub dest: Option<String>,
    /// Riepilogo finale: un errore per ogni elemento saltato dopo un errore.
    pub errors: Vec<AppError>,
    /// Prompt in attesa di risposta: `list_jobs` lo ripropone se l'evento `job-prompt` è andato perso.
    pub prompt: Option<Prompt>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Prompt {
    /// `same_file`: sorgente e destinazione sono lo stesso file (la UI nasconde "Sovrascrivi");
    /// `suggested_name`: percorso completo libero da proporre per "Rinomina".
    Conflict {
        prompt_id: u64,
        job_id: JobId,
        src: String,
        dst: String,
        src_meta: Meta,
        dst_meta: Meta,
        same_file: bool,
        suggested_name: String,
    },
    /// `can_delete_permanently`: il Cestino ha rifiutato l'elemento (`NO_RECYCLE_BIN`, oppure
    /// `INVALID_NAME` con detail `recycle_bin_unsupported_name`).
    Error { prompt_id: u64, job_id: JobId, error: AppError, can_elevate: bool, can_delete_permanently: bool },
    /// Il server non permette la scrittura sicura (temporaneo e rinomina) su `path`: si scrive direttamente
    /// solo se l'utente conferma.
    UnsafeWrite { prompt_id: u64, job_id: JobId, path: String },
}

impl Prompt {
    pub fn prompt_id(&self) -> u64 {
        match self {
            Prompt::Conflict { prompt_id, .. } | Prompt::Error { prompt_id, .. } | Prompt::UnsafeWrite { prompt_id, .. } => {
                *prompt_id
            }
        }
    }
}

/// `apply_to_all`: per i conflitti vale per i conflitti successivi dello stesso job; per gli errori
/// per gli errori successivi con lo stesso codice (solo `Skip`; `DeletePermanent` vale per i rifiuti
/// del Cestino successivi). Con `Retry` è ignorato: ripeterebbe all'infinito un errore persistente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PromptReply {
    /// `Rename` deve essere un percorso assoluto nella stessa cartella di `dst`.
    Conflict { decision: Decision, apply_to_all: bool },
    /// Risolve il conflitto con `resolve(policy, ..)` ("Solo se più recente", "Sovrascrivi se più
    /// grande"...); con `apply_to_all` la politica vale per i conflitti successivi. Non `Ask`.
    ConflictPolicy { policy: ConflictPolicy, apply_to_all: bool },
    Error { action: ErrorAction, apply_to_all: bool },
    /// `proceed = false` salta l'elemento (`UNSAFE_WRITE` tra gli errori del job); con `apply_to_all` la
    /// risposta vale per il resto del job.
    UnsafeWrite { proceed: bool, apply_to_all: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ErrorAction {
    Retry,
    Skip,
    Cancel,
    /// Riprova con l'helper elevato; con `apply_to_all` anche per il resto del job.
    RetryElevated,
    /// Solo con `can_delete_permanently`: elimina l'elemento in modo definitivo.
    DeletePermanent,
}

/// Primo job in coda (nell'ordine dato) i cui volumi sono tutti liberi.
pub fn next_runnable(jobs: &[JobView], busy: &HashSet<String>) -> Option<JobId> {
    jobs.iter()
        .find(|j| j.state == JobState::Queued && j.volumes.iter().all(|v| !busy.contains(v)))
        .map(|j| j.id)
}

pub enum Event {
    Updated(JobView),
    Prompt(Prompt),
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// Esecutore elevato condiviso, avviato al primo uso (`elevation::shared` nell'app).
pub type Elevate = Arc<dyn Fn() -> Result<Arc<dyn Executor>, AppError> + Send + Sync>;

const EMIT_EVERY: Duration = Duration::from_millis(100);
const SPEED_WINDOW: Duration = Duration::from_secs(2);

struct Job {
    view: JobView,
    /// Preso dal worker all'avvio.
    spec: Option<JobSpec>,
    started: bool,
    pause: bool,
    cancel: bool,
    reply: Option<PromptReply>,
    last_emit: Instant,
    samples: VecDeque<(Instant, u64)>,
}

struct State {
    jobs: Vec<Job>,
    next_id: JobId,
    next_prompt: u64,
}

impl State {
    fn job(&mut self, id: JobId) -> Result<&mut Job, AppError> {
        self.jobs.iter_mut().find(|j| j.view.id == id).ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, None))
    }
}

struct Inner {
    state: Mutex<State>,
    /// Unica condition variable: scheduler, pausa e prompt; ogni cambiamento fa `notify_all`.
    cv: Condvar,
    exec: Arc<dyn Executor>,
    elevate: Elevate,
    vfs: Arc<Vfs>,
    sink: Sink,
}

#[derive(Clone)]
pub struct Queue(Arc<Inner>);

impl Queue {
    // ponytail: lo scheduler non termina mai (nessuno shutdown all'uscita dell'app, per ora).
    pub fn new(exec: Arc<dyn Executor>, elevate: Elevate, vfs: Arc<Vfs>, sink: Sink) -> Self {
        let state = State { jobs: Vec::new(), next_id: 0, next_prompt: 0 };
        let q = Queue(Arc::new(Inner { state: Mutex::new(state), cv: Condvar::new(), exec, elevate, vfs, sink }));
        let s = q.clone();
        std::thread::spawn(move || s.schedule());
        q
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Lo stato resta coerente anche se un worker è andato in panico tenendo il lock.
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<'a>(&self, g: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.0.cv.wait(g).unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, job: &mut Job) {
        job.last_emit = Instant::now();
        (self.0.sink)(Event::Updated(job.view.clone()));
    }

    fn set_state(&self, job: &mut Job, state: JobState) {
        if state != JobState::Running {
            job.view.speed_bps = 0;
            job.samples.clear();
        }
        job.view.state = state;
        self.emit(job);
        self.0.cv.notify_all();
    }

    fn schedule(&self) {
        let mut st = self.lock();
        loop {
            let views: Vec<JobView> = st.jobs.iter().map(|j| j.view.clone()).collect();
            let busy: HashSet<String> = st
                .jobs
                .iter()
                .filter(|j| j.started && !j.view.state.is_finished())
                .flat_map(|j| j.view.volumes.iter().cloned())
                .collect();
            match next_runnable(&views, &busy) {
                Some(id) => self.start(&mut st, id),
                None => st = self.wait(st),
            }
        }
    }

    fn start(&self, st: &mut State, id: JobId) {
        let Ok(job) = st.job(id) else { return };
        let Some(spec) = job.spec.take() else { return };
        job.started = true;
        self.set_state(job, JobState::Running);
        let h = Handle { q: self.clone(), id };
        std::thread::spawn(move || {
            // Un panico non deve lasciare il job `Running` con i volumi occupati per sempre.
            // (Con `panic = "abort"` del profilo release l'app termina comunque.)
            let end = catch_unwind(AssertUnwindSafe(|| runner::run(&h, h.q.0.exec.clone(), &h.q.0.vfs, &spec))).unwrap_or_else(|_| {
                let e = AppError { detail: Some("internal_panic".into()), ..AppError::new(ErrorCode::Io, None) };
                h.update(|v| v.errors.push(e));
                JobState::Failed
            });
            h.finish(end);
        });
    }

    pub fn enqueue(&self, spec: JobSpec) -> JobId {
        let mut volumes: Vec<String> = spec.sources.iter().chain(spec.dest.iter()).map(|l| norm(l).volume_key()).collect();
        volumes.sort();
        volumes.dedup();
        let mut st = self.lock();
        st.next_id += 1;
        let id = st.next_id;
        let view = JobView {
            id,
            kind: spec.kind.clone(),
            state: JobState::Queued,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            current: None,
            speed_bps: 0,
            volumes,
            dest: match spec.kind {
                JobKind::Copy | JobKind::Move => spec.dest.as_ref().map(Loc::to_string),
                _ => None,
            },
            errors: Vec::new(),
            prompt: None,
        };
        let job = Job {
            view,
            spec: Some(spec),
            started: false,
            pause: false,
            cancel: false,
            reply: None,
            last_emit: Instant::now(),
            samples: VecDeque::new(),
        };
        st.jobs.push(job);
        let job = st.jobs.last_mut().expect("appena inserito");
        self.emit(job);
        self.0.cv.notify_all();
        id
    }

    /// Un job avviato si ferma al prossimo controllo (tra un file e l'altro o durante la copia);
    /// uno in attesa di risposta resta `Waiting` e si ferma dopo la risposta.
    pub fn pause(&self, id: JobId) -> Result<(), AppError> {
        let mut st = self.lock();
        let job = st.job(id)?;
        if job.view.state.is_finished() {
            return Ok(());
        }
        job.pause = true;
        if matches!(job.view.state, JobState::Queued | JobState::Running) {
            self.set_state(job, JobState::Paused);
        }
        Ok(())
    }

    pub fn resume(&self, id: JobId) -> Result<(), AppError> {
        let mut st = self.lock();
        let job = st.job(id)?;
        job.pause = false;
        if job.view.state == JobState::Paused {
            let s = if job.started { JobState::Running } else { JobState::Queued };
            self.set_state(job, s);
        }
        Ok(())
    }

    /// Un job avviato termina `Cancelled` appena il worker se ne accorge: subito se è in pausa o in
    /// attesa di un prompt, altrimenti al prossimo controllo (la copia in corso si interrompe).
    pub fn cancel(&self, id: JobId) -> Result<(), AppError> {
        let mut st = self.lock();
        let job = st.job(id)?;
        if job.view.state.is_finished() {
            return Ok(());
        }
        if job.started {
            job.cancel = true;
            self.0.cv.notify_all();
        } else {
            job.spec = None;
            self.set_state(job, JobState::Cancelled);
        }
        Ok(())
    }

    /// Sposta il job alla posizione `index` dell'elenco (limitata alla fine).
    pub fn move_to(&self, id: JobId, index: usize) -> Result<(), AppError> {
        let mut st = self.lock();
        let from = st.jobs.iter().position(|j| j.view.id == id).ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, None))?;
        let job = st.jobs.remove(from);
        let to = index.min(st.jobs.len());
        st.jobs.insert(to, job);
        self.0.cv.notify_all();
        Ok(())
    }

    /// Un id sconosciuto, una risposta del tipo sbagliato, una politica `Ask` o un `Rename` fuori
    /// dalla cartella di `dst` danno `INVALID_REQUEST`, e il prompt resta in attesa.
    pub fn reply(&self, prompt_id: u64, reply: PromptReply) -> Result<(), AppError> {
        let invalid = || AppError::new(ErrorCode::InvalidRequest, None);
        let mut st = self.lock();
        let job = st
            .jobs
            .iter_mut()
            .find(|j| j.view.prompt.as_ref().is_some_and(|p| p.prompt_id() == prompt_id))
            .ok_or_else(invalid)?;
        let valid = match (&job.view.prompt, &reply) {
            (Some(Prompt::Conflict { dst, .. }), PromptReply::Conflict { decision: Decision::Rename(new), .. }) => {
                Loc::parse(dst).is_ok_and(|dst| valid_rename(new, &dst))
            }
            (Some(Prompt::Conflict { .. }), PromptReply::Conflict { .. }) => true,
            (Some(Prompt::Conflict { .. }), PromptReply::ConflictPolicy { policy, .. }) => *policy != ConflictPolicy::Ask,
            (Some(Prompt::Error { .. }), PromptReply::Error { .. }) => true,
            (Some(Prompt::UnsafeWrite { .. }), PromptReply::UnsafeWrite { .. }) => true,
            _ => false,
        };
        if !valid {
            return Err(invalid());
        }
        job.view.prompt = None;
        job.reply = Some(reply);
        self.0.cv.notify_all();
        Ok(())
    }

    pub fn list(&self) -> Vec<JobView> {
        self.lock().jobs.iter().map(|j| j.view.clone()).collect()
    }

    pub fn clear_finished(&self) {
        self.lock().jobs.retain(|j| !j.view.state.is_finished());
    }
}

/// Nome nuovo scelto dall'utente: assoluto, nella stessa cartella di `dst` e diverso da `dst`.
fn valid_rename(new: &Loc, dst: &Loc) -> bool {
    match (new, dst) {
        (Loc::Local(new), Loc::Local(dst)) => {
            let (n, d) = (normalize_lexical(new), normalize_lexical(dst));
            let same_parent = match (n.parent(), d.parent()) {
                (Some(a), Some(b)) => same_path(a, b),
                _ => false,
            };
            new.is_absolute() && n.file_name().is_some() && same_parent && !same_path(&n, &d)
        }
        // Remoto: già pulito (`Loc::parse`); la stessa cartella vuol dire anche la stessa connessione.
        (Loc::Remote { .. }, Loc::Remote { .. }) => new.file_name().is_some() && new.parent() == dst.parent() && new != dst,
        _ => false,
    }
}

/// Lato worker di un job avviato.
pub(crate) struct Handle {
    q: Queue,
    id: JobId,
}

impl Handle {
    pub(crate) fn job_id(&self) -> JobId {
        self.id
    }

    fn with<R>(&self, f: impl FnOnce(&mut Job) -> R) -> R {
        let mut st = self.q.lock();
        // Un job avviato e non finito non si rimuove mai dall'elenco.
        f(st.job(self.id).expect("job del worker"))
    }

    /// Esecutore elevato (al primo uso compare il prompt UAC).
    pub(crate) fn elevated(&self) -> Result<Arc<dyn Executor>, AppError> {
        (self.q.0.elevate)()
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.with(|j| j.cancel)
    }

    /// Aggiorna la vista; l'evento parte al massimo ogni 100 ms.
    pub(crate) fn update(&self, f: impl FnOnce(&mut JobView)) {
        let mut st = self.q.lock();
        let job = st.job(self.id).expect("job del worker");
        f(&mut job.view);
        let now = Instant::now();
        job.samples.push_back((now, job.view.bytes_done));
        while job.samples.front().is_some_and(|(t, _)| now - *t > SPEED_WINDOW) {
            job.samples.pop_front();
        }
        if let (Some(&(t0, b0)), Some(&(t1, b1))) = (job.samples.front(), job.samples.back())
            && t1 > t0
        {
            job.view.speed_bps = (b1.saturating_sub(b0) as f64 / (t1 - t0).as_secs_f64()) as u64;
        }
        if job.last_emit.elapsed() >= EMIT_EVERY {
            self.q.emit(job);
        }
    }

    /// Blocca finché il job è in pausa; `false` se è stato annullato.
    pub(crate) fn checkpoint(&self) -> bool {
        let mut st = self.q.lock();
        loop {
            let job = st.job(self.id).expect("job del worker");
            if job.cancel {
                return false;
            }
            if !job.pause {
                return true;
            }
            st = self.q.wait(st);
        }
    }

    /// Invia il prompt (anche in `JobView.prompt`) e aspetta la risposta, senza timeout. Se il job
    /// è stato messo in pausa nel frattempo, dopo la risposta aspetta anche la ripresa.
    /// `None` se il job viene annullato.
    pub(crate) fn ask(&self, make: impl FnOnce(u64) -> Prompt) -> Option<PromptReply> {
        let mut st = self.q.lock();
        st.next_prompt += 1;
        let prompt = make(st.next_prompt);
        let job = st.job(self.id).expect("job del worker");
        job.view.prompt = Some(prompt.clone());
        job.reply = None;
        self.q.set_state(job, JobState::Waiting);
        (self.q.0.sink)(Event::Prompt(prompt));
        let reply = loop {
            let job = st.job(self.id).expect("job del worker");
            if job.cancel {
                job.view.prompt = None;
                return None;
            }
            if let Some(r) = job.reply.take() {
                let s = if job.pause { JobState::Paused } else { JobState::Running };
                self.q.set_state(job, s);
                break r;
            }
            st = self.q.wait(st);
        };
        drop(st);
        // Annulla non aspetta la ripresa di un job in pausa.
        let cancel = matches!(
            reply,
            PromptReply::Conflict { decision: Decision::Cancel, .. } | PromptReply::Error { action: ErrorAction::Cancel, .. }
        );
        (cancel || self.checkpoint()).then_some(reply)
    }

    fn finish(&self, end: JobState) {
        let mut st = self.q.lock();
        let job = st.job(self.id).expect("job del worker");
        job.view.current = None;
        job.view.prompt = None;
        self.q.set_state(job, end);
    }
}

#[tauri::command]
pub async fn enqueue_job(q: tauri::State<'_, Queue>, spec: JobSpec) -> Result<JobId, AppError> {
    Ok(q.enqueue(spec))
}

#[tauri::command]
pub async fn pause_job(q: tauri::State<'_, Queue>, id: JobId) -> Result<(), AppError> {
    q.pause(id)
}

#[tauri::command]
pub async fn resume_job(q: tauri::State<'_, Queue>, id: JobId) -> Result<(), AppError> {
    q.resume(id)
}

#[tauri::command]
pub async fn cancel_job(q: tauri::State<'_, Queue>, id: JobId) -> Result<(), AppError> {
    q.cancel(id)
}

#[tauri::command]
pub async fn move_job(q: tauri::State<'_, Queue>, id: JobId, index: usize) -> Result<(), AppError> {
    q.move_to(id, index)
}

#[tauri::command]
pub async fn reply_prompt(q: tauri::State<'_, Queue>, prompt_id: u64, reply: PromptReply) -> Result<(), AppError> {
    q.reply(prompt_id, reply)
}

#[tauri::command]
pub async fn list_jobs(q: tauri::State<'_, Queue>) -> Result<Vec<JobView>, AppError> {
    Ok(q.list())
}

#[tauri::command]
pub async fn clear_finished(q: tauri::State<'_, Queue>) -> Result<(), AppError> {
    q.clear_finished();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::JobState::*;
    use super::*;
    use crate::ops::conflict::next_free_name;
    use crate::ops::exec::LocalExecutor;
    use crate::ops::model::ConflictPolicy;
    use crate::vfs::loc::Loc;
    use crate::vfs::mem::{self, MemProvider};
    use crate::vfs::provider::Provider;
    use crate::vfs::router::Vfs;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    const T: Duration = Duration::from_secs(10);

    fn view(id: JobId, v: &[&str], state: JobState) -> JobView {
        JobView {
            id,
            kind: JobKind::Copy,
            state,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            current: None,
            speed_bps: 0,
            volumes: v.iter().map(|s| s.to_string()).collect(),
            dest: None,
            errors: vec![],
            prompt: None,
        }
    }

    #[test]
    fn scheduler_serializes_per_volume_and_parallelizes_across() {
        let j = |id, v: &[&str], st| view(id, v, st);
        let jobs = [j(1, &["C:", "D:"], Running), j(2, &["D:"], Queued), j(3, &["E:"], Queued)];
        let busy: HashSet<_> = ["C:".into(), "D:".into()].into();
        assert_eq!(next_runnable(&jobs, &busy), Some(3));
    }

    #[test]
    fn scheduler_respects_queue_order() {
        let jobs = [view(1, &["C:"], Done), view(2, &["D:"], Paused), view(3, &["E:"], Queued), view(4, &["F:"], Queued)];
        assert_eq!(next_runnable(&jobs, &HashSet::new()), Some(3));
        assert_eq!(next_runnable(&jobs[..2], &HashSet::new()), None);
    }

    /// Delega a `LocalExecutor`, con guasti e rallentamenti iniettati.
    #[derive(Default)]
    struct Fake {
        /// Il copy_file numero n (da 1) fallisce con `IO`.
        fail_copy_at: Option<usize>,
        /// Ogni copy_file segnala l'avvio e poi chiama il callback per ~200 ms prima di copiare.
        slow: Option<mpsc::Sender<()>>,
        /// move_path dà sempre `NOT_SAME_DEVICE` (spostamento tra volumi).
        cross_device: bool,
        /// Il Cestino rifiuta tutto con `NO_RECYCLE_BIN`.
        no_recycle: bool,
        /// copy_file va in panico.
        panic_copy: bool,
        /// Ogni copy_file dà `ACCESS_DENIED` (serve l'helper elevato).
        deny_copy: bool,
        /// Il Cestino "accetta" tutto: l'elemento si rimuove dal disco (mai il Cestino vero nei test).
        trash: bool,
        copies: AtomicUsize,
        /// Il parametro `flush` di ogni copy_file.
        flushes: Mutex<Vec<bool>>,
        /// La destinazione di ogni copy_file riuscito fin qui.
        copy_dsts: Mutex<Vec<PathBuf>>,
        /// Ogni delete: percorso e `permanent`.
        deletes: Mutex<Vec<(PathBuf, bool)>>,
    }

    impl Executor for Fake {
        fn copy_file(
            &self,
            src: &Path,
            dst: &Path,
            overwrite: bool,
            flush: bool,
            progress: &mut dyn FnMut(u64) -> bool,
        ) -> Result<(), AppError> {
            assert!(!self.panic_copy, "panico di prova");
            if self.deny_copy {
                return Err(AppError::new(ErrorCode::AccessDenied, Some(dst)));
            }
            self.flushes.lock().unwrap().push(flush);
            self.copy_dsts.lock().unwrap().push(dst.to_path_buf());
            let n = self.copies.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_copy_at == Some(n) {
                return Err(AppError::new(ErrorCode::Io, Some(dst)));
            }
            if let Some(tx) = &self.slow {
                let _ = tx.send(());
                for _ in 0..40 {
                    if !progress(0) {
                        return Err(AppError::new(ErrorCode::Cancelled, Some(src)));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            LocalExecutor.copy_file(src, dst, overwrite, flush, progress)
        }
        fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError> {
            if self.cross_device {
                return Err(AppError::new(ErrorCode::NotSameDevice, Some(src)));
            }
            LocalExecutor.move_path(src, dst, overwrite)
        }
        fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError> {
            self.deletes.lock().unwrap().push((path.to_path_buf(), permanent));
            if self.no_recycle && !permanent {
                return Err(AppError::new(ErrorCode::NoRecycleBin, Some(path)));
            }
            if self.trash && !permanent {
                let r = if is_dir { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
                return r.map_err(|e| AppError::from_io(&e, path));
            }
            assert!(permanent, "Cestino vero nei test");
            LocalExecutor.delete(path, is_dir, permanent)
        }
        fn mkdir(&self, path: &Path) -> Result<(), AppError> {
            LocalExecutor.mkdir(path)
        }
        fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError> {
            LocalExecutor.rename(src, dst)
        }
        fn list_dir(&self, path: &Path) -> Result<Vec<crate::ops::model::ListEntry>, AppError> {
            LocalExecutor.list_dir(path)
        }
    }

    fn queue(exec: impl Executor + 'static) -> (Queue, mpsc::Receiver<Event>) {
        queue_shared(Arc::new(exec))
    }

    fn queue_shared(exec: Arc<dyn Executor>) -> (Queue, mpsc::Receiver<Event>) {
        queue_elevated(exec, Arc::new(|| panic!("helper elevato non previsto")))
    }

    fn queue_elevated(exec: Arc<dyn Executor>, elevate: Elevate) -> (Queue, mpsc::Receiver<Event>) {
        queue_with(exec, elevate, mem::vfs(&[]))
    }

    fn queue_with(exec: Arc<dyn Executor>, elevate: Elevate, vfs: Vfs) -> (Queue, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let sink: Sink = Arc::new(move |e| {
            let _ = tx.send(e);
        });
        (Queue::new(exec, elevate, Arc::new(vfs), sink), rx)
    }

    /// Coda con le connessioni in memoria date; il locale passa per `LocalExecutor`.
    fn queue_remote(conns: &[(&str, &Arc<MemProvider>)]) -> (Queue, mpsc::Receiver<Event>) {
        queue_with(Arc::new(LocalExecutor), Arc::new(|| panic!("helper elevato non previsto")), mem::vfs(conns))
    }

    fn spec(kind: JobKind, sources: Vec<PathBuf>, dest: Option<&Path>, policy: ConflictPolicy) -> JobSpec {
        let sources = sources.into_iter().map(Loc::Local).collect();
        JobSpec { kind, sources, dest: dest.map(|d| Loc::Local(d.to_path_buf())), filter: None, policy }
    }

    fn rspec(kind: JobKind, sources: &[&str], dest: Option<Loc>, policy: ConflictPolicy) -> JobSpec {
        let sources = sources.iter().map(|s| Loc::parse(s).unwrap()).collect();
        JobSpec { kind, sources, dest, filter: None, policy }
    }

    fn loc(s: &str) -> Loc {
        Loc::parse(s).unwrap()
    }

    fn next_prompt(rx: &mpsc::Receiver<Event>) -> Prompt {
        loop {
            if let Event::Prompt(p) = rx.recv_timeout(T).expect("prompt atteso") {
                return p;
            }
        }
    }

    /// Attende lo stato finale del job; un prompt nel frattempo fa fallire il test.
    fn finish(rx: &mpsc::Receiver<Event>, id: JobId) -> JobView {
        loop {
            match rx.recv_timeout(T).expect("fine del job attesa") {
                Event::Prompt(p) => panic!("prompt inatteso: {p:?}"),
                Event::Updated(v) if v.id == id && v.state.is_finished() => return v,
                Event::Updated(_) => {}
            }
        }
    }

    fn reply_conflict(q: &Queue, p: &Prompt, decision: Decision, apply_to_all: bool) {
        q.reply(p.prompt_id(), PromptReply::Conflict { decision, apply_to_all }).unwrap();
    }

    fn reply_error(q: &Queue, p: &Prompt, action: ErrorAction, apply_to_all: bool) {
        q.reply(p.prompt_id(), PromptReply::Error { action, apply_to_all }).unwrap();
    }

    fn exists(p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok()
    }

    /// a/{x.txt, s/y.txt}
    fn tree(root: &Path) -> PathBuf {
        let a = root.join("a");
        std::fs::create_dir_all(a.join("s")).unwrap();
        std::fs::write(a.join("x.txt"), b"x").unwrap();
        std::fs::write(a.join("s").join("y.txt"), b"y").unwrap();
        a
    }

    #[test]
    fn view_carries_dest_for_copy_and_move_only() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a.txt"), b"x").unwrap();
        let (q, rx) = queue(LocalExecutor);
        let copy = q.enqueue(spec(JobKind::Copy, vec![src.join("a.txt")], Some(&dst), ConflictPolicy::Ask));
        assert_eq!(finish(&rx, copy).dest.as_deref(), dst.to_str());
        let mk = q.enqueue(spec(JobKind::Mkdir, vec![d.path().join("nuova")], None, ConflictPolicy::Ask));
        assert_eq!(finish(&rx, mk).dest, None);
        assert_eq!(q.list().iter().map(|j| j.dest.clone()).collect::<Vec<_>>(), [dst.to_str().map(String::from), None]);
    }

    #[test]
    fn conflict_prompt_roundtrip_with_apply_to_all() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        for n in ["a.txt", "b.txt"] {
            std::fs::write(src.join(n), b"nuovo").unwrap();
            std::fs::write(dst.join(n), b"vecchio").unwrap();
        }
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, vec![src.join("a.txt"), src.join("b.txt")], Some(&dst), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Conflict { job_id, src: ps, dst: pd, src_meta, dst_meta, .. } = &p else { panic!("{p:?}") };
        assert_eq!((*job_id, ps.as_str(), pd.as_str()), (id, src.join("a.txt").to_str().unwrap(), dst.join("a.txt").to_str().unwrap()));
        assert_eq!((src_meta.size, dst_meta.size), (5, 7));
        let Prompt::Conflict { same_file, suggested_name, .. } = &p else { unreachable!() };
        assert!(!same_file);
        assert_eq!(suggested_name.as_str(), dst.join("a (2).txt").to_str().unwrap());
        // Il prompt si recupera anche da `list` (evento perso, webview ricaricata).
        let listed = &q.list()[0];
        assert_eq!((listed.state, listed.prompt.as_ref()), (Waiting, Some(&p)));
        reply_conflict(&q, &p, Decision::Skip, true);
        assert_eq!(q.list()[0].prompt, None);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.files_done, v.files_total, v.prompt), (Done, 2, 2, None));
        assert!(v.errors.is_empty());
        for n in ["a.txt", "b.txt"] {
            assert_eq!(std::fs::read(dst.join(n)).unwrap(), b"vecchio");
        }
    }

    #[test]
    fn move_across_volumes_deletes_source_only_after_successful_copy() {
        // a/{1.txt, 2.txt, s/3.txt}: il 2° copy_file fallisce.
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::create_dir_all(a.join("s")).unwrap();
        for f in ["1.txt", "2.txt", "s/3.txt"] {
            std::fs::write(a.join(f), f.as_bytes()).unwrap();
        }
        let dest = d.path().join("D");
        std::fs::create_dir(&dest).unwrap();
        let fake = Arc::new(Fake { fail_copy_at: Some(2), cross_device: true, ..Default::default() });
        let (q, rx) = queue_shared(fake.clone());
        let id = q.enqueue(spec(JobKind::Move, vec![a.clone()], Some(&dest), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Error { error, can_elevate, can_delete_permanently, .. } = &p else { panic!("{p:?}") };
        assert_eq!((error.code, *can_elevate, *can_delete_permanently), (ErrorCode::Io, false, false));
        assert!(!exists(&a.join("1.txt")) && exists(&dest.join("a").join("1.txt")));
        assert!(exists(&a.join("2.txt")));
        reply_error(&q, &p, ErrorAction::Skip, false);
        let v = finish(&rx, id);
        assert_eq!(v.state, Done);
        assert_eq!(v.errors.iter().map(|e| e.code).collect::<Vec<_>>(), vec![ErrorCode::Io]);
        assert_eq!(std::fs::read(a.join("2.txt")).unwrap(), b"2.txt");
        assert!(!exists(&dest.join("a").join("2.txt")));
        assert_eq!(std::fs::read(dest.join("a").join("s").join("3.txt")).unwrap(), b"s/3.txt");
        // Cartelle sorgente: `s` vuota rimossa (figli prima dei genitori), `a` non vuota conservata.
        assert!(!exists(&a.join("s")));
        assert!(exists(&a));
        assert_eq!(v.files_done, v.files_total);
        // Ogni copia di uno spostamento è scritta su disco prima di eliminare il sorgente.
        assert_eq!(*fake.flushes.lock().unwrap(), vec![true; 3]);
    }

    #[test]
    fn cancel_stops_after_current_file() {
        let d = tempfile::tempdir().unwrap();
        let srcs: Vec<PathBuf> = (1..=3).map(|i| d.path().join(format!("f{i}.txt"))).collect();
        for s in &srcs {
            std::fs::write(s, b"x").unwrap();
        }
        let dest = d.path().join("D");
        std::fs::create_dir(&dest).unwrap();
        let (tx, started) = mpsc::channel();
        let (q, rx) = queue(Fake { slow: Some(tx), ..Default::default() });
        let id = q.enqueue(spec(JobKind::Copy, srcs, Some(&dest), ConflictPolicy::Ask));
        started.recv_timeout(T).unwrap();
        q.cancel(id).unwrap();
        let v = finish(&rx, id);
        assert_eq!(v.state, Cancelled);
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0, "nessun file copiato, nessun .mcpart");
        assert!(started.try_recv().is_err(), "il 2° file non è stato iniziato");
    }

    #[test]
    fn directory_rename_rewrites_children_destinations() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let (q, rx) = queue(LocalExecutor);
        // Copia nella propria cartella madre: la cartella è in conflitto con se stessa.
        let id = q.enqueue(spec(JobKind::Copy, vec![a.clone()], Some(d.path()), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Conflict { src, same_file, suggested_name, .. } = &p else { panic!("{p:?}") };
        assert_eq!(src.as_str(), a.to_str().unwrap());
        assert!(same_file);
        let new = next_free_name(&a);
        assert_eq!(suggested_name.as_str(), new.to_str().unwrap());
        // Un nome nuovo relativo, in un'altra cartella o uguale a dst è rifiutato; il prompt resta.
        for bad in [PathBuf::from("a (2)"), d.path().join("altro").join("a (2)"), a.clone()] {
            let r = q.reply(p.prompt_id(), PromptReply::Conflict { decision: Decision::Rename(Loc::Local(bad.clone())), apply_to_all: false });
            assert_eq!(r.unwrap_err().code, ErrorCode::InvalidRequest, "{bad:?}");
        }
        let r = q.reply(p.prompt_id(), PromptReply::ConflictPolicy { policy: ConflictPolicy::Ask, apply_to_all: false });
        assert_eq!(r.unwrap_err().code, ErrorCode::InvalidRequest);
        assert_eq!(q.list()[0].prompt.as_ref(), Some(&p));
        reply_conflict(&q, &p, Decision::Rename(Loc::Local(new.clone())), false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.files_done, v.files_total), (Done, 4, 4));
        assert_eq!(std::fs::read(new.join("x.txt")).unwrap(), b"x");
        assert_eq!(std::fs::read(new.join("s").join("y.txt")).unwrap(), b"y");
        assert_eq!(std::fs::read(a.join("s").join("y.txt")).unwrap(), b"y");
    }

    #[test]
    fn directory_skip_skips_descendants() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, vec![a.clone()], Some(d.path()), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        reply_conflict(&q, &p, Decision::Skip, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.files_done, v.files_total), (Done, 4, 4));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn user_overwrite_of_same_file_becomes_same_file_error() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"dati").unwrap();
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, vec![f.clone()], Some(d.path()), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        assert!(matches!(p, Prompt::Conflict { .. }), "{p:?}");
        reply_conflict(&q, &p, Decision::Overwrite, false);
        let p = next_prompt(&rx);
        let Prompt::Error { error, .. } = &p else { panic!("{p:?}") };
        assert_eq!(error.code, ErrorCode::SameFile);
        reply_error(&q, &p, ErrorAction::Skip, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 1));
        assert_eq!(std::fs::read(&f).unwrap(), b"dati");
    }

    #[test]
    fn cancel_while_waiting_ends_cancelled() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, vec![f], Some(d.path()), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        q.cancel(id).unwrap();
        assert_eq!(finish(&rx, id).state, Cancelled);
        // Il prompt non è più in attesa.
        let late = q.reply(p.prompt_id(), PromptReply::Conflict { decision: Decision::Skip, apply_to_all: false });
        assert_eq!(late.unwrap_err().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn pause_blocks_the_copy_and_cancel_while_paused_works() {
        let d = tempfile::tempdir().unwrap();
        let (f, dest) = (d.path().join("f.txt"), d.path().join("D"));
        std::fs::write(&f, b"x").unwrap();
        std::fs::create_dir(&dest).unwrap();
        let (tx, started) = mpsc::channel();
        let (q, rx) = queue(Fake { slow: Some(tx), ..Default::default() });
        let id = q.enqueue(spec(JobKind::Copy, vec![f.clone()], Some(&dest), ConflictPolicy::Ask));
        started.recv_timeout(T).unwrap();
        q.pause(id).unwrap();
        std::thread::sleep(Duration::from_millis(400)); // più della durata della copia finta
        assert_eq!(q.list()[0].state, Paused);
        assert!(!exists(&dest.join("f.txt")));
        q.resume(id).unwrap();
        assert_eq!(finish(&rx, id).state, Done);
        assert!(exists(&dest.join("f.txt")));

        let id = q.enqueue(spec(JobKind::Copy, vec![f], Some(&d.path().join("E")), ConflictPolicy::Ask));
        started.recv_timeout(T).unwrap();
        q.pause(id).unwrap();
        q.cancel(id).unwrap();
        assert_eq!(finish(&rx, id).state, Cancelled);
    }

    #[test]
    fn recycle_bin_refusal_offers_permanent_delete_for_all() {
        let d = tempfile::tempdir().unwrap();
        let files: Vec<PathBuf> = (1..=2).map(|i| d.path().join(format!("f{i}"))).collect();
        for f in &files {
            std::fs::write(f, b"x").unwrap();
        }
        let (q, rx) = queue(Fake { no_recycle: true, ..Default::default() });
        let id = q.enqueue(spec(JobKind::Delete { permanent: false }, files.clone(), None, ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Error { error, can_delete_permanently, .. } = &p else { panic!("{p:?}") };
        assert_eq!((error.code, *can_delete_permanently), (ErrorCode::NoRecycleBin, true));
        reply_error(&q, &p, ErrorAction::DeletePermanent, true);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 0));
        assert!(files.iter().all(|f| !exists(f)));
    }

    /// src/{a.txt (più grande), b.txt (più piccolo)} contro dst/{a.txt, b.txt} = "vecchio", più vecchi.
    fn policy_case() -> (tempfile::TempDir, Vec<PathBuf>, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a.txt"), b"nuovo e grande").unwrap();
        std::fs::write(src.join("b.txt"), b"n").unwrap();
        let old = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for n in ["a.txt", "b.txt"] {
            std::fs::write(dst.join(n), b"vecchio").unwrap();
            std::fs::File::options().write(true).open(dst.join(n)).unwrap().set_modified(old).unwrap();
        }
        (d, vec![src.join("a.txt"), src.join("b.txt")], dst)
    }

    #[test]
    fn conflict_policy_reply_applies_to_all() {
        let (_d, srcs, dst) = policy_case();
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, srcs, Some(&dst), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        q.reply(p.prompt_id(), PromptReply::ConflictPolicy { policy: ConflictPolicy::OverwriteIfLarger, apply_to_all: true }).unwrap();
        assert_eq!(finish(&rx, id).state, Done); // nessun secondo prompt
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"nuovo e grande");
        assert_eq!(std::fs::read(dst.join("b.txt")).unwrap(), b"vecchio");
    }

    #[test]
    fn conflict_policy_reply_for_current_only() {
        let (_d, srcs, dst) = policy_case();
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, srcs, Some(&dst), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        q.reply(p.prompt_id(), PromptReply::ConflictPolicy { policy: ConflictPolicy::OverwriteIfNewer, apply_to_all: false }).unwrap();
        let p = next_prompt(&rx); // b.txt chiede di nuovo
        let Prompt::Conflict { dst: pd, .. } = &p else { panic!("{p:?}") };
        assert_eq!(pd.as_str(), dst.join("b.txt").to_str().unwrap());
        reply_conflict(&q, &p, Decision::Skip, false);
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"nuovo e grande");
        assert_eq!(std::fs::read(dst.join("b.txt")).unwrap(), b"vecchio");
    }

    #[test]
    fn retry_after_pause_while_waiting_waits_for_resume() {
        let d = tempfile::tempdir().unwrap();
        let (f, dest) = (d.path().join("f.txt"), d.path().join("D"));
        std::fs::write(&f, b"x").unwrap();
        std::fs::create_dir(&dest).unwrap();
        let fake = Arc::new(Fake { fail_copy_at: Some(1), ..Default::default() });
        let (q, rx) = queue_shared(fake.clone());
        let id = q.enqueue(spec(JobKind::Copy, vec![f], Some(&dest), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        q.pause(id).unwrap();
        assert_eq!(q.list()[0].state, Waiting);
        reply_error(&q, &p, ErrorAction::Retry, false);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!((q.list()[0].state, fake.copies.load(Ordering::SeqCst)), (Paused, 1));
        q.resume(id).unwrap();
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(fake.copies.load(Ordering::SeqCst), 2);
        assert!(exists(&dest.join("f.txt")));
    }

    #[test]
    fn copy_creates_missing_destination_with_intermediates() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let (q, rx) = queue(LocalExecutor);
        // Con `\` finale la destinazione di un solo file è una cartella (senza, sarebbe il nome del file).
        let dest = d.path().join("X").join("Y\\");
        let id = q.enqueue(spec(JobKind::Copy, vec![f.clone()], Some(&dest), ConflictPolicy::Ask));
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(std::fs::read(d.path().join("X").join("Y").join("f.txt")).unwrap(), b"x");

        // Una destinazione che non si può creare passa dal prompt d'errore; "Salta" chiude il job.
        let g = d.path().join("g.txt");
        std::fs::write(&g, b"g").unwrap();
        let bad = g.join("sub\\"); // un file al posto della cartella intermedia
        let id = q.enqueue(spec(JobKind::Copy, vec![f.clone()], Some(&bad), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        assert!(matches!(p, Prompt::Error { .. }), "{p:?}");
        reply_error(&q, &p, ErrorAction::Skip, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len(), v.files_total), (Done, 1, 0));
    }

    #[test]
    fn failed_expand_does_not_create_the_destination() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let (q, rx) = queue(LocalExecutor);
        let into = a.join("nuova");
        let id = q.enqueue(spec(JobKind::Move, vec![a.clone()], Some(&into), ConflictPolicy::Ask));
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors[0].code), (Failed, ErrorCode::IntoItself));
        assert!(!exists(&into), "il job fallito non deve toccare la sorgente");
    }

    #[test]
    fn cancel_reply_does_not_wait_for_resume() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let (q, rx) = queue(Fake { fail_copy_at: Some(1), ..Default::default() });
        // Conflitto (stesso file) e poi errore di copia: Annulla da un job in pausa termina subito.
        let id = q.enqueue(spec(JobKind::Copy, vec![f.clone()], Some(d.path()), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        q.pause(id).unwrap();
        reply_conflict(&q, &p, Decision::Cancel, false);
        assert_eq!(finish(&rx, id).state, Cancelled);

        let id = q.enqueue(spec(JobKind::Copy, vec![f], Some(&d.path().join("D")), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        assert!(matches!(p, Prompt::Error { .. }), "{p:?}");
        q.pause(id).unwrap();
        reply_error(&q, &p, ErrorAction::Cancel, false);
        assert_eq!(finish(&rx, id).state, Cancelled);
    }

    #[test]
    fn conflict_policy_for_all_survives_same_file_error() {
        let (_d, srcs, dst) = policy_case();
        // Prima un file su se stesso (dst/a.txt), poi src/a.txt più nuovo contro dst/a.txt.
        let srcs = vec![dst.join("b.txt"), srcs[0].clone()];
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, srcs, Some(&dst), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Conflict { same_file, .. } = &p else { panic!("{p:?}") };
        assert!(same_file);
        q.reply(p.prompt_id(), PromptReply::ConflictPolicy { policy: ConflictPolicy::OverwriteIfNewer, apply_to_all: true }).unwrap();
        let p = next_prompt(&rx);
        let Prompt::Error { error, .. } = &p else { panic!("{p:?}") };
        assert_eq!(error.code, ErrorCode::SameFile);
        reply_error(&q, &p, ErrorAction::Skip, false);
        // La politica "per tutti" vale per il conflitto successivo: nessun nuovo prompt.
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"nuovo e grande");
    }

    #[test]
    fn worker_panic_fails_the_job_and_frees_its_volume() {
        // Il panico di prova non deve sporcare l'output dei test.
        static QUIET: std::sync::Once = std::sync::Once::new();
        QUIET.call_once(|| {
            let default = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if !info.to_string().contains("panico di prova") {
                    default(info);
                }
            }));
        });
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let (q, rx) = queue(Fake { panic_copy: true, ..Default::default() });
        let id = q.enqueue(spec(JobKind::Copy, vec![f], Some(&d.path().join("D")), ConflictPolicy::Ask));
        let v = finish(&rx, id);
        assert_eq!(v.state, Failed);
        assert_eq!((v.errors[0].code, v.errors[0].detail.as_deref()), (ErrorCode::Io, Some("internal_panic")));
        // Un job successivo sullo stesso volume parte.
        let new = d.path().join("nuova");
        let id = q.enqueue(spec(JobKind::Mkdir, vec![new.clone()], None, ConflictPolicy::Ask));
        assert_eq!(finish(&rx, id).state, Done);
        assert!(new.is_dir());
    }

    #[test]
    fn directory_link_in_copy_is_unsupported() {
        let d = tempfile::tempdir().unwrap();
        let target = tree(d.path());
        let p = d.path().join("p");
        std::fs::create_dir(&p).unwrap();
        let link = p.join("link");
        let ok = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&target).output().unwrap();
        assert!(ok.status.success());
        let dest = d.path().join("D");
        std::fs::create_dir(&dest).unwrap();
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, vec![p], Some(&dest), ConflictPolicy::Ask));
        let pr = next_prompt(&rx);
        let Prompt::Error { error, .. } = &pr else { panic!("{pr:?}") };
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!((error.path.as_deref(), error.detail.as_deref()), (link.to_str(), Some("reparse_point")));
        reply_error(&q, &pr, ErrorAction::Skip, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 1));
        assert!(dest.join("p").is_dir());
        assert!(!exists(&dest.join("p").join("link")));
    }

    #[test]
    fn retry_elevated_uses_the_elevated_executor() {
        let d = tempfile::tempdir().unwrap();
        let names = ["1.txt", "2.txt", "3.txt"];
        for n in names {
            std::fs::write(d.path().join(n), n).unwrap();
        }
        let dest = d.path().join("D");
        let elevated = Arc::new(Fake::default());
        let (calls, x) = (AtomicUsize::new(0), elevated.clone());
        // Il primo avvio dell'helper fallisce come un UAC rifiutato.
        let elevate: Elevate = Arc::new(move || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(AppError { detail: Some("uac_declined".into()), ..AppError::new(ErrorCode::AccessDenied, None) });
            }
            Ok(x.clone() as Arc<dyn Executor>)
        });
        let (q, rx) = queue_elevated(Arc::new(Fake { deny_copy: true, ..Default::default() }), elevate);
        let sources = names.iter().map(|n| d.path().join(n)).collect();
        let id = q.enqueue(spec(JobKind::Copy, sources, Some(&dest), ConflictPolicy::Ask));
        let error_of = |p: &Prompt| match p {
            Prompt::Error { error, can_elevate, .. } => (error.code, error.detail.clone(), *can_elevate),
            _ => panic!("{p:?}"),
        };
        let p = next_prompt(&rx);
        assert_eq!(error_of(&p), (ErrorCode::AccessDenied, None, true));
        reply_error(&q, &p, ErrorAction::RetryElevated, false);
        // UAC rifiutato: torna il prompt d'errore normale, da cui si può riprovare.
        let p = next_prompt(&rx);
        assert_eq!(error_of(&p), (ErrorCode::AccessDenied, Some("uac_declined".into()), true));
        reply_error(&q, &p, ErrorAction::RetryElevated, false);
        // Solo il file corrente: il secondo chiede di nuovo, ora "per tutti".
        let p = next_prompt(&rx);
        assert_eq!(error_of(&p), (ErrorCode::AccessDenied, None, true));
        reply_error(&q, &p, ErrorAction::RetryElevated, true);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 0));
        assert_eq!(elevated.copies.load(Ordering::SeqCst), 3);
        for n in names {
            assert_eq!(std::fs::read(dest.join(n)).unwrap(), n.as_bytes());
        }
    }

    /// Dopo "Riprova come amministratore" i percorsi sotto le cartelle del job arrivano all'helper
    /// con il prefisso risolto alla prima elevazione: la giunzione sostituita dopo non conta.
    #[test]
    fn elevated_paths_use_job_roots_resolved_once() {
        let d = tempfile::tempdir().unwrap();
        let (real, link, altrove) = (d.path().join("real"), d.path().join("link"), d.path().join("altrove"));
        std::fs::create_dir(&real).unwrap();
        std::fs::create_dir(&altrove).unwrap();
        let mklink = |target: &Path| {
            let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(target).output().unwrap();
            assert!(ok.status.success(), "mklink /J");
        };
        mklink(&real);
        for n in ["1.txt", "2.txt"] {
            std::fs::write(real.join(n), n).unwrap();
        }
        let elevated = Arc::new(Fake::default());
        let x = elevated.clone();
        let elevate: Elevate = Arc::new(move || Ok(x.clone() as Arc<dyn Executor>));
        let (q, rx) = queue_elevated(Arc::new(Fake { deny_copy: true, ..Default::default() }), elevate);
        let sources = vec![link.join("1.txt"), link.join("2.txt")];
        let id = q.enqueue(spec(JobKind::Copy, sources, Some(&link.join("D")), ConflictPolicy::Ask));
        reply_error(&q, &next_prompt(&rx), ErrorAction::RetryElevated, false);
        let p = next_prompt(&rx);
        std::fs::remove_dir(&link).unwrap();
        mklink(&altrove);
        reply_error(&q, &p, ErrorAction::RetryElevated, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 0));
        let real_d = crate::fs::listing::long_path(&real).join("D");
        let got = elevated.copy_dsts.lock().unwrap().clone();
        assert_eq!(got.len(), 2, "{got:?}");
        for (dst, n) in got.iter().zip(["1.txt", "2.txt"]) {
            assert!(same_path(dst, &real_d.join(n)), "{got:?}");
        }
        assert!(!exists(&altrove.join("D")));
    }

    #[test]
    fn recycle_delete_trashes_a_folder_whole() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let fake = Arc::new(Fake { trash: true, ..Default::default() });
        let (q, rx) = queue_shared(fake.clone());
        let id = q.enqueue(spec(JobKind::Delete { permanent: false }, vec![a.clone()], None, ConflictPolicy::Ask));
        let v = finish(&rx, id);
        assert_eq!((v.state, v.files_done, v.files_total, v.errors.len()), (Done, 1, 1, 0));
        assert_eq!(*fake.deletes.lock().unwrap(), vec![(a.clone(), false)]);
        assert!(!exists(&a));
    }

    #[test]
    fn recycle_refusal_of_a_folder_deletes_its_tree_permanently() {
        let d = tempfile::tempdir().unwrap();
        let a = tree(d.path());
        let fake = Arc::new(Fake { no_recycle: true, ..Default::default() });
        let (q, rx) = queue_shared(fake.clone());
        let id = q.enqueue(spec(JobKind::Delete { permanent: false }, vec![a.clone()], None, ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Error { error, can_delete_permanently, .. } = &p else { panic!("{p:?}") };
        assert_eq!((error.code, error.path.as_deref(), *can_delete_permanently), (ErrorCode::NoRecycleBin, a.to_str(), true));
        // Solo per questo elemento: il suo contenuto segue senza altri prompt.
        reply_error(&q, &p, ErrorAction::DeletePermanent, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len(), v.files_done, v.files_total), (Done, 0, 4, 4));
        assert!(!exists(&a));
        let expected = [(a.clone(), false), (a.join("x.txt"), true), (a.join("s").join("y.txt"), true), (a.join("s"), true), (a.clone(), true)];
        assert_eq!(*fake.deletes.lock().unwrap(), expected);
    }

    /// a/{x.txt, s/y.txt (cartella illeggibile), t/z.txt}
    fn tree_with_unreadable(root: &Path) -> (PathBuf, crate::ops::DenyList) {
        let a = tree(root);
        std::fs::create_dir(a.join("t")).unwrap();
        std::fs::write(a.join("t").join("z.txt"), b"z").unwrap();
        let deny = crate::ops::DenyList::new(&a.join("s"));
        (a, deny)
    }

    fn skip_access_denied(q: &Queue, rx: &mpsc::Receiver<Event>, dir: &Path) {
        let p = next_prompt(rx);
        let Prompt::Error { error, can_elevate, .. } = &p else { panic!("{p:?}") };
        assert_eq!((error.code, error.path.as_deref(), *can_elevate), (ErrorCode::AccessDenied, dir.to_str(), true));
        reply_error(q, &p, ErrorAction::Skip, false);
    }

    #[test]
    fn unreadable_subfolder_in_copy_prompts_and_skip_copies_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let (a, _deny) = tree_with_unreadable(d.path());
        let dest = d.path().join("D");
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Copy, vec![a.clone()], Some(&dest), ConflictPolicy::Ask));
        skip_access_denied(&q, &rx, &a.join("s"));
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.iter().map(|e| e.code).collect::<Vec<_>>()), (Done, vec![ErrorCode::AccessDenied]));
        assert_eq!(std::fs::read(dest.join("a").join("x.txt")).unwrap(), b"x");
        assert_eq!(std::fs::read(dest.join("a").join("t").join("z.txt")).unwrap(), b"z");
        assert!(!exists(&dest.join("a").join("s")), "la cartella saltata non si crea");
    }

    #[test]
    fn unreadable_subfolder_in_permanent_delete_prompts_and_skip_deletes_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let (a, _deny) = tree_with_unreadable(d.path());
        let (q, rx) = queue(LocalExecutor);
        let id = q.enqueue(spec(JobKind::Delete { permanent: true }, vec![a.clone()], None, ConflictPolicy::Ask));
        skip_access_denied(&q, &rx, &a.join("s"));
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 1));
        assert!(!exists(&a.join("x.txt")) && !exists(&a.join("t")));
        // La cartella saltata e i suoi antenati restano.
        assert!(exists(&a.join("s")) && exists(&a));
    }

    #[test]
    fn single_file_to_a_missing_name_creates_that_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"dati").unwrap();
        let (q, rx) = queue(LocalExecutor);
        let to = d.path().join("D").join("g.txt");
        let id = q.enqueue(spec(JobKind::Copy, vec![f.clone()], Some(&to), ConflictPolicy::Ask));
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(std::fs::read(&to).unwrap(), b"dati", "g.txt è il file copiato, non una cartella");
        let moved = d.path().join("h.txt");
        let id = q.enqueue(spec(JobKind::Move, vec![f.clone()], Some(&moved), ConflictPolicy::Ask));
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(std::fs::read(&moved).unwrap(), b"dati");
        assert!(!exists(&f));
    }

    #[test]
    fn serde_shapes() {
        let p = Prompt::Error {
            prompt_id: 1,
            job_id: 2,
            error: AppError::new(ErrorCode::AccessDenied, None),
            can_elevate: true,
            can_delete_permanently: false,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({"type": "error", "promptId": 1, "jobId": 2, "error": {"code": "ACCESS_DENIED", "path": null, "detail": null},
                   "canElevate": true, "canDeletePermanently": false})
        );
        let r: PromptReply = serde_json::from_value(json!({"type": "conflict", "decision": {"type": "rename", "path": "x"}, "applyToAll": true})).unwrap();
        assert_eq!(r, PromptReply::Conflict { decision: Decision::Rename(Loc::Local("x".into())), apply_to_all: true });
        let r: PromptReply = serde_json::from_value(json!({"type": "error", "action": {"type": "deletePermanent"}, "applyToAll": false})).unwrap();
        assert_eq!(r, PromptReply::Error { action: ErrorAction::DeletePermanent, apply_to_all: false });
        let r: PromptReply = serde_json::from_value(
            json!({"type": "conflictPolicy", "policy": {"type": "overwriteIfNewer"}, "applyToAll": true}),
        )
        .unwrap();
        assert_eq!(r, PromptReply::ConflictPolicy { policy: ConflictPolicy::OverwriteIfNewer, apply_to_all: true });
        let c = Prompt::Conflict {
            prompt_id: 1,
            job_id: 2,
            src: "s".into(),
            dst: "d".into(),
            src_meta: Meta { size: 1, modified_ms: 2 },
            dst_meta: Meta { size: 3, modified_ms: 4 },
            same_file: true,
            suggested_name: "d (2)".into(),
        };
        let cv = serde_json::to_value(&c).unwrap();
        assert_eq!((&cv["type"], &cv["sameFile"], &cv["suggestedName"]), (&json!("conflict"), &json!(true), &json!("d (2)")));
        assert_eq!(cv["srcMeta"], json!({"size": 1, "modifiedMs": 2}));
        let v = serde_json::to_value(view(3, &["C:"], Waiting)).unwrap();
        assert_eq!(v["state"], json!({"type": "waiting"}));
        assert_eq!(v["filesDone"], json!(0));
        assert_eq!(v["speedBps"], json!(0));
        assert_eq!(v["prompt"], json!(null));
        assert_eq!(serde_json::to_value(ErrorCode::Unsupported).unwrap(), json!("UNSUPPORTED"));
        let u = Prompt::UnsafeWrite { prompt_id: 1, job_id: 2, path: "sftp://m/f".into() };
        assert_eq!(serde_json::to_value(&u).unwrap(), json!({"type": "unsafeWrite", "promptId": 1, "jobId": 2, "path": "sftp://m/f"}));
        let r: PromptReply = serde_json::from_value(json!({"type": "unsafeWrite", "proceed": true, "applyToAll": false})).unwrap();
        assert_eq!(r, PromptReply::UnsafeWrite { proceed: true, apply_to_all: false });
    }

    #[test]
    fn download_invalid_windows_name_is_per_item() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/logs/a:b.log", b"x");
        m.add_file("/logs/ok.log", b"ok");
        let d = tempfile::tempdir().unwrap();
        let (q, rx) = queue_remote(&[("m", &m)]);
        let id = q.enqueue(rspec(JobKind::Copy, &["sftp://m/logs"], Some(Loc::Local(d.path().into())), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Error { error, can_elevate, .. } = &p else { panic!("{p:?}") };
        assert_eq!(error.code, ErrorCode::InvalidName);
        assert!(error.path.as_deref().is_some_and(|p| p.ends_with("a:b.log")), "{error:?}");
        assert!(!can_elevate, "l'elevazione vale solo per il locale");
        reply_error(&q, &p, ErrorAction::Skip, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.iter().map(|e| e.code).collect::<Vec<_>>()), (Done, vec![ErrorCode::InvalidName]));
        let logs = d.path().join("logs");
        assert_eq!(std::fs::read(logs.join("ok.log")).unwrap(), b"ok");
        assert_eq!(std::fs::read_dir(&logs).unwrap().count(), 1);
    }

    #[test]
    fn remote_dir_links_are_skipped() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/app/x.log", b"x");
        m.add_dir_link("/app/current");
        let d = tempfile::tempdir().unwrap();
        let (q, rx) = queue_remote(&[("m", &m)]);
        let id = q.enqueue(rspec(JobKind::Copy, &["sftp://m/app"], Some(Loc::Local(d.path().into())), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Error { error, .. } = &p else { panic!("{p:?}") };
        let got = (error.code, error.path.as_deref(), error.detail.as_deref());
        assert_eq!(got, (ErrorCode::Unsupported, Some("sftp://m/app/current"), Some("reparse_point")));
        reply_error(&q, &p, ErrorAction::Skip, false);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.len()), (Done, 1));
        assert_eq!(std::fs::read(d.path().join("app").join("x.log")).unwrap(), b"x");
        assert!(!exists(&d.path().join("app").join("current")));
    }

    #[test]
    fn jobs_on_same_connection_run_one_at_a_time() {
        let (m, n) = (Arc::new(MemProvider::new()), Arc::new(MemProvider::new()));
        m.add_file("/a.txt", b"nuovo");
        m.add_file("/d/a.txt", b"vecchio");
        let (q, rx) = queue_remote(&[("m", &m), ("n", &n)]);
        // Il conflitto tiene il primo job in attesa, con la connessione occupata.
        let first = q.enqueue(rspec(JobKind::Copy, &["sftp://m/a.txt"], Some(loc("sftp://m/d")), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        assert!(matches!(p, Prompt::Conflict { .. }), "{p:?}");
        let second = q.enqueue(rspec(JobKind::Mkdir, &["sftp://m/x"], None, ConflictPolicy::Ask));
        let third = q.enqueue(rspec(JobKind::Mkdir, &["sftp://n/y"], None, ConflictPolicy::Ask));
        assert_eq!(finish(&rx, third).state, Done);
        let view = |id| q.list().into_iter().find(|j| j.id == id).unwrap();
        assert_eq!(view(second).state, Queued);
        assert_eq!(view(first).volumes, ["sftp://m"]);
        reply_conflict(&q, &p, Decision::Overwrite, false);
        assert_eq!(finish(&rx, first).state, Done);
        assert_eq!(finish(&rx, second).state, Done);
        assert_eq!(m.data("/d/a.txt").unwrap(), b"nuovo");
        assert!(m.stat("/x").unwrap().is_dir && n.stat("/y").unwrap().is_dir);
    }

    #[test]
    fn unsafe_write_is_asked_once_per_job_with_apply_to_all() {
        let mut m = MemProvider::new();
        m.atomic_replace = false;
        let m = Arc::new(m);
        for f in ["1", "2"] {
            m.add_file(&format!("/s/{f}"), b"nuovo");
            m.add_file(&format!("/d/{f}"), b"vecchio");
        }
        let (q, rx) = queue_remote(&[("m", &m)]);
        let sources = ["sftp://m/s/1", "sftp://m/s/2"];
        let job = |q: &Queue| q.enqueue(rspec(JobKind::Copy, &sources, Some(loc("sftp://m/d")), ConflictPolicy::Overwrite));
        let id = job(&q);
        let p = next_prompt(&rx);
        let Prompt::UnsafeWrite { job_id, path, .. } = &p else { panic!("{p:?}") };
        assert_eq!((*job_id, path.as_str()), (id, "sftp://m/d/1"));
        q.reply(p.prompt_id(), PromptReply::UnsafeWrite { proceed: true, apply_to_all: true }).unwrap();
        let v = finish(&rx, id); // nessun secondo prompt
        assert_eq!((v.state, v.errors.len()), (Done, 0));
        assert_eq!((m.data("/d/1").unwrap(), m.data("/d/2").unwrap()), (b"nuovo".to_vec(), b"nuovo".to_vec()));

        // "Annulla" lascia la destinazione com'era e la segna tra gli errori del job.
        m.add_file("/d/1", b"vecchio");
        m.add_file("/d/2", b"vecchio");
        let id = job(&q);
        let p = next_prompt(&rx);
        assert!(q.reply(p.prompt_id(), PromptReply::Error { action: ErrorAction::Skip, apply_to_all: false }).is_err());
        q.reply(p.prompt_id(), PromptReply::UnsafeWrite { proceed: false, apply_to_all: true }).unwrap();
        let v = finish(&rx, id);
        assert_eq!((v.state, v.errors.iter().map(|e| e.code).collect::<Vec<_>>()), (Done, vec![ErrorCode::UnsafeWrite; 2]));
        assert_eq!((m.data("/d/1").unwrap(), m.data("/d/2").unwrap()), (b"vecchio".to_vec(), b"vecchio".to_vec()));
    }

    #[test]
    fn move_between_providers_deletes_the_source_only_after_the_transfer() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/giu.txt", b"scaricato");
        let mut n = MemProvider::new();
        n.fail_read_after = Some(10);
        let n = Arc::new(n);
        n.add_file("/rotto.txt", &[7u8; 100]);
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("D")).unwrap();
        let up = d.path().join("su.txt");
        std::fs::write(&up, b"caricato").unwrap();
        let (q, rx) = queue_remote(&[("m", &m), ("n", &n)]);
        let local = || Some(Loc::Local(d.path().join("D")));
        // Download: la copia locale va su disco, poi si elimina la sorgente remota.
        let id = q.enqueue(rspec(JobKind::Move, &["sftp://m/giu.txt"], local(), ConflictPolicy::Ask));
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(std::fs::read(d.path().join("D").join("giu.txt")).unwrap(), b"scaricato");
        assert_eq!(m.data("/giu.txt"), None);
        // Upload.
        m.add_dir("/su");
        let upload = JobSpec { dest: Some(loc("sftp://m/su")), ..spec(JobKind::Move, vec![up.clone()], None, ConflictPolicy::Ask) };
        let id = q.enqueue(upload);
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!(m.data("/su/su.txt").unwrap(), b"caricato");
        assert!(!exists(&up));
        // Connessione caduta a metà: la sorgente resta e la destinazione non c'è.
        let id = q.enqueue(rspec(JobKind::Move, &["sftp://n/rotto.txt"], local(), ConflictPolicy::Ask));
        let p = next_prompt(&rx);
        let Prompt::Error { error, .. } = &p else { panic!("{p:?}") };
        assert_eq!(error.code, ErrorCode::ConnectionFailed);
        reply_error(&q, &p, ErrorAction::Skip, false);
        assert_eq!(finish(&rx, id).state, Done);
        assert!(n.data("/rotto.txt").is_some());
        let names: Vec<String> =
            std::fs::read_dir(d.path().join("D")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(names, ["giu.txt"]);
    }

    #[test]
    fn same_connection_move_is_a_rename() {
        let m = Arc::new(MemProvider::new());
        m.add_file("/a/x", b"x");
        m.add_file("/a/s/y", b"y");
        m.add_dir("/b");
        let (q, rx) = queue_remote(&[("m", &m)]);
        let id = q.enqueue(rspec(JobKind::Move, &["sftp://m/a"], Some(loc("sftp://m/b")), ConflictPolicy::Ask));
        let v = finish(&rx, id);
        // Un solo elemento: la cartella si rinomina in blocco, senza elencarla.
        assert_eq!((v.state, v.files_total), (Done, 1));
        assert_eq!((m.data("/b/a/x").unwrap(), m.data("/b/a/s/y").unwrap()), (b"x".to_vec(), b"y".to_vec()));
        assert!(m.stat("/a").is_err());
    }

    #[test]
    fn same_connection_move_over_a_file_without_atomic_rename_asks() {
        let mut m = MemProvider::new();
        m.atomic_replace = false;
        let m = Arc::new(m);
        m.add_file("/s/f", b"nuovo");
        m.add_file("/d/f", b"vecchio");
        let (q, rx) = queue_remote(&[("m", &m)]);
        let id = q.enqueue(rspec(JobKind::Move, &["sftp://m/s/f"], Some(loc("sftp://m/d")), ConflictPolicy::Overwrite));
        let p = next_prompt(&rx);
        assert!(matches!(&p, Prompt::UnsafeWrite { path, .. } if path == "sftp://m/d/f"), "{p:?}");
        q.reply(p.prompt_id(), PromptReply::UnsafeWrite { proceed: true, apply_to_all: false }).unwrap();
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!((m.data("/d/f").unwrap(), m.data("/s/f")), (b"nuovo".to_vec(), None));
    }

    #[test]
    fn newer_policy_compares_imprecise_dates_at_minute_precision() {
        let mut m = MemProvider::new();
        m.precise_mtime = false;
        let m = Arc::new(m);
        let t = 1_700_000_040_000; // multiplo di 60 s
        // a: stesso minuto della destinazione (resta), b: un minuto dopo (si sovrascrive).
        for (f, src_ms) in [("a", t + 30_000), ("b", t + 70_000)] {
            m.add_file(&format!("/s/{f}"), b"nuovo");
            m.add_file(&format!("/d/{f}"), b"vecchio");
            m.set_meta(&format!("/s/{f}"), Some(src_ms), None).unwrap();
            m.set_meta(&format!("/d/{f}"), Some(t + 10_000), None).unwrap();
        }
        let (q, rx) = queue_remote(&[("m", &m)]);
        let sources = ["sftp://m/s/a", "sftp://m/s/b"];
        let id = q.enqueue(rspec(JobKind::Copy, &sources, Some(loc("sftp://m/d")), ConflictPolicy::OverwriteIfNewer));
        assert_eq!(finish(&rx, id).state, Done);
        assert_eq!((m.data("/d/a").unwrap(), m.data("/d/b").unwrap()), (b"vecchio".to_vec(), b"nuovo".to_vec()));
    }
}
