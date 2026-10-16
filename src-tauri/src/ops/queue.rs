//! Coda delle operazioni. Un thread scheduler avvia i job nell'ordine della coda, al massimo un
//! job alla volta per volume; ogni job avviato gira in un proprio thread worker (`runner`).
//!
//! Stati finali: `Done` anche se alcuni elementi sono stati saltati dopo un errore (gli errori
//! restano in `JobView.errors`); `Cancelled` se annullato dall'utente; `Failed` solo per un errore
//! fuori dai prompt per elemento (per esempio `expand` che fallisce).
//! I job finiti restano in elenco finché il frontend non chiama `clear_finished`.

use super::exec::Executor;
use super::model::{Decision, JobKind, JobSpec, Meta};
use super::runner;
use crate::error::{AppError, ErrorCode};
use crate::paths::{normalize_lexical, volume_key};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
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
    pub volumes: Vec<String>,
    /// Riepilogo finale: un errore per ogni elemento saltato dopo un errore.
    pub errors: Vec<AppError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Prompt {
    Conflict { prompt_id: u64, job_id: JobId, src: String, dst: String, src_meta: Meta, dst_meta: Meta },
    /// `can_delete_permanently`: il Cestino ha rifiutato l'elemento (`NO_RECYCLE_BIN`, oppure
    /// `INVALID_NAME` con detail `recycle_bin_unsupported_name`).
    Error { prompt_id: u64, job_id: JobId, error: AppError, can_elevate: bool, can_delete_permanently: bool },
}

impl Prompt {
    pub fn prompt_id(&self) -> u64 {
        match self {
            Prompt::Conflict { prompt_id, .. } | Prompt::Error { prompt_id, .. } => *prompt_id,
        }
    }
}

