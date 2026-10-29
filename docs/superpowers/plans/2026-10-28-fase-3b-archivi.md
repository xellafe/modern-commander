# Fase 3b "Archivi": piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** archivi ZIP, 7z, TAR e TAR.GZ aperti come cartelle, ovunque stia l'archivio (locale, SFTP, FTP, HTTP); estrazione con la coda; creazione (Alt+F5) e modifica degli ZIP, anche sul remoto.

**Architecture:** `Loc` guadagna la variante `Archive { container, path }`, con forma testuale `arc://<contenitore>/<percorso interno>`. Il router `Vfs` la instrada a un `ArchiveProvider` che legge il contenitore attraverso il suo provider (locale o remoto) e tiene in cache l'indice. Così lista, ispettore, viewer e coda leggono gli archivi come leggono il remoto, e l'estrazione è la copia tra provider della 3a. La scrittura non passa dal provider: il runner della coda raccoglie in uno `ZipEdit` le modifiche del job verso uno ZIP e alla fine riscrive l'archivio una volta (temporaneo e rinomina; sul remoto upload, poi rinomina, o eliminazione e rinomina dove la sostituzione atomica non c'è). La creazione (Alt+F5) è un job `Pack`: una copia verso la radice di uno ZIP nuovo.

**Tech Stack:** come la fase 3a (Tauri 2, Rust, React 19 + TypeScript, Zustand, i18next, Vitest). Nessuna crate nuova; cambiano le feature, sempre con `cargo add`:
- `zip`: `deflate-flate2`, `deflate64`, `bzip2`, `lzma` (niente `zstd`, `ppmd`, `xz`, `aes-crypto`);
- `sevenz-rust2`: `deflate`, `bzip2` (niente `ppmd`); le feature di `[dev-dependencies]` restano;
- `windows`: `Win32_System_Time` (per `TzSpecificLocalTimeToSystemTime`).

Il builder verifica nomi e disponibilità delle feature sul `Cargo.toml` delle versioni installate (`zip` 9.0.5, `sevenz-rust2` 0.24.1).

**Spec:** `docs/superpowers/specs/2026-10-28-fase-3b-archivi-design.md`, con il design della 3a (`2026-10-22-fase-3a-rete-design.md`) e lo spec generale (§6.6).

## Global Constraints

- Valgono i vincoli dei piani della fase 2 e della fase 3a ("Global Constraints").
- **Commit solo con conferma esplicita dell'utente**: i passi "Commit" sono proposte da confermare. Niente push.
- Documentazione, commenti e messaggi di commit in italiano; identificatori in inglese.
- Nessuna stringa UI nel codice: chiavi i18n sempre in `en.json` e `it.json`. Il nuovo `ErrorCode` `ARCHIVE_TOO_DEMANDING` ha `errors.ARCHIVE_TOO_DEMANDING` in entrambe ed entra nell'unione `ErrorCode` di `src/ipc/types.ts`.
- **Un `Loc::Archive` non arriva mai** a `to_extended`, `normalize_lexical`, `std::fs` né alle API della shell di Windows; il contenitore locale si apre solo dentro `archive::container`.
- **Archivio non fidato**: nessuna allocazione dimensionata da un campo dell'archivio senza tetto. Con `panic = "abort"` un OOM chiude l'app.
- **L'originale di uno ZIP resta intatto** finché il nuovo archivio non è completo: errore o annullamento eliminano i temporanei (locali e remoti) e basta.
- I job che toccano un archivio non usano mai l'helper elevato (`local_job` falso).
- Valori fissati dallo spec e dal piano:
  - dizionario LZMA/LZMA2 (e memoria PPMd) oltre **256 MiB** → `ARCHIVE_TOO_DEMANDING`;
  - cache degli indici: **8** archivi, ricontrollo di dimensione e data del contenitore al più ogni **2 s**;
  - voci per archivio oltre **1 000 000** → `UNSUPPORTED` (detail `too_many_entries`);
  - lettura dell'intestazione codificata 7z per il controllo: al più **64 KiB**;
  - lettore a canale (`pipe`): **4** blocchi da **64 KiB**; lettura anticipata del contenitore **256 KiB**;
  - cursore 7z inattivo: chiuso dopo **30 s**.

## Review Focus

1. **ZIP creati da Explorer o da strumenti vecchi**: senza voci per le cartelle, con nomi CP437 o con `\` come separatore. Si navigano come cartelle con i nomi giusti. → test `implicit_dirs_backslashes_and_cp437_names` (Task 2).
2. **La lista non si aggiorna dopo una modifica**: dopo il commit di uno ZIP la scheda dell'archivio mostra subito il contenuto nuovo, senza aspettare i 2 s della cache. → test `commit_invalidates_index_cache` (Task 7).
3. **Connessione caduta durante l'upload** di uno ZIP modificato: l'originale sul server resta intatto e il temporaneo remoto viene eliminato. → test `remote_install_failure_keeps_original` (Task 6).
4. **Voci con nomi ostili** (`../x`, `/etc/x`, `C:\x`, `a:b`, `CON`) estratte insieme a voci buone: `INVALID_NAME` solo per quelle voci, le altre si estraggono e niente finisce fuori dalla destinazione. → test `extract_bad_names_are_per_item` (Task 5).
5. **Annullamento di Alt+F5 verso il server**: non restano né file sul server né temporanei locali. → test `pack_to_remote_cancelled_leaves_nothing` (Task 7).

---

### Task 1: Posizioni d'archivio (`Loc::Archive`)

**Files:**
- Modify: `src-tauri/src/vfs/loc.rs`, `src-tauri/src/fs/mod.rs` (`local_only`, `normalize_paths`), `src-tauri/src/ops/queue.rs` (`valid_rename`)

**Interfaces:**
- Produces (`vfs::loc`):
  - variante `Loc::Archive { container: Box<Loc>, path: String }`. Il contenitore è `Local` o `Remote`, mai `Archive`, e ha un nome di file. `path` è POSIX pulito come quello remoto (`clean`).
  - Forma testuale: `arc://<contenitore codificato><path>`. La codifica del contenitore sostituisce solo `%` con `%25` e `/` con `%2F`. Esempi: `arc://C:\rel\app.zip/conf`, `arc://sftp:%2F%2Fweb1%2Frel%2Fapp.zip/`.
  - `Loc::parse`: lo schema `arc` (senza distinguere le maiuscole) si riconosce prima della ricerca tra gli schemi remoti. La decodifica accetta solo `%25` e `%2F`/`%2f`: qualunque altro `%` dà `INVALID_REQUEST`, come un contenitore `Archive` o senza nome di file.
  - `pub fn archive(container: Loc, path: &str) -> Loc`, che pulisce `path`.
  - `join` e `file_name` sul percorso interno; `parent` della radice dell'archivio = `None`.
  - `is_within` e `same`: stesso contenitore (`same` del contenitore) e percorso interno contenuto o uguale.
  - `volume_key()` = `arc://` + chiave del contenitore. Per un contenitore locale si usa `paths::normalize_lexical` e il minuscolo, perché due forme dello stesso file devono dare la stessa chiave. Così due job sullo stesso archivio vanno in sequenza e uno spostamento tra archivio e disco non è mai un "rename".
  - `as_local()` = `None`. `is_remote()` = `true`: il commento diventa "fuori dal file system locale (remoto o dentro un archivio)".
  - `pub fn archive_container(&self) -> Option<&Loc>`.
- `fs::local_only`: `UNSUPPORTED` anche per `Archive`. `normalize_paths`: lascia intatta ogni posizione con `is_remote()`. `valid_rename`: `(Archive, Archive)` con la stessa regola di `(Remote, Remote)`.

