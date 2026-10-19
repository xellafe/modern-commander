# Modern Commander — Helper elevato: correzione TOCTOU sulle giunzioni — piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** un'operazione elevata ("Riprova come amministratore") non può più essere dirottata fuori dal percorso richiesto da una giunzione o da un collegamento simbolico. Vale sia per un collegamento già presente nel percorso, sia per uno inserito tra la validazione e l'esecuzione.

**Architecture:** oggi l'helper valida i percorsi solo in modo lessicale (`protocol::is_safe`) e poi chiama le primitive di `LocalExecutor`, che seguono i reparse point nei componenti intermedi. Con la correzione, per ogni richiesta l'helper apre uno per uno i componenti del percorso dalla radice al padre del bersaglio, con `FILE_FLAG_OPEN_REPARSE_POINT`. Rifiuta la richiesta se un componente è un reparse point. Tiene gli handle aperti, senza `FILE_SHARE_DELETE`, finché la primitiva non finisce: in quel tempo nessuno può rinominare o eliminare quelle cartelle, quindi nemmeno sostituirle con una giunzione. Le primitive restano quelle di oggi. Le giunzioni legittime (per esempio un'unità `subst` o una cartella collegata dall'utente) le risolve l'app, che non è elevata, prima di inviare la richiesta. L'helper riceve così solo percorsi reali.

**Tech Stack:** Rust, crate `windows` (feature già presenti: `Win32_Storage_FileSystem`, `Win32_Foundation`). Nessuna dipendenza nuova.