/// `apply_to_all`: per i conflitti vale per i conflitti successivi dello stesso job; per gli errori
/// per gli errori successivi con lo stesso codice (solo `Skip`; `DeletePermanent` vale per i rifiuti
/// del Cestino successivi). Con `Retry` è ignorato: ripeterebbe all'infinito un errore persistente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PromptReply {
    Conflict { decision: Decision, apply_to_all: bool },
    Error { action: ErrorAction, apply_to_all: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ErrorAction {
    Retry,
    Skip,
    Cancel,
    /// Fino al Task 17 (helper elevato) si comporta come `Retry`.
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

const EMIT_EVERY: Duration = Duration::from_millis(100);
const SPEED_WINDOW: Duration = Duration::from_secs(2);

struct Job {
    view: JobView,
    /// Preso dal worker all'avvio.
    spec: Option<JobSpec>,
    started: bool,
    pause: bool,
    cancel: bool,
    /// Prompt in attesa: id e se è un conflitto.
    prompt: Option<(u64, bool)>,
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
    sink: Sink,
}

#[derive(Clone)]
pub struct Queue(Arc<Inner>);

impl Queue {
    // ponytail: lo scheduler non termina mai (nessuno shutdown all'uscita dell'app, per ora).
    pub fn new(exec: Arc<dyn Executor>, sink: Sink) -> Self {
        let state = State { jobs: Vec::new(), next_id: 0, next_prompt: 0 };
        let q = Queue(Arc::new(Inner { state: Mutex::new(state), cv: Condvar::new(), exec, sink }));
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
            let end = runner::run(&h, &*h.q.0.exec, &spec);
            h.finish(end);
        });
    }

    pub fn enqueue(&self, spec: JobSpec) -> JobId {
        let mut volumes: Vec<String> =
            spec.sources.iter().chain(spec.dest.iter()).map(|p| volume_key(&normalize_lexical(p))).collect();
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
            errors: Vec::new(),
        };
        let job = Job {
            view,
            spec: Some(spec),
            started: false,
            pause: false,
            cancel: false,
            prompt: None,
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

    /// Un id sconosciuto o una risposta del tipo sbagliato danno `INVALID_REQUEST`.
    pub fn reply(&self, prompt_id: u64, reply: PromptReply) -> Result<(), AppError> {
        let mut st = self.lock();
        let is_conflict = matches!(reply, PromptReply::Conflict { .. });
        let job = st
            .jobs
            .iter_mut()
            .find(|j| j.prompt == Some((prompt_id, is_conflict)))
            .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, None))?;
        job.prompt = None;
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

    /// Invia il prompt e aspetta la risposta, senza timeout; `None` se il job viene annullato.
    pub(crate) fn ask(&self, make: impl FnOnce(u64) -> Prompt) -> Option<PromptReply> {
        let mut st = self.q.lock();
        st.next_prompt += 1;
        let prompt = make(st.next_prompt);
        let job = st.job(self.id).expect("job del worker");
        job.prompt = Some((prompt.prompt_id(), matches!(prompt, Prompt::Conflict { .. })));
        job.reply = None;
        self.q.set_state(job, JobState::Waiting);
        (self.q.0.sink)(Event::Prompt(prompt));
        loop {
            let job = st.job(self.id).expect("job del worker");
            if job.cancel {
                job.prompt = None;
                return None;
            }
            if let Some(r) = job.reply.take() {
                let s = if job.pause { JobState::Paused } else { JobState::Running };
                self.q.set_state(job, s);
                return Some(r);
            }
            st = self.q.wait(st);
        }
    }

    fn finish(&self, end: JobState) {
        let mut st = self.q.lock();
        let job = st.job(self.id).expect("job del worker");
        job.view.current = None;
        job.prompt = None;
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
            errors: vec![],
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
        copies: AtomicUsize,
    }

    impl Executor for Fake {
        fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError> {
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
            LocalExecutor.copy_file(src, dst, overwrite, progress)
        }
        fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError> {
            if self.cross_device {
                return Err(AppError::new(ErrorCode::NotSameDevice, Some(src)));
            }
            LocalExecutor.move_path(src, dst, overwrite)
        }
        fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError> {
            if self.no_recycle && !permanent {
                return Err(AppError::new(ErrorCode::NoRecycleBin, Some(path)));
            }
            LocalExecutor.delete(path, is_dir, permanent)
        }
        fn mkdir(&self, path: &Path) -> Result<(), AppError> {
            LocalExecutor.mkdir(path)
        }
        fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError> {
            LocalExecutor.rename(src, dst)
        }
    }

    fn queue(exec: impl Executor + 'static) -> (Queue, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let q = Queue::new(Arc::new(exec), Arc::new(move |e| {
            let _ = tx.send(e);
        }));
        (q, rx)
    }

    fn spec(kind: JobKind, sources: Vec<PathBuf>, dest: Option<&Path>, policy: ConflictPolicy) -> JobSpec {
        JobSpec { kind, sources, dest: dest.map(Path::to_path_buf), filter: None, policy }
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
        assert_eq!(q.list()[0].state, Waiting);
        reply_conflict(&q, &p, Decision::Skip, true);
        let v = finish(&rx, id);
        assert_eq!((v.state, v.files_done, v.files_total), (Done, 2, 2));
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
        let fake = Fake { fail_copy_at: Some(2), cross_device: true, ..Default::default() };
        let (q, rx) = queue(fake);
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
        let Prompt::Conflict { src, .. } = &p else { panic!("{p:?}") };
        assert_eq!(src.as_str(), a.to_str().unwrap());
        let new = next_free_name(&a);
        reply_conflict(&q, &p, Decision::Rename(new.clone()), false);
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
        assert_eq!(r, PromptReply::Conflict { decision: Decision::Rename("x".into()), apply_to_all: true });
        let r: PromptReply = serde_json::from_value(json!({"type": "error", "action": {"type": "deletePermanent"}, "applyToAll": false})).unwrap();
        assert_eq!(r, PromptReply::Error { action: ErrorAction::DeletePermanent, apply_to_all: false });
        let v = serde_json::to_value(view(3, &["C:"], Waiting)).unwrap();
        assert_eq!(v["state"], json!({"type": "waiting"}));
        assert_eq!(v["filesDone"], json!(0));
        assert_eq!(v["speedBps"], json!(0));
    }
}