- [ ] **Step 1: Write the failing tests** in `loc.rs`:

```rust
#[test]
fn archive_roundtrip_local_and_remote_container() {
    let l = Loc::parse(r"arc://C:\rel\app.zip/conf/a b%#.xml").unwrap();
    assert_eq!(l, Loc::Archive { container: Box::new(Loc::Local(r"C:\rel\app.zip".into())), path: "/conf/a b%#.xml".into() });
    assert_eq!(l.to_string(), r"arc://C:\rel\app.zip/conf/a b%#.xml");
    let r = Loc::archive(Loc::parse("sftp://w/rel/100%/app.zip").unwrap(), "/");
    assert_eq!(r.to_string(), "arc://sftp:%2F%2Fw%2Frel%2F100%25%2Fapp.zip/");
    assert_eq!(Loc::parse(&r.to_string()).unwrap(), r);
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<Loc>(&json).unwrap(), r);
}

#[test]
fn archive_parse_rejects_nesting_and_bad_escapes() {
    let nested = format!("arc://{}/x", Loc::parse(r"arc://C:\a.zip/b.zip").unwrap().to_string().replace('%', "%25").replace('/', "%2F"));
    for bad in [nested.as_str(), "arc://C:\\a.zip%41/x", "arc:///x", "arc://sftp:%2F%2Fw%2F/x"] {
        assert_eq!(Loc::parse(bad).unwrap_err().code, ErrorCode::InvalidRequest, "{bad}");
    }
}

#[test]
fn archive_navigation_and_keys() {
    let root = Loc::parse(r"arc://C:\a.zip/").unwrap();
    let f = root.join("d").join("..").join("e");
    assert_eq!(f.to_string(), r"arc://C:\a.zip/e");
    assert_eq!(f.parent().unwrap(), root);
    assert_eq!(root.parent(), None);
    assert_eq!(root.file_name(), None);
    assert!(f.is_within(&root) && !f.is_within(&Loc::parse(r"arc://C:\b.zip/").unwrap()));
    assert!(Loc::parse(r"arc://c:\A.ZIP/e").unwrap().same(&f));
    assert_eq!(Loc::parse(r"arc://c:\A.ZIP/").unwrap().volume_key(), root.volume_key());
    assert_ne!(root.volume_key(), Loc::parse(r"C:\a.zip").unwrap().volume_key());
    assert!(root.is_remote() && root.as_local().is_none());
}
```

In `fs/mod.rs`, `local_only(r"arc://C:\a.zip/x")` dà `UNSUPPORTED`. In `queue.rs`, `valid_rename` accetta `arc://C:\a.zip/d/x → arc://C:\a.zip/d/y` e rifiuta `arc://C:\a.zip/d/x → arc://C:\b.zip/d/y`.

- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml loc` → FAIL (variante inesistente).
- [ ] **Step 3: Implement** la variante e i metodi qui sopra; ogni `match` su `Loc` che il compilatore segnala tratta `Archive` come `Remote`, salvo dove il task dice altro.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` → PASS, tutti i test esistenti compresi.
- [ ] **Step 5: Commit** (dopo conferma): `feat(vfs): posizioni dentro un archivio`

---

### Task 2: Contenitore e indice: ZIP e TAR