**Spec:** `docs/superpowers/specs/2026-10-16-modern-commander-design.md`, §6.2 "Elevazione" (sicurezza non semplificabile: "I percorsi vengono validati e normalizzati nell'helper") e §8 (helper elevato come superficie d'attacco). Origine: `docs/superpowers/fase-1-follow-up.md`, voce "Sicurezza (fase 2)", e i "Rischi residui" della nota di sicurezza in `src-tauri/src/elevation/mod.rs`.

## Global Constraints

- Valgono le Global Constraints del piano della fase 1 (`docs/superpowers/plans/2026-10-16-fase-1-fondamenta.md`): solo Windows 10/11 x64, MIT, commit solo con conferma esplicita dell'utente e mai push, italiano per documentazione, commenti e commit, identificatori in inglese, errori come `AppError { code, path, detail }`.
- Le regole di sicurezza di §6.2 restano tutte: questo piano le aggiunge, non ne toglie nessuna. `validate` (lessicale) resta il primo controllo.
- L'helper non esegue un'operazione se non tiene aperti, per tutta la sua durata, gli handle di ogni cartella antenata del bersaglio.
- Gli handle di blocco chiedono un diritto di accesso ai dati: `FILE_LIST_DIRECTORY` (in alternativa `FILE_TRAVERSE` se il primo è negato) per le cartelle e `FILE_READ_DATA` per i file, con condivisione `FILE_SHARE_READ | FILE_SHARE_WRITE`, `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`. Un handle con i soli attributi (`FILE_READ_ATTRIBUTES`) non partecipa al controllo della condivisione e quindi non blocca niente.
- Il blocco impedisce di rinominare o eliminare una cartella, non di trasformarla sul posto in giunzione: `FSCTL_SET_REPARSE_POINT` basta un handle con `FILE_WRITE_ATTRIBUTES`, che la condivisione non esclude, ma la cartella deve essere vuota (`ERROR_DIR_NOT_EMPTY`, verificato). Le cartelle antenate più in alto contengono un figlio bloccato e quindi non si svuotano; la più profonda sì. Per questo il blocco crea una **sentinella** nella cartella bloccata più profonda (e, con `lock_with_item` su una cartella, dentro l'elemento): file `.mc-lock-<uuid v4>` aperto con `DELETE | FILE_READ_ATTRIBUTES`, condivisione 0, `CREATE_NEW`, `FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE`. Nessuno può eliminarla finché l'handle è aperto, la cartella resta non vuota e sparisce da sola alla chiusura (anche se l'helper termina). Se la sentinella non si crea, il blocco fallisce con l'errore di `CreateFileW` sul percorso della cartella: niente operazione senza sentinella.
- Errore per un reparse point dove non è ammesso: `ErrorCode::Unsupported` con `detail` `reparse_point_in_path` (componente intermedio) o `reparse_point` (bersaglio; è il valore che `expand` usa già). Nessun nuovo codice di errore.

## Review Focus

1. **Giunzione inserita in un componente intermedio dopo la validazione** (`C:\ProgramData\X\cache` sostituita con una giunzione a `C:\Windows\System32` durante un'eliminazione elevata): mentre l'helper esegue la primitiva la sostituzione deve fallire; se la giunzione c'era già prima, la richiesta va rifiutata. → test in Task 1 e Task 2.
2. **Percorsi legittimi che passano per una giunzione o un'unità `subst`**: "Riprova come amministratore" deve continuare a funzionare, perché l'app risolve il percorso prima dell'invio. → test in Task 2.
3. **Le primitive funzionano con gli antenati bloccati**: copia (con `.mcpart` e rinomina finale), spostamento, rinomina, eliminazione definitiva e nel Cestino, creazione di cartella ed elenco non devono fallire per violazione di condivisione con i propri stessi handle. → test in Task 1.
4. **Bersaglio che è un collegamento**: la copia elevata non deve leggere il file a cui punta un collegamento simbolico. La rimozione della sola lettura non deve toccare il file puntato. L'elenco elevato di una giunzione non deve mostrare il contenuto della cartella puntata. → test in Task 1 e Task 2.
5. **Cartelle intermedie che l'amministratore non può elencare**: l'apertura ripiega su `FILE_TRAVERSE`; se anche quella fallisce, l'errore è `ACCESS_DENIED` sul componente, non un panico né un'operazione eseguita senza blocco. → test in Task 1.

---

### Task 1: Blocco dei percorsi e rimozione della sola lettura senza seguire i collegamenti

**Files:**
- Create: `src-tauri/src/elevation/pathlock.rs`
- Modify: `src-tauri/src/elevation/mod.rs` (`pub mod pathlock;`), `src-tauri/src/ops/exec.rs` (`clear_readonly`, `clear_readonly_of`)

**Interfaces:**
- Consumes: `AppError`, `ErrorCode`, `to_extended`, `normalize_lexical` (fase 1); `owned`, `raw` di `elevation/mod.rs`
- Produces:

```rust
/// Handle aperti su ogni cartella da sotto la radice fino al padre di un percorso (e, se richiesto,
/// sull'elemento stesso). Finché vive, quelle cartelle non si possono rinominare né eliminare.
pub struct PathLock { /* Vec<OwnedHandle> */ }
pub fn lock_ancestors(path: &Path) -> Result<PathLock, AppError>;
/// Come `lock_ancestors`, più l'elemento stesso, che non deve essere un reparse point.
pub fn lock_with_item(path: &Path) -> Result<PathLock, AppError>;
```

  `exec.rs`: `clear_readonly(p, attrs)` e `clear_readonly_of(dst)` non usano più `SetFileAttributesW`, che segue i collegamenti. Aprono l'elemento con `FILE_WRITE_ATTRIBUTES` e `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS` e impostano gli attributi con `SetFileInformationByHandle(FileBasicInfo)`, con i tempi a 0 (lasciati invariati). La firma resta quella attuale. La correzione vale anche per l'esecutore non elevato.

  Regole di `lock_ancestors`:
  - Il percorso si normalizza con `normalize_lexical` e si apre in forma estesa. I componenti si aprono in ordine dalla radice in giù; la radice dell'unità (`C:\`) o della condivisione (`\\srv\share\`) non si apre.
  - Ogni componente si apre con le opzioni delle Global Constraints. Si legge `GetFileInformationByHandle`: con `FILE_ATTRIBUTE_REPARSE_POINT` → `Unsupported` / `reparse_point_in_path`, con `path` uguale al componente. Un componente che non è una cartella → `NotFound` (stesso esito che avrebbe la primitiva).
  - `ACCESS_DENIED` su `FILE_LIST_DIRECTORY` → nuovo tentativo con `FILE_TRAVERSE`; se fallisce anche quello, l'errore con il percorso del componente.
  - `lock_with_item` blocca anche l'elemento finale (cartella o file, `FILE_LIST_DIRECTORY` / `FILE_READ_DATA` = stesso valore `0x1`); se è un reparse point → `Unsupported` / `reparse_point`.
  - Sentinella (Global Constraints): `lock_ancestors` la crea nella cartella padre del percorso; `lock_with_item` la crea anche dentro l'elemento se è una cartella. Gli handle delle sentinelle vivono nel `PathLock`. `PathLock::hides(&self, name: &OsStr) -> bool` dice se `name` è una delle sue sentinelle: serve all'elenco elevato per non mostrarla.

- [ ] **Step 1: Test** (in `pathlock.rs`; le giunzioni si creano con `cmd /c mklink /J`, che non richiede l'amministratore)

```rust
fn junction(link: &Path, target: &Path) {
    let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
    assert!(ok, "mklink /J");
}
#[test] fn junction_in_the_middle_is_refused() {
    let d = tempfile::tempdir().unwrap(); let real = d.path().join("real");
    std::fs::create_dir(&real).unwrap(); junction(&d.path().join("link"), &real);
    let e = lock_ancestors(&d.path().join(r"link\f.txt")).err().unwrap();
    assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("reparse_point_in_path")));
    assert_eq!(e.path.as_deref().map(Path::new), Some(d.path().join("link").as_path()));
    assert!(lock_ancestors(&real.join("f.txt")).is_ok());
}
#[test] fn locked_ancestors_cannot_be_renamed_or_swapped() {
    let d = tempfile::tempdir().unwrap(); let a = d.path().join("a");
    std::fs::create_dir_all(a.join("b")).unwrap();
    let lock = lock_ancestors(&a.join(r"b\f.txt")).unwrap();
    let err = std::fs::rename(&a, d.path().join("a2")).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(32));                                    // ERROR_SHARING_VIOLATION
    assert!(std::fs::rename(a.join("b"), a.join("b2")).is_err());
    drop(lock);
    std::fs::rename(&a, d.path().join("a2")).unwrap();
}
#[test] fn primitives_work_under_the_lock() {
    /* d\a\b con f.txt (sola lettura); tenendo lock_ancestors(d\a\b\x) per tutta la durata, con LocalExecutor:
       mkdir(d\a\b\x); copy_file(f.txt → d\a\b\g.txt, overwrite false) e poi di nuovo con overwrite true;
       rename(g.txt → h.txt); move_path(h.txt → x\h.txt, false); delete(x\h.txt, false, permanent=false) (Cestino);
       delete(f.txt, false, true) (sola lettura tolta via handle); delete(x, true, true); list_dir(d\a\b): tutto Ok */
}
#[test] fn item_lock_refuses_a_link_and_pins_the_item() {
    let d = tempfile::tempdir().unwrap(); let real = d.path().join("real");
    std::fs::create_dir(&real).unwrap(); junction(&d.path().join("link"), &real);
    let e = lock_with_item(&d.path().join("link")).err().unwrap();
    assert_eq!((e.code, e.detail.as_deref()), (ErrorCode::Unsupported, Some("reparse_point")));
    let lock = lock_with_item(&real).unwrap();
    assert!(std::fs::rename(&real, d.path().join("moved")).is_err());
    drop(lock);
}
#[test] fn sentinel_keeps_the_deepest_folder_non_empty() {
    /* d\a vuota; lock_ancestors(d\a\x): d\a contiene esattamente un elemento, e `lock.hides` lo riconosce;
       eliminarlo dà ERROR_SHARING_VIOLATION; dopo drop(lock) d\a è di nuovo vuota.
       lock_with_item(d\a): una sentinella dentro d\a e una in d. */
}
#[test] fn traverse_fallback_and_access_denied() {
    /* d\a\b; icacls d\a /deny <utente corrente>:(RD) (niente elenco, attraversamento ammesso):
       lock_ancestors(d\a\b\x) → Ok; poi /deny anche (X): → AccessDenied con path d\a.
       Ripristino delle ACL alla fine anche in caso di fallimento (guardia con Drop). */
}
#[test] fn missing_or_file_component() {
    /* d\file.txt è un file: lock_ancestors(d\file.txt\x) → NotFound; lock_ancestors(d\nope\x) → NotFound */
}
```

  In `exec.rs`:

```rust
#[test] fn clearing_readonly_does_not_follow_symlinks() {
    /* target.txt con sola lettura; link.txt = std::os::windows::fs::symlink_file(target); se la creazione fallisce
       (niente privilegio né modalità sviluppatore) il test stampa il motivo ed esce senza fallire.
       clear_readonly_of(link.txt) → Ok; target.txt è ancora di sola lettura */
}
```

- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml` → FAIL (simboli mancanti).
- [ ] **Step 3:** Implementare `pathlock.rs` e il cambio di `clear_readonly`/`clear_readonly_of`. I test esistenti di `exec.rs` sulla sola lettura (`readonly_file_permanent_delete` e la sostituzione di un `dst` di sola lettura) devono restare verdi senza modifiche.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` → PASS. Se `primitives_work_under_the_lock` fallisce per violazione di condivisione su una primitiva, **fermarsi e riferire all'utente**: vuol dire che quella API apre una cartella antenata in modo esclusivo, e cambia il design di quella primitiva.
- [ ] **Step 5: Commit** `fix(elevation): blocco dei componenti del percorso e sola lettura senza seguire i collegamenti`

### Task 2: L'helper esegue solo su percorsi bloccati; l'app risolve i collegamenti prima dell'invio

**Files:**
- Create: `src-tauri/src/ops/rooted.rs`
- Modify: `src-tauri/src/elevation/helper.rs` (`execute`), `src-tauri/src/elevation/client.rs` (nessuna risoluzione per chiamata), `src-tauri/src/ops/runner.rs` (`executor`), `src-tauri/src/ops/mod.rs`, `src-tauri/tests/elevation_pipe.rs`

**Interfaces:**
- Consumes: Task 1 `lock_ancestors`, `lock_with_item`; fase 1 `validate`, `normalize_lexical`, `LocalExecutor`
- Produces:

```rust
// helper.rs
/// Blocchi da tenere durante l'esecuzione di `req` (percorsi già validati e normalizzati).
fn locks_for(req: &Request) -> Result<Vec<PathLock>, AppError>;
// ops/rooted.rs
/// Cartella risolta con GetFinalPathNameByHandleW (segue giunzioni, `subst` e unità di rete mappate).
/// Se `dir` non esiste si risolve l'antenato esistente più profondo e si riattacca il resto;
/// se non si risolve niente, `dir` invariato.
pub(crate) fn resolve_dir(dir: &Path) -> PathBuf;
/// Radici di un job, normalizzate: la cartella padre di ogni sorgente; `dest` se è la cartella di
/// destinazione di Copy/Move, altrimenti (nome di file, `file_target`, o Rename) la sua cartella padre.
/// Una radice non è mai l'elemento su cui si opera: risolverla seguirebbe un collegamento.
pub(crate) fn job_roots(spec: &JobSpec) -> Vec<PathBuf>;
/// Esecutore elevato di un job: ogni percorso che sta sotto una radice (la più lunga) riceve il
/// prefisso risolto di quella radice; gli altri passano invariati. Le radici si risolvono una sola
/// volta, alla creazione.
pub(crate) struct RootedExecutor { /* inner: Arc<dyn Executor>, roots: Vec<(PathBuf, PathBuf)> */ }
impl RootedExecutor { pub(crate) fn new(inner: Arc<dyn Executor>, roots: Vec<PathBuf>) -> Self; }
impl Executor for RootedExecutor { /* mappa i percorsi e delega */ }
```

  **Perché per job e non per chiamata** (decisione dell'utente dopo la revisione): se l'app risolvesse ogni percorso subito prima dell'invio, una giunzione messa al posto di una sottocartella tra due richieste dello stesso job (dopo l'elenco di un'eliminazione ricorsiva) verrebbe seguita dall'app, e l'helper riceverebbe un percorso "reale" nel bersaglio. Risolvendo solo le radici, una volta, quando il job passa all'helper, una giunzione comparsa sotto una radice arriva all'helper come giunzione e viene rifiutata. Le giunzioni legittime che l'utente attraversa per arrivare alla cartella del job (unità `subst`, cartella collegata) stanno sopra o sulla radice e quindi si risolvono.

  `runner.rs`: `executor()` crea il `RootedExecutor` la prima volta che il job passa all'helper (da `self.h.elevated()` e `job_roots(self.spec)`) e lo riusa per il resto del job. L'helper non sa niente delle radici: i controlli di sicurezza restano tutti nell'helper.

  `locks_for`, per richiesta:

  | Richiesta | Blocchi |
  |---|---|
  | `Copy { src, dst }` | `lock_with_item(src)` (la sorgente non può essere un collegamento: la copia elevata leggerebbe il file puntato) + `lock_ancestors(dst)` |
  | `Move { src, dst }`, `Rename { src, dst }` | `lock_ancestors(src)` + `lock_ancestors(dst)`; il bersaglio può essere un collegamento: si sposta il collegamento stesso |
  | `Delete { permanent: true }` | `lock_ancestors(path)`; un collegamento si elimina come collegamento (`remove_dir`/`remove_file` non lo seguono) |
  | `Delete { permanent: false }` | `lock_with_item(path)`: un collegamento non va nel Cestino come amministratore. Se il Cestino fallisce per violazione di condivisione con l'handle sull'elemento: `lock_ancestors(path)` + rifiuto (`reparse_point`) se l'elemento è un reparse point, e il rischio residuo va nella nota di sicurezza |
  | `Mkdir` | `lock_ancestors(path)` |
  | `List` | `lock_with_item(path)`; le voci per cui `lock.hides(nome)` non si inviano |

  `execute` diventa: `validate` → normalizzazione (come oggi) → `locks_for` → primitiva (le voci di `List` si inviano sotto blocco) → rilascio dei blocchi (drop) → risposta finale. Rilasciare prima della risposta evita che l'app riceva `Ok` mentre la sentinella è ancora nella cartella.

- [ ] **Step 1: Test** (in `helper.rs` e `client.rs`, chiamando le funzioni senza pipe)

```rust
// helper.rs
#[test] fn requests_through_a_junction_are_refused_and_change_nothing() {
    /* d\protected\keep.txt; d\work\link = giunzione a d\protected.
       execute(Delete { path: d\work\link\keep.txt, is_dir: false, permanent: true }) → Unsupported/reparse_point_in_path,
       keep.txt esiste ancora; Mkdir d\work\link\new → stesso errore, d\protected\new non esiste;
       Copy { src: d\work\link\keep.txt, dst: d\out.txt } → stesso errore, d\out.txt non esiste */
}
#[test] fn link_targets_follow_the_table() {
    /* d\real con f.txt; d\link giunzione a d\real.
       List { d\link } → Unsupported/reparse_point; Delete { d\link, is_dir: true, permanent: false } → Unsupported/reparse_point;
       Delete { d\link, is_dir: true, permanent: true } → Ok: sparisce la giunzione, d\real\f.txt esiste ancora */
}
#[test] fn ordinary_requests_still_work() { /* Mkdir, Copy, Rename, Move, Delete definitivo, List su un albero senza collegamenti: tutti Ok */ }
// ops/rooted.rs
#[test] fn resolve_dir_follows_junctions_and_keeps_unresolvable_paths() {
    /* d\link giunzione a d\real: resolve_dir(d\link) → long_path(d\real) (confronto con same_path;
       long_path perché %TEMP% può contenere nomi 8.3, che GetFinalPathNameByHandleW espande);
       resolve_dir(d\link\nuova\sotto) → long_path(d\real)\nuova\sotto; resolve_dir(Z:\inesistente con unità assente) → invariato;
       il risultato passa sempre `validate` se lo passava l'originale */
}
#[test] fn rooted_executor_maps_under_roots_only_and_resolves_once() {
    /* esecutore finto che registra i percorsi ricevuti. Radici [d\link, d\altro]; d\link giunzione a d\real.
       mkdir(d\link\a\b) → d\real\a\b (prefisso più lungo); mkdir(d\fuori\x) → invariato.
       Poi d\link si sostituisce con una giunzione a d\altrove: mkdir(d\link\c) → ancora d\real\c (niente nuova risoluzione) */
}
```

  In `runner.rs`: un test con l'esecutore elevato finto mostra che dopo "Riprova come amministratore" i percorsi sotto la cartella del job arrivano con il prefisso risolto una sola volta.

  In `tests/elevation_pipe.rs` (helper vero avviato senza elevazione con `spawn_with`, come i test esistenti):
  - `helper_refuses_mkdir_through_junction_sent_raw`: una richiesta `Mkdir` scritta a mano sulla pipe con un percorso che attraversa una giunzione → risposta `Err` con `Unsupported` / `reparse_point_in_path`, nessuna cartella creata nel bersaglio della giunzione.
  - `rooted_elevated_executor_resolves_legitimate_junctions`: `RootedExecutor::new(ElevatedExecutor, [d\link]).mkdir(d\link\new)` → Ok, e `d\real\new` esiste; dopo aver sostituito una sottocartella `d\real\sub` con una giunzione, `mkdir(d\link\sub\x)` → `Unsupported` / `reparse_point_in_path`.

- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks` → FAIL.
- [ ] **Step 3:** Implementare `locks_for`, il nuovo `execute`, `ops/rooted.rs` e l'uso nel runner. Per la risoluzione: `CreateFileW(cartella, FILE_READ_ATTRIBUTES, share READ|WRITE|DELETE, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS)`, che segue i collegamenti, poi `GetFinalPathNameByHandleW(..., VOLUME_NAME_DOS)` e `normalize_lexical` sul risultato.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks` → PASS, compresi tutti i test esistenti di `elevation_pipe.rs`.
- [ ] **Step 5: Commit** `fix(elevation): l'helper esegue solo su percorsi bloccati e senza giunzioni intermedie`

### Task 3: Documentazione e verifica manuale con UAC

**Files:**
- Modify: `src-tauri/src/elevation/mod.rs` (nota di sicurezza), `docs/superpowers/fase-1-follow-up.md`

- [ ] **Step 1:** Nota di sicurezza in `elevation/mod.rs`:
  - in "Cosa garantiscono i controlli" aggiungere il blocco dei componenti, la sentinella (perché serve: una cartella vuota si trasforma in giunzione anche se bloccata) e il rifiuto dei reparse point;
  - nei limiti: l'app risolve le radici del job quando il job passa all'helper, quindi una radice sostituita prima di quel momento viene seguita; l'eliminazione nel Cestino controlla l'elemento senza bloccarlo (il Cestino non può spostare un elemento bloccato), quindi resta una finestra in cui l'elemento si può scambiare con un collegamento, che finisce nel Cestino come collegamento; i reparse point che non sono collegamenti (OneDrive "File su richiesta", deduplicazione) sono rifiutati anche loro; una cartella in cui l'amministratore non può creare file (supporti in sola lettura) non si blocca;
  - in "Rischi residui" sostituire la voce sulle giunzioni con i limiti che restano: volumi montati in una cartella (anche quelli sono reparse point, quindi rifiutati) e percorsi che l'app non riesce a risolvere (arrivano invariati e l'helper li rifiuta se contengono un collegamento).
- [ ] **Step 2: Verifica manuale** con il vero prompt UAC, build `npm run tauri dev`:
  - F7 in `C:\Program Files` → "Riprova come amministratore" → la cartella si crea (la funzione non è regredita).
  - Unità `subst X: C:\Program Files` → F7 in `X:\` → "Riprova come amministratore" → la cartella si crea in `C:\Program Files` (risoluzione lato app).
  - Unità di rete mappata (`net use Z: \\localhost\c$`) → F7 elevato in `Z:\Program Files` → verificare se ora funziona: la risoluzione dà `\\?\UNC\localhost\c$\...`, che l'helper vede. Se funziona, il follow-up "unità di rete mappate non visibili all'helper elevato" si può chiudere.
  - Giunzione `C:\Users\Public\mc-test\link` → `C:\Windows\Temp\mc-target` (con un file dentro): eliminazione elevata di `link\file` → il file viene eliminato attraverso la risoluzione dell'app, il che è corretto perché l'utente vede dove punta il collegamento. Eliminazione elevata della giunzione stessa con Maiusc+F8 → sparisce la giunzione e il contenuto del bersaglio resta.
- [ ] **Step 3:** `fase-1-follow-up.md`: togliere la voce "TOCTOU sulle giunzioni" e aggiornare quella sulle unità di rete mappate secondo l'esito dello Step 2.
- [ ] **Step 4: Commit** `docs(elevation): nota di sicurezza aggiornata dopo la correzione TOCTOU`