**Files:**
- Create: `src-tauri/src/archive/container.rs`, `src-tauri/src/archive/index.rs`
- Modify: `src-tauri/src/archive/tar_walk.rs` (da funzione a iteratore), `src-tauri/src/archive/list.rs` (ZIP tramite l'indice, `dos_to_ms` in ora locale), `src-tauri/src/archive/mod.rs`, `src-tauri/Cargo.toml` (feature `zip`, `windows`)

**Interfaces:**
- Produces (`archive::container`):
  - `#[derive(Clone)] pub struct ContainerFile`, che implementa `Read + Seek + Send`. Ogni clone ha la sua posizione e la sua lettura anticipata (256 KiB), quindi i clone si leggono in modo indipendente.
  - `pub fn open(connector: &Arc<dyn Connector>, container: &Loc) -> Result<ContainerFile, AppError>`:
    - locale: `open_shared` del percorso esteso, letture posizionali con `std::os::windows::fs::FileExt::seek_read`;
    - remoto: `Provider::read_at` con `Purpose::Transfer`, chiedendo il provider al connettore a ogni lettura, così nessuna sessione resta occupata dalla cache.
  - `pub fn len(&self) -> u64`.
- Produces (`archive::index`):
  - `pub struct IndexEntry { pub path: String, pub is_dir: bool, pub link: bool, pub bad_name: bool, pub size: u64, pub modified_ms: Option<i64>, pub order: u64, pub at: EntryAt }`. `path` è POSIX con `/` iniziale. `order` è la posizione nell'archivio.
  - `pub enum EntryAt { Zip(usize), SevenZ { file: usize, block: Option<usize> }, Tar { data: u64 }, TarGz }`.
  - `pub struct Index { pub format: ArchiveFormat, pub solid: bool, … }`, con:
    - `pub fn get(&self, path: &str) -> Option<&IndexEntry>`;
    - `pub fn stat(&self, path: &str) -> Option<Stat>` (cartelle implicite comprese, radice `/` sempre cartella);
    - `pub fn list(&self, dir: &str) -> Option<Vec<Stat>>`;
    - `pub fn entries(&self) -> &[IndexEntry]` (in ordine d'archivio).
  - `pub fn build_zip(file: ContainerFile, cancel: &AtomicBool) -> Result<(Index, zip::ZipArchive<ContainerFile>), AppError>`. Lo `ZipArchive` resta in cache e si clona per ogni lettura.
  - `pub fn build_tar(r: impl Read, seekable: bool, cancel: &AtomicBool) -> Result<Index, AppError>`. Con `seekable` le voci sono `EntryAt::Tar { data }`, altrimenti `EntryAt::TarGz`.
  - `pub(crate) fn normalize_name(raw: &str) -> (String, bool)`: percorso pulito e `bad_name`.
- Produces (`archive::tar_walk`):
  - `pub(super) struct TarWalker<R>`, con `next_header(&mut self) -> Result<Option<TarHeader>, AppError>`;
  - `pub(super) struct TarHeader { name: String, flag: u8, data_len: u64, shown_size: u64, modified_ms: Option<i64>, data_offset: u64 }`;
  - `walk_tar` resta, scritto sopra `TarWalker`, per l'elenco limitato dell'ispettore.
- Regole dell'indice:
  - **Nomi.** `\` diventa `/`. Si tolgono `./` iniziali e la barra finale. `bad_name` vale per i componenti `..`, per i nomi assoluti (`/x`) e per quelli con lettera di unità (`C:`). La voce resta elencata sotto il nome pulito, dove `..` è risolto e il prefisso tolto.
  - **Duplicati.** Vince l'ultima voce.
  - **Link.** `link` vale per i flag TAR `1`/`2` e per le voci ZIP con modo Unix `S_IFLNK`.
  - **Cifratura.** Le voci ZIP cifrate restano nell'indice: l'errore arriva alla lettura (Task 3).
  - **Troppe voci.** Oltre 1 000 000 voci: `UNSUPPORTED` con detail `too_many_entries`.
  - **Date ZIP.** Si usa il campo esteso UT (`0x5455`), letto da `ZipFile::extra_data_fields`. Se manca, la data DOS si legge in ora locale con `TzSpecificLocalTimeToSystemTime`. Il campo NTFS (`0x000a`) si usa solo se il crate lo espone già.
- Correzioni TAR dal follow-up della fase 2:
  - `MultiGzDecoder`;
  - un TAR troncato dentro i dati di una voce dà `CORRUPT_ARCHIVE`;
  - un `L` di size 0 o un PAX `path=` vuoto danno `CORRUPT_ARCHIVE`.

- [ ] **Step 1: Write the failing tests** (`index.rs`, archivi costruiti nel test con `zip::ZipWriter` e con l'helper `tar_into` di `list.rs`):
  - `implicit_dirs_backslashes_and_cp437_names`: ZIP con le sole voci `a\b\c.txt` (nome scritto con `\`) e `x/y.txt`, più un nome CP437 senza flag UTF-8 (byte `0x82` = `é`). Le verifiche:
    - `list("/")` dà `a`, `x` e il file CP437, nell'ordine dei nomi della voce;
    - `stat("/a/b")` è una cartella;
    - `get("/a/b/c.txt").unwrap().size == 3`;
    - il nome CP437 è `é.txt`.
  - `duplicates_last_wins_and_bad_names_flagged`: ZIP con `d/f` (contenuto "1"), `d/f` ("22"), `../up`, `/abs`, `C:/drv`. Le verifiche:
    - `get("/d/f").size == 2`;
    - `get("/up").bad_name`, `get("/abs").bad_name` e `get("/C:/drv").bad_name` sono veri.
  - `zip_dos_time_is_local`: voce con data DOS 2026-07-01 12:00:00 e nessun campo esteso. `modified_ms` è uguale all'ora locale convertita con `TzSpecificLocalTimeToSystemTime`, calcolata nel test con la stessa API.
  - `tar_index_offsets_and_links`: TAR con un file `a` (5 byte), un link simbolico `l` e una cartella `d/`. Le verifiche:
    - con `seekable` i byte del contenitore da `EntryAt::Tar { data }` per 5 sono il contenuto di `a`;
    - `get("/l").link` è vero.
  - `tar_truncated_in_data_is_corrupt`: TAR tagliato a metà dei dati dell'ultima voce → `CORRUPT_ARCHIVE`.
  - `container_clones_read_independently` (`container.rs`): due clone di un `ContainerFile` locale, letti alternandosi, danno ognuno i byte giusti.
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml archive::` → FAIL.
- [ ] **Step 3: Implement** `container.rs`, `index.rs` e `TarWalker`. In `list.rs`, `list_zip` passa da `build_zip` e prende le prime `limit` voci con i percorsi a `\`. `ArchiveEntry` resta com'è per l'ispettore.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml archive::` → PASS, compresi i test di `list.rs` e `tar_walk.rs`.
- [ ] **Step 5: Commit** (dopo conferma): `feat(archive): indice di ZIP e TAR letto attraverso il provider del contenitore`

---

### Task 3: Protezione e indice 7z

**Files:**
- Create: `src-tauri/src/archive/sevenz_guard.rs`
- Modify: `src-tauri/src/archive/index.rs` (`build_7z`), `src-tauri/src/archive/list.rs` (`list_7z` tramite `build_7z`), `src-tauri/src/error.rs` (`ArchiveTooDemanding`), `src-tauri/Cargo.toml` (feature `sevenz-rust2`), `src/ipc/types.ts`, `src/i18n/en.json`, `src/i18n/it.json`

**Interfaces:**
- Produces (`archive::sevenz_guard`):
  - `pub fn check_coder(method_id: &[u8], props: &[u8]) -> Result<(), AppError>`:
    - LZMA (`03 01 01`): dizionario `u32` LE in `props[1..5]`;
    - LZMA2 (`21`): `p = props[0]`; `p == 40` vale 4 GiB − 1, altrimenti `(2 | (p & 1)) << (p / 2 + 11)`; `p > 40` dà `CORRUPT_ARCHIVE`;
    - PPMd (`03 04 01`): memoria `u32` LE in `props[1..5]`;
    - oltre 256 MiB: `ARCHIVE_TOO_DEMANDING`. Proprietà troppo corte: `CORRUPT_ARCHIVE`. Gli altri metodi passano.
  - `pub fn check_header(r: &mut (impl Read + Seek)) -> Result<(), AppError>`:
    - legge l'intestazione di avvio (32 byte) e, se l'intestazione successiva è codificata (`0x17`), ne legge al più 64 KiB;
    - analizza `PackInfo`/`UnpackInfo` fino ai coder di ogni folder e chiama `check_coder`;
    - una struttura che non si analizza dà `CORRUPT_ARCHIVE`;
    - si chiama prima di `sevenz_rust2::Archive::read`.
  - `pub fn check_blocks(a: &sevenz_rust2::Archive) -> Result<(), AppError>`: `check_coder` su tutti i coder di tutti i blocchi, con `Coder::encoder_method_id()` e `properties()`.
- Produces (`archive::index`): `pub fn build_7z(file: ContainerFile, cancel: &AtomicBool) -> Result<(Index, sevenz_rust2::Archive), AppError>`:
  - prima `check_header`, poi `Archive::read`, poi `check_blocks`;
  - `EntryAt::SevenZ { file, block }` da `stream_map.file_block_index`;
  - `solid` = `Archive::is_solid`;
  - date da `last_modified_date`;
  - mappatura degli errori come oggi in `list_7z` (cifrato → `UNSUPPORTED` con detail `encrypted`).
- `ErrorCode::ArchiveTooDemanding` (`ARCHIVE_TOO_DEMANDING`).

- [ ] **Step 1: Write the failing tests** (`sevenz_guard.rs`):
  - `lzma2_dictionary_limits`: `check_coder(&[0x21], &[36])` (256 MiB) → Ok; `&[37]` (384 MiB) → `ARCHIVE_TOO_DEMANDING`; `&[41]` → `CORRUPT_ARCHIVE`; `&[]` → `CORRUPT_ARCHIVE`.
  - `lzma_dictionary_limits`: props `[0x5D, 0,0,0,0x10]` (256 MiB) → Ok; `[0x5D, 0,0,0,0x20]` → `ARCHIVE_TOO_DEMANDING`; `[0x5D, 0, 0]` → `CORRUPT_ARCHIVE`.
  - `crafted_encoded_header_is_refused_before_the_crate`: 7z costruito a mano con intestazione codificata, un folder e un coder LZMA con dizionario 4 GiB − 1. `build_7z` → `ARCHIVE_TOO_DEMANDING`, senza panico.
  - `normal_7z_passes`: 7z creato nel test con `sevenz-rust2` (feature `compress` in `dev-dependencies`), solido e con due file → `build_7z` ok, `solid` vero, i due file con `EntryAt::SevenZ { block: Some(0) }`.
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml sevenz` → FAIL.
- [ ] **Step 3: Implement** il controllo, `build_7z` e il nuovo codice d'errore, con le chiavi i18n:
  - `en`: "This archive needs too much memory to open (dictionary over 256 MB).";
  - `it`: "Questo archivio richiede troppa memoria per essere aperto (dizionario oltre 256 MB).".
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml archive::` e `npm test` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(archive): indice 7z con limite al dizionario dei decoder`

---

### Task 4: `ArchiveProvider`, cache e instradamento

**Files:**
- Create: `src-tauri/src/archive/provider.rs`, `src-tauri/src/archive/cache.rs`, `src-tauri/src/archive/pipe.rs`, `src-tauri/src/archive/cursor.rs`
- Modify: `src-tauri/src/vfs/router.rs` (`Vfs` con la cache, `remote` per `Archive`), `src-tauri/src/vfs/provider.rs` (`stream_order`), `src-tauri/src/vfs/mem.rs` (contatore dei byte letti), `src-tauri/src/fs/source.rs` (`open_source` per `Archive`), `src-tauri/src/archive/mod.rs` (`list_archive` per i contenitori remoti)

**Interfaces:**
- Consumes: `container::open`, `index::build_zip`/`build_tar`/`build_7z` (Task 2-3), `net::temp::fetch_in`.
- Produces:
  - `Provider::stream_order(&self, path: &str) -> Option<u64> { None }`: nuovo metodo con default. L'`ArchiveProvider` dà `Some(order)` solo per le voci dei formati a flusso (7z solido, TAR.GZ).
  - `pub struct ArchiveCache`, con questi metodi:
    - `pub fn new() -> Self`;
    - `pub fn provider(&self, connector: &Arc<dyn Connector>, container: &Loc) -> Result<Arc<dyn Provider>, AppError>`;
    - `pub fn invalidate(&self, container: &Loc)`.

    La cache segue queste regole:
    - **Capienza:** 8 voci, la meno usata di recente esce per prima.
    - **Chiave:** il contenitore (`volume_key` del `Loc::Archive` di radice).
    - **Validità:** la voce vale finché dimensione e data del contenitore non cambiano, controllate al più ogni 2 s.
    - **Contenuto di ogni voce:** l'`Index`, lo `ZipArchive` o il `sevenz_rust2::Archive`, l'eventuale copia temporanea locale (eliminata quando la voce esce) e lo slot del cursore.
  - `Vfs::new(connector)` crea la cache. `Vfs::remote` su `Loc::Archive` restituisce `(cache.provider(…)?, path)`. `pub fn archives(&self) -> &ArchiveCache`.
  - `pub fn piped(f: impl FnOnce(&mut dyn Write) -> Result<(), AppError> + Send + 'static) -> Box<dyn Read + Send>` (`pipe.rs`):
    - `f` gira in un thread e scrive in un canale limitato (4 × 64 KiB);
    - un errore di `f` arriva al lettore come `io::Error::other(AppError)`, quindi `stream_error` lo ritrova;
    - chiudere il lettore fa fallire la scrittura successiva e il thread finisce.
  - `ArchiveProvider` implementa `Provider`:
    - `caps()`: `writable` = ZIP e contenitore `writable`; `atomic_replace` falso; `precise_mtime` falso per ZIP; `ranged_read` vero.
    - `stat`/`list` dall'indice. Un `link` ha `dir_link` falso ed è un file.
    - `reader`:
      - `bad_name` → `INVALID_NAME`;
      - `link` → `UNSUPPORTED` con detail `reparse_point`, come `unsupported_link` del runner;
      - voce ZIP cifrata → `UNSUPPORTED` con detail `encrypted`; metodo non supportato → `UNSUPPORTED`;
      - ZIP: `piped` su un clone dello `ZipArchive`;
      - TAR: `ContainerFile` posizionato su `data` e limitato a `size`;
      - 7z e TAR.GZ: il cursore (sotto).
    - `read_at`: `reader`, poi si saltano `offset` byte e si leggono `len`.
    - Scritture (`writer`, `rename`, `delete`, `mkdir`, `set_meta`): `UNSUPPORTED`. Le modifiche passano dal runner (Task 7).
  - **Cursore** (`cursor.rs`): uno slot `Arc<Mutex<Option<Cursor>>>` per voce di cache.
    - **TAR.GZ:** il cursore è il `TarWalker` sopra `MultiGzDecoder<ContainerFile>` più la posizione `order`. Per una voce più avanti si prosegue saltando; per una voce più indietro, o senza cursore, si riparte dall'inizio. Il lettore restituito, quando lo si chiude dopo aver letto tutta la voce, rimette il cursore nello slot.
    - **7z:** il cursore è un thread che esegue `BlockDecoder::for_each_entries` sul blocco e che, voce per voce, aspetta su un canale la richiesta della voce successiva:
      - le voci non chieste si scartano;
      - quella chiesta passa nel canale del lettore;
      - con una richiesta per un blocco diverso, o per una voce già passata, si chiude il thread e se ne apre uno nuovo;
      - senza richieste per 30 s il thread si chiude.
      - I file non solidi (un blocco per file) usano lo stesso meccanismo.
  - **Copia locale.** Un contenitore TAR.GZ remoto, o uno con `ranged_read` falso, si scarica una volta con `net::temp::fetch_in` (annullamento con il flag della richiesta che costruisce l'indice) e si legge da lì.
  - **`list_archive`**:
    - contenitore locale: come oggi;
    - contenitore remoto ZIP o 7z: dall'indice in cache, con le prime `limit` voci in `ArchiveEntry` (percorsi a `\`);
    - TAR e TAR.GZ remoti: `UNSUPPORTED` (l'ispettore non scarica).
  - `open_source` su `Archive`: come per `Remote` (`RemoteFile` sopra il provider).

- [ ] **Step 1: Write the failing tests** (`provider.rs`; contenitori su disco con `tempfile` e su `MemProvider` con `vfs::mem::vfs`):
  - `zip_on_remote_reads_index_and_one_entry_only`: ZIP da 4 MiB su `sftp://m/rel/app.zip`, con una voce `big.bin` da 4 MiB "stored" e una `conf/a.xml` da 10 byte. Le verifiche:
    - `vfs.remote(arc://sftp:%2F%2Fm%2Frel%2Fapp.zip/conf)` → `list` dà `a.xml`;
    - leggere `a.xml` con `reader` dà i 10 byte;
    - i byte letti da `MemProvider` (nuovo contatore `bytes_read: AtomicU64`) sono meno di 1 MiB.
  - `solid_7z_block_decoded_once`: 7z solido da 200 file da 20 KiB ciascuno, su `MemProvider`. Si leggono tutti i file con `reader` nell'ordine `stream_order` e ognuno dà il contenuto giusto. `bytes_read` è al più la dimensione del contenitore × 1,1 più 64 KiB.
  - `targz_cursor_goes_forward_and_restarts_backwards`: TAR.GZ locale con `a`, `b`, `c`. Si leggono `c` e poi `a`, ed entrambi danno il contenuto giusto. `stream_order` di `a` < `b` < `c`.
  - `remote_targz_is_downloaded_once`: TAR.GZ su `MemProvider`. Due `vfs.remote` consecutivi riusano la stessa copia: `bytes_read` è uguale alla dimensione del file dopo due `list`.
  - `cache_revalidates_on_change`: si elenca un ZIP locale, lo si riscrive con una voce in più, si attende 2,1 s e si rielenca: c'è la voce nuova. Con `invalidate` subito, senza attesa, si vede lo stesso.
  - `bad_entries_fail_on_read`: le voci `bad_name`, `link` e cifrata danno `INVALID_NAME`, `UNSUPPORTED` (`reparse_point`) e `UNSUPPORTED` (`encrypted`).
  - `pipe_propagates_errors_and_stops_on_drop` (`pipe.rs`): un errore `AppError` di `f` si ritrova con `stream_error`. Chiudere il lettore dopo il primo blocco fa finire il thread entro 1 s.
  - In `fs/mod.rs`: `read_chunk` su `arc://<zip>/conf/a.xml` (offset 2, len 3) dà i byte 2..5.
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml archive::provider` → FAIL.
- [ ] **Step 3: Implement** cache, provider, pipe, cursori e instradamento.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(archive): archivi come provider di cartella, in sola lettura`

---

### Task 5: Estrazione con la coda

**Files:**
- Modify: `src-tauri/src/ops/plan.rs`, `src-tauri/src/ops/runner.rs`

**Interfaces:**
- Consumes: `Provider::stream_order` (Task 4).
- Produces:
  - in `expand`, una sorgente dentro un archivio non resta mai `unlisted`: le sue cartelle si elencano subito, ricorsivamente. Con la `volume_key` del Task 1 lo spostamento verso fuori non è mai un "rename".
  - `pub(crate) fn stream_sort(items: &mut Vec<FileItem>, vfs: &Vfs)` in `plan.rs`, chiamata alla fine di `expand` per Copy, Move e Pack. Se almeno una sorgente ha `stream_order`, l'elenco diventa: le cartelle nell'ordine attuale, poi i file in ordine `(contenitore, stream_order)`. I file senza `stream_order` restano dopo, nell'ordine attuale.
  - Nessun'altra modifica al runner per l'estrazione: `check_name` (archivio = `is_remote`), `transfer::copy` e i controlli dei nomi di `transfer::write` valgono già.

- [ ] **Step 1: Write the failing tests** (`runner.rs` o i test della coda in `queue.rs`, con lo stesso helper dei test 3a):
  - `extract_zip_to_local_with_conflicts`: Copy di `arc://<zip>/conf` verso una cartella locale che contiene già `conf/a.xml`, con `ConflictPolicy::Skip`. Le verifiche:
    - gli altri file si estraggono;
    - `a.xml` non cambia;
    - la data di modifica di un file estratto è entro 2 s da quella della voce.
  - `extract_bad_names_are_per_item`: ZIP con `ok.txt`, `../up.txt`, `a:b.txt`, `CON`, Copy della radice con `skip_all` per `INVALID_NAME`. Le verifiche:
    - si estrae solo `ok.txt`;
    - `errors` ha 3 elementi `INVALID_NAME`;
    - la cartella madre della destinazione non contiene `up.txt`.
  - `move_out_of_readonly_archive_fails_per_item`: Move da un 7z verso il locale. Ogni file dà `UNSUPPORTED` all'eliminazione, i file estratti restano e il 7z non cambia.
  - `stream_sort_orders_files_by_archive`: elementi da un TAR.GZ con cartelle `b/` e `a/` e file in ordine d'archivio `b/2`, `a/1`. Dopo `stream_sort`: `b`, `a`, `b/2`, `a/1`.
  - `extract_from_remote_zip_to_other_remote`: Copy da `arc://sftp:%2F%2Fm%2Fa.zip/x.txt` verso `sftp://n/dst`: il contenuto arriva su `n`.
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml extract` → FAIL.
- [ ] **Step 3: Implement** l'elenco subito delle sorgenti d'archivio e `stream_sort`.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(ops): estrazione dagli archivi con la coda`

---

### Task 6: Riscrittura degli ZIP (`ZipEdit`) e installazione sicura

**Files:**
- Create: `src-tauri/src/archive/edit.rs`
- Modify: `src-tauri/src/vfs/transfer.rs` (`install`)

**Interfaces:**
- Produces (`vfs::transfer`): `pub fn install(vfs: &Vfs, dst: &Loc, local: &Path, replace: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError>`. Il caso locale non passa da qui (vedi `commit`). Solo per `dst` remoto:
  1. upload di `local` su `<dst>.<8 cifre esadecimali>.mcpart` (`writer(…, create_new = true)`). Un rifiuto (`ACCESS_DENIED`) è un errore e `dst` resta intatto;
  2. se `dst` esisteva, si riapplicano i permessi POSIX (`set_meta(tmp, None, mode)`);
  3. se `dst` non esiste o `caps().atomic_replace` è vero, `rename(tmp, dst, replace)`. Altrimenti, con `replace`, si fa `delete(dst)` e poi `rename(tmp, dst, false)`; se quest'ultima rinomina fallisce, l'errore ha come `path` la forma testuale del temporaneo remoto e il temporaneo **non** si elimina;
  4. ogni altro errore o annullamento elimina il temporaneo remoto.

  Senza `replace` un `dst` esistente dà `ALREADY_EXISTS` prima dell'upload. Non c'è mai scrittura diretta su `dst` e non si chiede conferma (decisione dell'utente).
- Produces (`archive::edit`): `pub struct ZipEdit`, con questi metodi:
  - `pub fn open(vfs: &Vfs, container: &Loc) -> Result<ZipEdit, AppError>`: lo ZIP esiste; si annotano dimensione e data;
  - `pub fn create(container: &Loc, replace: bool) -> ZipEdit`: ZIP nuovo (Pack);
  - `pub fn container(&self) -> &Loc`;
  - `pub fn stage_file(&mut self) -> Result<PathBuf, AppError>`: file nuovo nella cartella di lavoro dell'edit, sotto `net::temp::dir()`;
  - `pub fn put(&mut self, path: &str, staged: PathBuf, modified_ms: Option<i64>)`;
  - `pub fn mkdir(&mut self, path: &str)` (la radice `/` non fa nulla);
  - `pub fn delete(&mut self, path: &str)` (la voce e tutto quello sotto `path/`, anche con una cartella implicita);
  - `pub fn rename(&mut self, from: &str, to: &str)` (la voce e tutto quello sotto);
  - `pub fn is_empty(&self) -> bool`;
  - `pub fn commit(self, vfs: &Vfs, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError>`;
  - `pub fn abort(self)` (elimina la cartella di lavoro; anche `Drop` la elimina).
- `commit`:
  1. con `open`, si ristatta il contenitore: dimensione o data diverse danno `CHANGED_ON_DISK`;
  2. le operazioni si applicano in ordine a una mappa nome → origine (`Original(indice)`, `Staged`, `Dir`). Le voci originali restano nel loro ordine, quelle nuove vengono dopo;
  3. il nuovo ZIP si scrive:
     - contenitore locale: in `<contenitore>.<8 cifre>.mcpart` accanto all'originale;
     - remoto: in un file della cartella di lavoro;
     - le voci originali si copiano con `raw_copy_file_rename`, senza ricomprimerle;
     - le nuove con Deflate, data di modifica, `large_file` sopra 4 GiB e nomi UTF-8;
     - le cartelle create con `add_directory`;
  4. installazione:
     - locale: `LocalProvider::rename(tmp, contenitore, replace)`, con `replace = true` per `open`;
     - remoto: `transfer::install`;
  5. si chiama `vfs.archives().invalidate(contenitore)`; su errore o annullamento si eliminano i temporanei e l'originale resta com'era.

  `progress` riceve i byte scritti più quelli caricati; con `false` dà `CANCELLED`.

- [ ] **Step 1: Write the failing tests** (`edit.rs`, `transfer.rs`):
  - `edit_put_delete_rename_mkdir_keeps_untouched_entries_raw`: ZIP locale con `a.txt`, `d/b.txt`, `d/c.txt`. Operazioni: `put("/new.txt")`, `delete("/d/b.txt")`, `rename("/d", "/e")`, `mkdir("/empty")`, poi `commit`. Le verifiche sul risultato:
    - le voci sono `a.txt`, `e/c.txt`, `new.txt`, `empty/`;
    - i byte compressi e il CRC di `a.txt` sono identici a quelli originali, letti con `by_index_raw`.
  - `commit_failure_and_cancel_keep_original`: `progress` che restituisce `false` al primo blocco, poi un `put` di un file nella cartella di lavoro eliminato prima del commit. In entrambi i casi:
    - l'originale ha gli stessi byte di prima;
    - nella sua cartella non resta nessun `*.mcpart`;
    - la cartella di lavoro non c'è più.
  - `commit_detects_external_change`: dopo `open`, lo ZIP si riscrive dall'esterno → `commit` dà `CHANGED_ON_DISK` e il file esterno resta.
  - `remote_install_failure_keeps_original` (`transfer.rs`, `MemProvider` con `atomic_replace = false`): `progress` alza `offline` a metà upload. Le verifiche:
    - `dst` resta con i byte originali;
    - nessun `*.mcpart` su `m`.
  - `install_without_atomic_replace_deletes_then_renames`: `MemProvider` con `atomic_replace = false`, `dst` esistente con `mode = 0o640` e `replace`. Le verifiche:
    - `dst` ha il contenuto nuovo e `mode` 0o640;
    - nessun `*.mcpart`.
  - `install_reports_temp_name_when_final_rename_fails`: `MemProvider` che rifiuta la rinomina dopo l'eliminazione (nuovo flag di test `fail_rename_after_delete`). Le verifiche:
    - l'errore ha `path` che finisce con `.mcpart`;
    - quel file su `m` contiene l'archivio completo.
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml edit install` → FAIL.
- [ ] **Step 3: Implement** `ZipEdit` e `install`.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(archive): riscrittura degli ZIP con installazione sicura anche sul remoto`

---

### Task 7: Modifica e creazione degli ZIP nella coda (job `Pack`)

**Files:**
- Modify: `src-tauri/src/ops/model.rs` (`JobKind::Pack`), `src-tauri/src/ops/runner.rs`, `src-tauri/src/ops/plan.rs`, `src/ipc/types.ts` (`JobKind`), `src/i18n/en.json`, `src/i18n/it.json` (nome del tipo di job nella scheda Operazioni, accanto alle chiavi esistenti degli altri tipi)

**Interfaces:**
- Consumes: `ZipEdit` (Task 6).
- Produces:
  - `JobKind::Pack { replace: bool }` (JSON `{ "type": "pack", "replace": true }`). È una Copy verso `dest` = radice `Loc::Archive` di uno ZIP che può non esistere. `expand` la tratta come Copy (`file_target` falso).
  - Il runner ha `edits: Vec<ZipEdit>` (uno per contenitore) e `deferred: Vec<(Loc, bool)>`. Le operazioni elementari su un `Loc::Archive` passano dall'edit del suo contenitore, aperto alla prima operazione: con `ZipEdit::create(container, replace)` per Pack, con `ZipEdit::open` negli altri casi.
    - `mkdir_at` → `mkdir`.
    - `delete_at` → `delete`, sempre definitiva: il flag `permanent` si ignora.
    - `move_path`/`rename` tra due posizioni dello stesso archivio → `rename`. Il caso va trattato prima del ramo "stesso `volume_key` remoto"; tra archivi diversi o tra archivio e altro si ha `NOT_SAME_DEVICE`.
    - `copy_file` verso un archivio: `transfer::copy` dalla sorgente a `Loc::Local(edit.stage_file()?)`, poi `put` con la data della sorgente.
  - **Move verso un archivio.** L'eliminazione della sorgente si rimanda (`deferred`) a dopo il commit riuscito, e così la rimozione delle cartelle sorgente (`src_dirs`).
  - **Fine del job:**
    - se `run()` è `Ok`, ogni edit non vuoto fa `commit`, con `current` = nome del contenitore, `bytes_total` aumentato della dimensione del contenitore (il doppio se è remoto) e `progress` collegato a `bytes_done` e `checkpoint()`;
    - un commit fallito mette l'errore nel riepilogo, dà `JobState::Failed` e non esegue i `deferred`;
    - poi si eseguono i `deferred`;
    - con `Stop` (annullamento) si fa `abort` di tutti gli edit e i `deferred` si scartano.
  - Una sorgente in un archivio non scrivibile (7z, TAR) dà `UNSUPPORTED` alla prima operazione di scrittura, come gli altri errori per elemento.

- [ ] **Step 1: Write the failing tests** (test della coda, con lo ZIP su disco e su `MemProvider`):
  - `copy_into_zip_then_listing_shows_it`: Copy di due file locali verso `arc://<zip>/conf` con `ConflictPolicy::Ask`. Il conflitto su `conf/a.xml` si risponde con Sovrascrivi. Le verifiche:
    - dopo il job lo ZIP contiene il file nuovo e la `a.xml` nuova;
    - le altre voci sono intatte.
  - `commit_invalidates_index_cache`: si elenca l'archivio con `list_dir`, poi un job aggiunge `x.txt`. Il `list_dir` successivo, senza attesa, contiene `x.txt`.
  - `delete_mkdir_rename_inside_zip`: tre job (Delete permanent di `/d`, Mkdir `/n`, Rename `/a.txt` → `/b.txt`) e il contenuto finale atteso.
  - `move_into_zip_deletes_sources_only_after_commit`: Move di `f.txt` locale verso uno ZIP su `MemProvider`, con `offline` alzato durante il commit. Le verifiche:
    - job `Failed`;
    - `f.txt` esiste ancora;
    - lo ZIP remoto non cambia.
  - `cancel_during_job_leaves_zip_untouched`: annullamento durante la copia del secondo di tre file verso uno ZIP. Lo ZIP ha gli stessi byte di prima e non resta nessun `*.mcpart`.
  - `pack_local_folder_to_new_zip`: Pack di una cartella con una sottocartella vuota verso `arc://<dir>\new.zip/` → ZIP con tutte le voci, la cartella vuota compresa, e date entro 2 s. Con `replace = false` e `new.zip` già esistente → `ALREADY_EXISTS` e il file esistente resta intatto.
  - `pack_to_remote_cancelled_leaves_nothing`: Pack verso `arc://sftp:%2F%2Fm%2Fout.zip/`, annullato durante l'upload. Le verifiche:
    - su `m` non c'è né `out.zip` né `*.mcpart`;
    - la cartella di lavoro non esiste più.
  - `jobs_on_same_zip_run_in_sequence`: due job sullo stesso ZIP hanno la stessa `volumes` e il secondo parte dopo la fine del primo.
- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml zip` → FAIL.
- [ ] **Step 3: Implement** `JobKind::Pack` e gli instradamenti del runner.
- [ ] **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` e `cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(ops): modifica e creazione degli ZIP con la coda`

---

### Task 8: F4 su una voce d'archivio (salvataggio)

**Files:**
- Modify: `src-tauri/src/fs/mod.rs` (`write_text_file`), `src-tauri/src/fs/content.rs`, `src-tauri/src/archive/edit.rs`

**Interfaces:**
- Produces:
  - `pub fn save_entry(vfs: &Vfs, loc: &Loc, bytes: &[u8], expected_modified_ms: Option<i64>, expected_size: Option<u64>) -> Result<(), AppError>` (`archive::edit`):
    1. `ZipEdit::open`;
    2. confronto con la voce attuale: data diversa (`modified_ms.unwrap_or(0)`) o dimensione diversa dà `CHANGED_ON_DISK`;
    3. `stage_file`, `put` con la data di adesso, `commit`.

    Su un archivio non ZIP dà `UNSUPPORTED`.
  - `write_text_file` su `Loc::Archive`: `encode_text` (lo stesso di `remote_write_text_file`), poi `save_entry`, poi `remote_file_info` sul provider d'archivio. `allow_unsafe` si ignora.

- [ ] **Step 1: Write the failing tests** (`content.rs` o `fs/mod.rs`):
  - `save_entry_replaces_one_entry`: ZIP locale con `conf/a.xml` e `big.bin`. Si salva `a.xml` con un testo nuovo. Le verifiche:
    - `read_file` dà il testo nuovo;
    - i byte compressi di `big.bin` sono invariati.
  - `save_entry_detects_change`: `expected_size` sbagliata → `CHANGED_ON_DISK` e lo ZIP non cambia.
  - `save_entry_on_7z_is_unsupported`.
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** `cargo test --manifest-path src-tauri/Cargo.toml` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(editor): salvataggio di una voce dentro uno ZIP`

---

### Task 9: Posizioni d'archivio nel frontend

**Files:**
- Modify: `src/lib/paths.ts`, `src/lib/paths.test.ts`, `src/state/tabView.ts` (`parentPath`), `src/commands/fileOps.ts`, `src/commands/navigation.ts`, `src/commands/shell.ts`, `src/state/tabViews.ts`, `src/state/jobs.ts`, `src/state/connectionLifetime.ts`, `src/components/groups/Breadcrumb.tsx`, `src/components/groups/TabStrip.tsx`, `src/components/groups/ConnIcon.tsx` (icona d'archivio)

**Interfaces:**
- Produces (`lib/paths.ts`):
  - `parseArchive(p: string): { container: string; inner: string } | null` e `formatArchive(container: string, inner = "/"): string`, con la stessa codifica di `Loc` (solo `%25` e `%2F`), inner pulito con `cleanPosix`;
  - `isArchive(p)`;
  - `isVirtual(p) = isRemote(p) || isArchive(p)`;
  - `isArchiveName(name)`, che riusa il riconoscimento di `previewKind` in `lib/preview.ts`;
  - `isZipName(name)`;
  - `connectionPath(p)`: il contenitore per un archivio, `p` altrimenti.
- Regole:
  - `sourceCaps(arc)` = `{ ...caps(false), write: isZipName(container) && sourceCaps(container).write }`.
  - `joinPath` usa `/` su `isVirtual`.
  - `parentPath` (`tabView.ts`) della radice dell'archivio è la cartella del contenitore.
  - `folderName` della radice è il nome del contenitore, così `cm_GoToParent` mette il cursore sull'archivio.
  - `rootPath(arc)` è la radice dell'archivio.
  - `pathCrumbs(arc)` = crumb del contenitore, con l'ultimo che punta alla radice dell'archivio, seguiti dai segmenti interni. Breadcrumb e TabStrip mostrano l'icona d'archivio sul crumb dell'archivio e sulla scheda. Per un contenitore remoto il primo crumb resta il nome della connessione.
  - `dirTarget(path, row)` su un file con `isArchiveName` restituisce `formatArchive(joinPath(path, row.name))`, ma non in `DRIVES` e non dentro un archivio (niente annidamento). Così Invio e "apri nell'altro gruppo" aprono l'archivio.
  - **Da `isRemote` a `isVirtual`**, con lo stesso comportamento del remoto: `nameError`, `serverAbsolute`, `mkdirTarget`, `resolveDest`, `noTrailing`, `readOnly()`, `startCopyMove`, `dropOnGroup`, `cm_Return` (un file nell'archivio si apre nel viewer), `shellPaths`, `showMenu`, `rowContextMenu`, `trackDragOut`, `cm_Edit` (niente editor esterno), `fetchListing` e `load` di `tabViews.ts`.
  - `fetchListing` chiama `ensureConnected` su `connectionPath(path)`. `connectionsInUse` conta il contenitore remoto di un archivio aperto.
  - **Errore all'apertura di un archivio** (`load` di una posizione `arc` che fallisce al primo caricamento): la scheda torna alla cartella del contenitore, con il cursore sull'archivio e l'avviso `errors.<CODE>` (`showNotice`).
  - **`reloadRemoteTabs`** (`jobs.ts`) ricarica anche tutte le schede d'archivio aperte alla fine di ogni job. *(ponytail: ricarica anche quelle non toccate; la cache dell'indice la rende economica.)*
  - **Cancellazione.** In una scheda d'archivio `openDelete` invia sempre `permanent: true`, e il dialogo di conferma mostra la nota "eliminazione definitiva dall'archivio" (nuova chiave i18n).

- [ ] **Step 1: Write the failing tests** (`paths.test.ts`, `navigation.test.ts`, `fileOps.test.ts`, `tabViews.test.ts` o equivalente):
  - `archive locations roundtrip like Loc`: `formatArchive("sftp://w/rel/100%/app.zip", "/a")` === `"arc://sftp:%2F%2Fw%2Frel%2F100%25%2Fapp.zip/a"` e `parseArchive` restituisce le parti; `parseArchive("C:\\a.zip")` === null.
  - `archive caps`: `sourceCaps("arc://C:\\a.zip/").write` è vero; `sourceCaps("arc://C:\\a.7z/").write` è falso; per `arc://http:%2F%2Fh%2Fa.zip/` `write` è falso; `hash`, `shell` e `watch` sono falsi.
  - `parent of archive root is the container folder`: `parentPath("arc://C:\\r\\a.zip/")` === `"C:\\r"`, `folderName(…)` === `"a.zip"`, `parentPath("arc://C:\\r\\a.zip/d")` === `"arc://C:\\r\\a.zip/"`.
  - `crumbs through the archive`: `pathCrumbs("arc://C:\\r\\a.zip/d")` ha le etichette `["C:", "r", "a.zip", "d"]`, e il path di `a.zip` è la radice dell'archivio.
  - `Enter on an archive opens it`: `cm_Return` su `a.zip` in `C:\r` apre `arc://C:\r\a.zip/` e non chiama `openPath`. Su `b.zip` dentro un archivio apre il viewer.
  - `failed archive open goes back to the container`: `listDir` simulato che rifiuta con `CORRUPT_ARCHIVE`. La scheda torna in `C:\r`, il cursore è su `a.zip` e l'avviso ha il codice `CORRUPT_ARCHIVE`.
  - `copy into a zip tab targets the archive`: `resolveDest` con destinazione in una scheda `arc://C:\r\a.zip/conf` dà quella posizione. Con un 7z dà `""` (non scrivibile).
- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3: Implement** le regole qui sopra.
- [ ] **Step 4:** `npm test` e `npx tsc --noEmit` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(ui): archivi come cartelle nelle schede`

---

### Task 10: Comandi Ctrl+PgDn, Alt+F5 e Alt+F9

**Files:**
- Create: `src/commands/archives.ts`, `src/commands/archives.test.ts`, `src/components/dialogs/PackDialog.tsx`
- Modify: `src/commands/tcKeymap.ts`, `src/lib/fkeys.ts` (`SHORT`), `src/state/ui.ts` (`DialogRequest` `pack`), il componente che rende i dialoghi, `src/main.tsx` (registrazione), `src/i18n/en.json`, `src/i18n/it.json`

**Interfaces:**
- Produces (`commands/archives.ts`), con `registerArchiveCommands()` e `ARCHIVE_COMMAND_IDS`:
  - **`mc_OpenArchive`** (`'Ctrl+PageDown'`, `when: 'fileList'`): apre come cartella l'archivio sotto il cursore, come Invio; su un elemento che non è un archivio non fa nulla. È un id `mc_` perché non esiste un `cm_` di TC equivalente.
  - **`cm_PackFiles`** (`'Alt+F5'`): apre il dialogo `pack`, con:
    - `sources` = selezione (marcati o cursore);
    - `name` = nome dell'elemento se ce n'è uno solo, altrimenti `folderName` della cartella corrente, più `.zip`;
    - `destDir` = cartella dell'altro gruppo.

    Il comando non c'è in `DRIVES`, dentro un archivio e quando `destDir` non è scrivibile (`sourceCaps`); in quei casi appare l'avviso `READ_ONLY_SOURCE`.
  - **`cm_UnpackFiles`** (`'Alt+F9'`): sull'archivio sotto il cursore (o sul solo marcato):
    1. `listDir` della radice dell'archivio;
    2. dialogo di copia esistente (`copyMove`) con `sources` = le voci della radice e `dest` = `joinPath(altroGruppo, nome senza estensione)` + separatore finale.

    Un archivio vuoto mostra l'avviso `errors.EMPTY_ARCHIVE` (nuova chiave i18n solo frontend).
- **`PackDialog`:**
  - campi nome (con `.zip` imposto se manca) e cartella di destinazione;
  - all'OK chiama `fileInfo` sul file di destinazione. Se esiste, mostra l'avviso "esiste già" e il pulsante diventa "Sovrascrivi" (`replace: true`). Cambiare il nome toglie l'avviso, e così si ottiene Rinomina. Annulla chiude;
  - invio: `enqueue({ kind: { type: "pack", replace }, sources, dest: formatArchive(joinPath(destDir, name)), filter: null, policy: { type: "ask" } })`.
- **Etichette.** `SHORT` delle F-key: `cm_PackFiles` → `shell.fkeys.pack`, `cm_UnpackFiles` → `shell.fkeys.unpack`. Titoli `commands.mc_OpenArchive`, `commands.cm_PackFiles`, `commands.cm_UnpackFiles` in entrambe le lingue.

- [ ] **Step 1: Write the failing tests** (`archives.test.ts`):
  - `every archive command has a title in both languages`, come negli altri file di comandi.
  - `keymap binds the TC keys`: `DEFAULT_KEYMAP["Alt+F5"] === "cm_PackFiles"`, `["Alt+F9"] === "cm_UnpackFiles"`, `["Ctrl+PageDown"] === "mc_OpenArchive"`; nessuna di queste combinazioni era già legata (il controllo dei conflitti esistente in `keymap.test.ts` resta verde).
  - `pack proposes name and other group`: con un solo elemento marcato `src` in `C:\w` e l'altro gruppo in `D:\out`, il dialogo ha `name === "src.zip"` e `destDir === "D:\\out"`.
  - `pack submits a pack job`: conferma con file inesistente → `enqueueJob` con `kind { type: "pack", replace: false }` e `dest === "arc://D:\\out\\src.zip/"`. Con file esistente e "Sovrascrivi" → `replace: true`.
  - `unpack lists the root into the copy dialog`: `listDir` simulato con `a`, `b`. Il dialogo `copyMove` ha `sources` `[".../a.zip/a", ".../a.zip/b"]` in forma `arc` e `dest` `"D:\\out\\a\\"`.
- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3: Implement** comandi, dialogo e chiavi i18n.
- [ ] **Step 4:** `npm test` e `npx tsc --noEmit` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(ui): comandi per aprire, creare ed estrarre archivi`

---

### Task 11: Viewer, editor e ispettore sulle voci d'archivio

**Files:**
- Modify: `src/components/viewer/Viewer.tsx`, `src/commands/shell.ts` (`cm_Edit`), `src/state/editors.ts`, `src/state/layout.ts` (`FileRef` con `readOnly?`), `src/state/inspector.ts`, i test vicini

**Interfaces:**
- **Viewer.** `needsCopy` = `!!state && !copy && (isArchive(path) || (isRemote(path) && NEEDS_LOCAL.includes(state.mode)))`: una voce d'archivio si estrae sempre con `fetchTemp` e si legge dalla copia locale. Annullato il download, si resta sull'esadecimale come per il remoto. `skipSample` vale anche per gli archivi con le modalità `NEEDS_LOCAL`.
- **Editor.**
  - `cm_Edit` su una voce di un archivio non scrivibile non blocca più con l'avviso: apre l'editor con `FileRef.readOnly = true`, che `editors.ts` porta in `doc.readOnly`.
  - Su uno ZIP scrivibile l'editor salva con `writeTextFile`, passando `expectedSize` come per il remoto.
  - L'editor esterno non si usa mai.
- **Ispettore.**
  - Le voci d'archivio seguono le regole del remoto: anteprima testuale o markdown e niente hash.
  - Un archivio su una scheda remota si elenca solo se è ZIP o 7z: il 7z resta "su richiesta" (`ON_DEMAND`) e il TAR/TAR.GZ remoto non mostra l'elenco. La riga `:117` di `inspector.ts` lo permette per ZIP e 7z.
  - *(La scelta di elencare il 7z da solo, prevista dallo spec, resta da decidere: il pulsante rimane finché non è verificato che la protezione copre tutti i percorsi che allocano.)*

- [ ] **Step 1: Write the failing tests:**
  - `Viewer downloads archive entries before reading`: in `Viewer.test.tsx` o nel test di stato equivalente, `fetchTemp` viene chiamato per `arc://C:\a.zip/x.txt` in modalità testo.
  - `Edit on a 7z entry opens read-only`: in `shell.test.ts`, `openFileTab` riceve `{ mode: "editor", readOnly: true }` e non viene mostrato nessun avviso.
  - `inspector lists a remote zip but not a remote tar.gz`: in `inspector.test.ts`, `listArchive` viene chiamato per `sftp://m/a.zip` e non per `sftp://m/a.tar.gz`.
- [ ] **Step 2:** `npm test` → FAIL. **Step 3:** implement. **Step 4:** `npm test` e `npx tsc --noEmit` → PASS.
- [ ] **Step 5: Commit** (dopo conferma): `feat(ui): viewer, editor e ispettore dentro gli archivi`

---

### Task 12: Documentazione e verifiche finali

**Files:**
- Modify: `README.md` (nuova sezione "Archivi (fase 3b)" con tabella dei tasti e limiti accettati; nella sezione della fase 2 il punto "Archivi: solo l'elenco…" rimanda alla nuova sezione)
- Create: `docs/superpowers/fase-3b-follow-up.md` (verifiche manuali e limiti noti)

- [ ] **Step 1:** README:
  - **Tasti:** Invio/Ctrl+PgDn, Alt+F5, Alt+F9, e F5/F6/F7/F8/Shift+F6/F4 dentro uno ZIP.
  - **Formati:** 7z, TAR e TAR.GZ in sola lettura.
  - **Limiti accettati:**
    - niente RAR, niente archivi annidati, niente archivi cifrati;
    - dizionario oltre 256 MiB rifiutato;
    - TAR.GZ remoto scaricato per intero;
    - modifica senza elevazione;
    - su FTP la sostituzione di uno ZIP elimina l'originale prima della rinomina finale.
- [ ] **Step 2:** follow-up, con le verifiche manuali da fare sulla build release:
  - ZIP creato da Explorer e da 7-Zip con nomi accentati;
  - 7z "Ultra" di 7-Zip (dizionario 64 MiB) aperto ed estratto;
  - ZIP di rilascio da 300 MB su un server Lepida: aperto, elencato, una voce estratta, un file aggiunto, con la verifica del tempo e del traffico;
  - Alt+F9 su un 7z solido grande, con la verifica del tempo;
  - F4 su una configurazione dentro uno ZIP remoto;
  - FTP senza sostituzione atomica: aggiunta di un file a uno ZIP.
- [ ] **Step 3:** `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets`, `npm test`, `npx tsc --noEmit` → tutti verdi, senza avvisi nuovi di clippy.
- [ ] **Step 4: Commit** (dopo conferma): `docs: archivi (fase 3b) nel README e follow-up delle verifiche`
