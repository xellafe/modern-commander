# Modern Commander — Fase 2 "Vedere": piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** vedere e modificare i file senza uscire dall'app: viewer F3 interno (testo con codifica, esadecimale, immagine, PDF, media, file di più GB letti a blocchi), editor F4 con Monaco (o editor esterno), ispettore Ctrl+Q, command palette Ctrl+Shift+P, salto rapido Ctrl+P, terminale a schede Ctrl+ò, vista miniature.

**Architecture:** il viewer e l'editor sono **schede di file** dentro i gruppi esistenti: `Tab` acquista un campo `file` opzionale e il suo `path` resta sempre la cartella, così copia, "+", albero e terminale continuano a ragionare su cartelle. Il contenuto dei file arriva alla UI solo dal backend: comandi a blocchi (`read_chunk`, risposta binaria) per testo ed esadecimale, asset protocol di Tauri (con richieste Range) per immagini, PDF e media. Le richieste lunghe (dimensione, hash, scansione, archivi) condividono un unico registro di annullamento nel backend. Monaco, pdf.js, xterm.js e markdown-it si caricano con `import()` dinamico al primo uso, per non toccare l'avvio.

**Tech Stack:** come la fase 1 (Tauri 2.12, Rust, React 19 + TypeScript, Zustand, i18next, TanStack Virtual, Vitest), più: `monaco-editor` (MIT), `pdfjs-dist` (Apache-2.0), `@xterm/xterm` + `@xterm/addon-fit` (MIT), `markdown-it` (MIT); crate `portable-pty` (MIT), `encoding_rs` (MIT/Apache + BSD-3), `zip` (MIT), `tar` e `flate2` (MIT/Apache), `sevenz-rust2` (Apache-2.0; `sevenz-rust` non è più mantenuto). Feature `protocol-asset` di `tauri`. Shiki, suggerito dallo spec per l'ispettore, non serve: il codice si colora con `monaco.editor.colorize`, già presente per l'editor.

**Spec:** `docs/superpowers/specs/2026-10-16-modern-commander-design.md`, §9 punto 2, con §5.5 (ispettore), §5.6 (terminale), §5.7 (palette e Ctrl+P), §6.3 (viewer ed editor; il confronto contenuti è della fase 4). In più la vista miniature `cm_SrcThumbs`, rimandata dal piano della fase 1 (Task 10).

## Global Constraints

Valgono tutte quelle del piano della fase 1 (`docs/superpowers/plans/2026-10-16-fase-1-fondamenta.md`), in particolare:
- Solo Windows 10/11 x64. Licenza MIT; dipendenze MIT-compatibili, niente GPL.
- Dipendenze solo con `npm install` / `cargo add`, mai versioni scritte a mano. Le API si leggono nella documentazione della versione installata (Monaco, pdf.js e xterm cambiano spesso tra major).
- **Commit solo con conferma esplicita dell'utente**: i passi "Commit" sono proposte da confermare. Niente push.
- Documentazione, commenti e messaggi di commit in italiano; identificatori in inglese.
- Nessuna stringa UI nel codice: chiavi i18n sempre in `en.json` e `it.json` (il test sulle chiavi resta).
- Il backend restituisce `AppError { code, path, detail }`; il frontend traduce `errors.<CODE>`.
- Scorciatoie: mappa di fabbrica di TC; una scorciatoia nuova non prende un tasto di TC; binding sul virtual key.
- Prestazioni: avvio < 1 s; tasto → cursore < 16 ms.

Nuovi per questa fase:
- **Caricamento pigro:** Monaco, pdf.js, xterm.js e markdown-it solo con `import()` dinamico. Il chunk principale di `npm run build` non li contiene.
- **Memoria limitata nel viewer:** un file non si carica mai intero. Una lettura è al massimo `MAX_CHUNK` = 4 MiB; il viewer di testo tiene una finestra di 256 KiB.
- **Lettura senza bloccare:** i file si aprono in lettura con condivisione `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`, così un log aperto da un altro programma si legge e chi lo scrive non si blocca.
- **Scrittura dell'editor mai sul posto:** si scrive su `<file>.mcpart` nella stessa cartella e poi si sostituisce con `ReplaceFileW` (che conserva attributi e ACL). La codifica si fa prima di toccare il disco.
- **Contenuti non fidati:** il Markdown si rende con `markdown-it` e `html: false`; pdf.js con `isEvalSupported: false`; nessun HTML dei file finisce nel DOM. I link nel Markdown non navigano.
- **Asset protocol:** `scope: ["**"]` serve file locali alla WebView. È coerente con §7.1 (lo serve il backend, il frontend non usa API di file system) e non dà più accesso di `read_chunk`; resta valida la regola sopra sui contenuti non fidati.
- I test frontend coprono la logica pura (Vitest in ambiente node, niente jsdom). Le parti di rendering si verificano a mano con i passi "Verifica manuale".

## Review Focus

1. **File di più GB o con una riga enorme** (log da 5 GB, JSON minificato da 200 MB su una riga, file di soli zeri): F3 si apre subito, Fine e il trascinamento della barra arrivano in fondo senza leggere tutto, la memoria del processo WebView resta stabile. → test in Task 3 (`splitLines` con riga lunga, `lineUp` dentro una riga lunga), verifica in Task 4.
2. **Salvataggio che perderebbe dati**: file cambiato da un altro programma dopo l'apertura, caratteri non rappresentabili nella codifica originale, file binario aperto con F4. Mai sovrascrivere in silenzio, mai lasciare un file a metà. → test in Task 6.
3. **Cursore veloce con l'ispettore aperto** (↓ tenuto premuto su mille file, cartelle enormi, rete lenta): la UI resta fluida, un'anteprima vecchia non sostituisce mai quella nuova, i calcoli in corso si annullano. → test in Task 11.
4. **File in uso** (log aperto in scrittura, file bloccato in esclusiva): viewer, ispettore e hash li leggono senza bloccare chi scrive; un blocco esclusivo dà `FILE_IN_USE` tradotto. → test in Task 1.
5. **Tasti nel posto sbagliato**: le lettere nel viewer non vanno alla ricerca rapida; Tab, Ctrl+Tab e Ctrl+W su un viewer agiscono su gruppi e schede; Ctrl+ò e Ctrl+Shift+P funzionano anche con il focus nel terminale o in Monaco; Ctrl+C, Tab e frecce nel terminale restano alla shell. → test in Task 2 e Task 13.

## Fuori perimetro

Non richiesti dallo spec per la fase 2; da proporre all'utente come aggiunte, non da fare di nascosto:
- Ricerca nel viewer (F7 del Lister di TC) e Shift+F4 (nuovo file).
- Diff con HEAD, stato git e tag nell'ispettore: fase 5. Le sezioni restano nascoste.
- Persistenza delle cartelle recenti di Ctrl+P: arriva con i workspace (fase 5); in fase 2 l'elenco vive per la sessione.
- Il follow-up "TOCTOU sulle giunzioni per le primitive elevate", segnato "fase 2" in `docs/superpowers/fase-1-follow-up.md`, tocca l'helper UAC e non il perimetro di §9.2: va in un piano a parte.

---

### Task 1: Backend: registro delle richieste e lettura dei file

**Files:**
- Create: `src-tauri/src/requests.rs`, `src-tauri/src/fs/content.rs`
- Modify: `src-tauri/src/fs/size.rs` (usa `Requests`; tolti `SizeJobs`, `Registered`, `cancel_dir_size`), `src-tauri/src/fs/mod.rs`, `src-tauri/src/error.rs`, `src-tauri/src/lib.rs`
- Modify: `src/ipc/api.ts`, `src/ipc/types.ts`, `src/state/sizeRequests.ts`, `src/i18n/en.json`, `src/i18n/it.json`

**Interfaces:**
- Consumes: `AppError`, `to_extended`, `blocking()` (fase 1)
- Produces (Rust):

```rust
// requests.rs: un solo registro gestito da Tauri per dimensione, hash, scansione e archivi
#[derive(Default, Clone)] pub struct Requests(/* Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> */);
impl Requests { pub fn register(&self, id: u64) -> RequestGuard; pub fn cancel(&self, id: u64); }
pub struct RequestGuard { /* Requests, id, flag */ }   // al drop toglie l'id dal registro
impl RequestGuard { pub fn flag(&self) -> &AtomicBool; }
#[tauri::command] pub fn cancel_request(requests: State<'_, Requests>, request_id: u64);

// error.rs: nuovi codici
ErrorCode::{ TooLarge, ChangedOnDisk, UnmappableChars, CorruptArchive }

// fs/content.rs
#[serde(rename_all = "camelCase")]
pub struct FileInfo { pub size: u64, pub is_dir: bool, pub created_ms: i64, pub modified_ms: i64, pub accessed_ms: i64, pub attributes: u32 }
pub const MAX_CHUNK: u32 = 4 << 20;
pub(crate) fn open_shared(path: &Path) -> std::io::Result<std::fs::File>;   // share 7, percorso esteso
pub fn file_info(path: &Path) -> Result<FileInfo, AppError>;
pub fn read_chunk(path: &Path, offset: u64, len: u32) -> Result<Vec<u8>, AppError>; // oltre la fine: vuoto; len > MAX_CHUNK: INVALID_REQUEST
pub fn read_whole(path: &Path, max: u64) -> Result<Vec<u8>, AppError>;              // size > max: TOO_LARGE
pub fn encoding_for_acp(acp: u32) -> &'static str;
// comandi: file_info(path) -> FileInfo; read_chunk(path, offset, len) -> tauri::ipc::Response;
//          read_file(path, max) -> tauri::ipc::Response; ansi_encoding() -> String
```

- Produces (TS, `api.ts`): `nextRequestId(): number` (contatore di modulo, unico per tutta l'app), `cancelRequest(requestId: number): Promise<void>`, `fileInfo(path): Promise<FileInfo>`, `readChunk(path, offset, len): Promise<Uint8Array>`, `readFile(path, max): Promise<Uint8Array>`, `ansiEncoding(): Promise<string>` (memorizzato dopo la prima chiamata). `types.ts`: `FileInfo`, i nuovi `ErrorCode`.

- [ ] **Step 1: Test** (in `content.rs` e `requests.rs`)

```rust
#[test] fn chunk_reads_range_and_stops_at_eof() {
    let d = tempfile::tempdir().unwrap(); let f = d.path().join("a.bin");
    std::fs::write(&f, b"0123456789").unwrap();
    assert_eq!(read_chunk(&f, 3, 4).unwrap(), b"3456");
    assert_eq!(read_chunk(&f, 8, 100).unwrap(), b"89");
    assert!(read_chunk(&f, 50, 10).unwrap().is_empty());
    assert_eq!(read_chunk(&f, 0, MAX_CHUNK + 1).unwrap_err().code, ErrorCode::InvalidRequest);
}
#[test] fn reads_a_file_another_program_is_writing() {
    use std::os::windows::fs::OpenOptionsExt;
    let d = tempfile::tempdir().unwrap(); let f = d.path().join("log.txt");
    std::fs::write(&f, b"riga").unwrap();
    let _writer = std::fs::OpenOptions::new().append(true).share_mode(0x1 | 0x2).open(&f).unwrap(); // READ|WRITE
    assert_eq!(read_chunk(&f, 0, 4).unwrap(), b"riga");
    drop(_writer);
    let _excl = std::fs::OpenOptions::new().read(true).share_mode(0).open(&f).unwrap();
    assert_eq!(read_chunk(&f, 0, 4).unwrap_err().code, ErrorCode::FileInUse);
}
#[test] fn whole_file_respects_the_limit() { /* 10 byte, max 9 → TOO_LARGE; max 10 → 10 byte */ }
#[test] fn info_has_times_and_attributes() { /* file normale: size, modified_ms > 0, created_ms > 0; cartella: is_dir */ }
#[test] fn long_path_chunk() { /* percorso > 260 caratteri come nel Task 3 della fase 1: read_chunk funziona */ }
#[test] fn acp_labels() {
    assert_eq!(encoding_for_acp(1252), "windows-1252"); assert_eq!(encoding_for_acp(1250), "windows-1250");
    assert_eq!(encoding_for_acp(932), "shift_jis"); assert_eq!(encoding_for_acp(65001), "utf-8");
    assert_eq!(encoding_for_acp(12345), "windows-1252");
}
#[test] fn guard_unregisters_and_cancel_sets_flag() {
    let r = Requests::default(); let g = r.register(7);
    r.cancel(7); assert!(g.flag().load(Ordering::Relaxed));
    drop(g); r.cancel(7); // id non più registrato: nessun effetto, nessun panic
}
```

- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml` → FAIL (simboli mancanti).
- [ ] **Step 3:** Implementare:
  - `encoding_for_acp`: 874 e 1250–1258 → `windows-<n>`; 932 `shift_jis`; 936 `gbk`; 949 `euc-kr`; 950 `big5`; 65001 `utf-8`; altrimenti `windows-1252`. `ansi_encoding` chiama `GetACP`.
  - `file_info` usa `std::fs::metadata` sul percorso esteso e `MetadataExt` (`creation_time`, `last_write_time`, `last_access_time`, `file_attributes`), FILETIME → epoch ms come in `listing.rs`.
  - `read_chunk` e `read_file` restituiscono `tauri::ipc::Response::new(bytes)`: al frontend arriva un `ArrayBuffer`.
  - `size.rs`: `calc_dir_size` prende `State<'_, Requests>`; il comportamento verso il frontend (evento `dir-size`) non cambia. `lib.rs`: `.manage(Requests::default())` al posto di `SizeJobs`, `cancel_request` al posto di `cancel_dir_size`.
  - `sizeRequests.ts` usa `nextRequestId` e `cancelRequest`.
  - i18n: `errors.TOO_LARGE`, `errors.CHANGED_ON_DISK`, `errors.UNMAPPABLE_CHARS`, `errors.CORRUPT_ARCHIVE`.
- [ ] **Step 4:** `cargo test` e `npm test` → PASS (i test di `dirSize.test.ts` e `sizeRequests.test.ts` vanno adeguati ai nuovi nomi, senza cambiare cosa verificano).
- [ ] **Step 5: Commit** `feat(fs): registro delle richieste annullabili e lettura dei file a blocchi`

### Task 2: Schede di file e contesto dei comandi

**Files:**
- Modify: `src/state/layout.ts`, `src/state/layout.test.ts`, `src/commands/registry.ts`, `src/commands/registry.test.ts`, `src/commands/useKeyboard.ts`, `src/commands/navigation.ts`, `src/commands/hotlist.ts`, `src/components/groups/TabStrip.tsx`, `src/components/groups/GroupsArea.tsx` (`useQuickSearch`), `src/components/groups/Group.tsx`, `src/components/sidebar/FolderTree.tsx`

**Interfaces:**
- Produces:

```ts
// layout.ts
interface FileRef { path: string; mode: 'viewer' | 'editor' }
interface Tab { id: string; path: string; file?: FileRef; returnTo?: string } // path = sempre una cartella (per i file: quella che li contiene)
openFileTab(s: LayoutState, groupId: string, file: FileRef): LayoutState
activeTab(s: LayoutState): Tab
// registry.ts
interface Command { /* … */ when: 'fileList' | 'group' | 'global' }
shouldExecute(cmd, opts: { repeat: boolean; focusInText: boolean; dirTab: boolean }): boolean
isRunnable(cmd: Command): boolean        // false per 'fileList' se la scheda attiva è un file
// navigation.ts
ensureDirTab(): string                   // id di una scheda cartella nel gruppo attivo, attivata
```

  Semantica:
  - `openFileTab` inserisce dopo la scheda attiva, la attiva, imposta `path = parentPath(file.path)` e `returnTo` = la scheda attiva di prima. Se nel gruppo c'è già una scheda con lo stesso file (confronto senza maiuscole) e lo stesso `mode`, attiva quella.
  - `closeTab` di una scheda attiva con `returnTo` ancora nello stesso gruppo attiva `returnTo`; altrimenti la regola della fase 1 (vicino a destra).
  - `'fileList'`: solo con focus fuori dai campi e scheda attiva di tipo cartella. `'group'`: focus fuori dai campi, qualunque scheda. `'global'`: sempre.
  - Diventano `'group'`: `cm_OpenNewTab`, `cm_CloseCurrentTab`, `cm_SwitchToNextTab`, `cm_SwitchToPreviousTab`, `mc_FocusNextGroup`, `mc_FocusPrevGroup`, `mc_SplitGroup`.
  - `executeCommand` non esegue un comando con `isRunnable` false: vale anche per la barra F-key e per la palette.
  - `ensureDirTab`: se la scheda attiva è un file, attiva `returnTo` se esiste nel gruppo, altrimenti apre una nuova scheda cartella su `tab.path`.

- [ ] **Step 1: Verifica delle scorciatoie nuove della fase 2** sulla documentazione ufficiale di TC (help "Keyboard" / `TOTALCMD.INC`), come nel Task 10 della fase 1: Ctrl+Q (`cm_SrcQuickview`), Ctrl+Shift+F1 (`cm_SrcThumbs`), e che siano libere Ctrl+P, Ctrl+Shift+P e Ctrl+Shift+Oem3. Se un tasto nuovo è occupato in TC, sceglierne un altro e aggiornare la tabella del Task 14 prima di proseguire.
- [ ] **Step 2: Test**

```ts
test('openFileTab opens after the active tab and remembers the opener', () => {
  let s = initialLayout(['C:\\a']); const g = s.groups[0].id; const opener = s.groups[0].activeTabId;
  s = openFileTab(s, g, { path: 'C:\\a\\f.txt', mode: 'viewer' });
  const t = activeTab(s);
  expect(t).toMatchObject({ path: 'C:\\a', file: { path: 'C:\\a\\f.txt', mode: 'viewer' }, returnTo: opener });
  expect(s.groups[0].tabIds).toEqual([opener, t.id]);
});
test('closing a file tab goes back to its opener', () => {
  let s = initialLayout(['C:\\a']); const g = s.groups[0].id; const a = s.groups[0].activeTabId;
  s = newTab(s, g, 'C:\\b');                                              // [a, b*]
  s = activateTab(s, a);                                                  // [a*, b]
  s = openFileTab(s, g, { path: 'C:\\a\\f', mode: 'viewer' });            // [a, f*, b]
  s = closeTab(s, activeTab(s).id);
  expect(activeTab(s).id).toBe(a);                                        // senza returnTo sarebbe b
});
test('same file and mode reuses the tab, another mode opens a new one', () => {
  let s = initialLayout(['C:\\a']); const g = s.groups[0].id;
  s = openFileTab(s, g, { path: 'C:\\a\\F.txt', mode: 'viewer' }); const v = activeTab(s).id;
  s = activateTab(s, s.groups[0].tabIds[0]);
  s = openFileTab(s, g, { path: 'c:\\a\\f.txt', mode: 'viewer' });
  expect(activeTab(s).id).toBe(v);
  s = openFileTab(s, g, { path: 'C:\\a\\F.txt', mode: 'editor' });
  expect(s.groups[0].tabIds).toHaveLength(3);
});
test('command context', () => {
  const c = (when: Command['when']) => ({ id: 'x', titleKey: 'x', when, run() {} });
  const o = { repeat: false, focusInText: false, dirTab: false };
  expect(shouldExecute(c('fileList'), o)).toBe(false);
  expect(shouldExecute(c('group'), o)).toBe(true);
  expect(shouldExecute(c('group'), { ...o, focusInText: true })).toBe(false);
  expect(shouldExecute(c('global'), { ...o, focusInText: true })).toBe(true);
  expect(shouldExecute(c('fileList'), { ...o, dirTab: true })).toBe(true);
});
```

- [ ] **Step 3:** `npm test` → FAIL.
- [ ] **Step 4:** Implementare. In più:
  - `useKeyboard` passa `dirTab: !activeTab(useLayout.getState()).file`.
  - `useQuickSearch` (GroupsArea) esce subito se la scheda attiva è un file: senza questo controllo le lettere del viewer finirebbero alla ricerca rapida, che le consuma in cattura.
  - `Group.tsx` non chiama `load` per le schede di file. Il corpo per viewer ed editor arriva nei Task 4 e 6; fino ad allora una scheda di file mostra solo la barra delle schede.
  - `TabStrip`: per le schede di file l'etichetta è il nome del file con un'icona (`Eye` per il viewer, `Pencil` per l'editor); il `title` è il percorso del file.
  - `FolderTree` e la hotlist navigano su `ensureDirTab()` invece che sulla scheda attiva.
- [ ] **Step 5:** `npm test` → PASS.
- [ ] **Step 6: Commit** `feat(state): schede di file nei gruppi e contesto dei comandi`

### Task 3: Funzioni pure per il contenuto dei file

**Files:**
- Create: `src/lib/preview.ts`, `src/lib/encoding.ts`, `src/lib/textPaging.ts`, `src/lib/hex.ts` e i rispettivi `*.test.ts`

**Interfaces:**
- Consumes: `fileKind` di `src/lib/format.ts` (fase 1; per pdf e archivi)
- Produces:

```ts
// preview.ts
type PreviewKind = 'image' | 'pdf' | 'video' | 'audio' | 'markdown' | 'archive'
previewKind(name: string): PreviewKind | null
looksBinary(sample: Uint8Array): boolean       // c'è un byte 0 e non c'è un BOM UTF-16
// encoding.ts
interface Detected { encoding: string; bom: number }   // bom = lunghezza in byte del BOM (0, 2, 3)
detectEncoding(sample: Uint8Array, ansi: string, complete: boolean): Detected   // complete = il campione è il file intero
codeUnit(encoding: string): 1 | 2                       // 2 solo per utf-16le/utf-16be
ENCODINGS: string[]   // etichette WHATWG proposte nel viewer
// textPaging.ts
MAX_LINE = 4096       // byte; una riga più lunga si spezza in righe da MAX_LINE
splitLines(chunk: Uint8Array, base: number, unit: 1 | 2, bigEndian: boolean): number[]
lineUp(before: Uint8Array, base: number, top: number, unit: 1 | 2, bigEndian: boolean): number
seekLine(chunk: Uint8Array, offset: number, unit: 1 | 2, bigEndian: boolean): number
// hex.ts
offsetDigits(size: number): number
hexRow(bytes: Uint8Array, offset: number, digits: number): { offset: string; hex: string; ascii: string }
```

  Valori e regole:
  - Immagini che la WebView mostra da sola: `png jpg jpeg gif webp bmp ico svg avif`. Video: `mp4 m4v webm mov ogv`. Audio: `mp3 m4a aac wav ogg oga opus flac`. Markdown: `md markdown`. Archivi: `zip 7z tar tgz` e i nomi che finiscono in `.tar.gz`; `x.gz` da solo non è un archivio.
  - `ENCODINGS`: `utf-8`, `utf-16le`, `utf-16be`, `windows-1250`…`windows-1258`, `iso-8859-2`, `iso-8859-15`, `koi8-r`, `shift_jis`, `gbk`, `big5`, `euc-kr`. Le decodifica `TextDecoder`: nessuna tabella scritta a mano. Le codifiche DOS (437/850) non sono WHATWG e restano fuori.
  - `detectEncoding`: BOM `EF BB BF` → utf-8 (3); `FF FE` → utf-16le (2); `FE FF` → utf-16be (2). Senza BOM: se il campione decodifica come UTF-8 con `TextDecoder('utf-8', { fatal: true })` → utf-8; altrimenti `ansi`. Con `complete` falso si decodifica con `stream: true`, così un campione tagliato a metà di una sequenza non conta come errore; con `complete` vero una sequenza incompleta in fondo è un errore (l'editor decodifica il file intero e non deve perdere l'ultimo byte).
  - `splitLines` restituisce l'offset assoluto di ogni inizio riga nel blocco, a partire da `base`, solo quelli minori di `base + chunk.length`. Una riga finisce dopo LF (`0A`, oppure l'unità `0A 00` / `00 0A` in UTF-16) o dopo `MAX_LINE` byte. Gli offset sono sempre multipli di `unit`.
  - `lineUp`: l'inizio della riga sopra `top`, cercando nel blocco `before` che va da `base` a `top`. Se la riga sopra è più lunga di `MAX_LINE`, restituisce l'ultimo spezzone che inizia prima di `top`. Se nel blocco non c'è un LF e `base > 0`, restituisce `max(base, top - MAX_LINE)`.
  - `seekLine`: per il salto con la barra di scorrimento. Il primo inizio riga ≥ `offset` nel blocco che parte da `offset` (subito dopo il primo LF); se non c'è un LF entro `MAX_LINE`, `offset` allineato a `unit`. `offset` 0 resta 0.
  - `hexRow`: 16 byte per riga, maiuscolo, uno spazio in più dopo l'ottavo byte; i byte mancanti dell'ultima riga sono spazi, così le colonne restano allineate. ASCII stampabile 0x20–0x7E, il resto `.`.

- [ ] **Step 1: Test**

```ts
const enc = (s: string) => new TextEncoder().encode(s);
test('preview kinds', () => {
  expect(previewKind('a.PNG')).toBe('image'); expect(previewKind('a.tif')).toBeNull();
  expect(previewKind('a.tar.gz')).toBe('archive'); expect(previewKind('a.gz')).toBeNull();
  expect(previewKind('r.md')).toBe('markdown'); expect(previewKind('a.pdf')).toBe('pdf');
});
test('binary detection', () => {
  expect(looksBinary(new Uint8Array([0x41, 0, 0x42]))).toBe(true);
  expect(looksBinary(new Uint8Array([0xff, 0xfe, 0x41, 0]))).toBe(false);
  expect(looksBinary(enc('testo'))).toBe(false);
});
test('encoding detection', () => {
  const W = 'windows-1252';
  expect(detectEncoding(new Uint8Array([0xef, 0xbb, 0xbf, 0x41]), W, true)).toEqual({ encoding: 'utf-8', bom: 3 });
  expect(detectEncoding(new Uint8Array([0xfe, 0xff, 0, 0x41]), W, true)).toEqual({ encoding: 'utf-16be', bom: 2 });
  expect(detectEncoding(enc('perché'), W, true).encoding).toBe('utf-8');
  expect(detectEncoding(new Uint8Array([0x70, 0xe8, 0x20]), W, false).encoding).toBe(W);   // "pè " in ANSI
  expect(detectEncoding(enc('perché').slice(0, 6), W, false).encoding).toBe('utf-8');      // é tagliata dal campione
  expect(detectEncoding(new Uint8Array([0x70, 0xe8]), W, true).encoding).toBe(W);          // file intero che finisce con "è" ANSI
});
test('splitLines', () => {
  expect(splitLines(enc('a\nbb\n\nc'), 0, 1, false)).toEqual([0, 2, 5, 6]);
  expect(splitLines(enc('x\ny'), 100, 1, false)).toEqual([100, 102]);
  expect(splitLines(new Uint8Array([0x61, 0, 0x0a, 0, 0x62, 0]), 0, 2, false)).toEqual([0, 4]);
  expect(splitLines(new Uint8Array(10000).fill(0x78), 0, 1, false)).toEqual([0, 4096, 8192]);
});
test('lineUp', () => {
  const t = enc('aa\nbbb\ncc');
  expect(lineUp(t.slice(0, 7), 0, 7, 1, false)).toBe(3);
  expect(lineUp(t.slice(0, 3), 0, 3, 1, false)).toBe(0);
  expect(lineUp(new Uint8Array(0), 0, 0, 1, false)).toBe(0);
  expect(lineUp(new Uint8Array(8192).fill(0x78), 0, 8192, 1, false)).toBe(4096);
  expect(lineUp(new Uint8Array(65536).fill(0x78), 1000, 66536, 1, false)).toBe(66536 - 4096);
});
test('seekLine', () => {
  expect(seekLine(enc('zz\nnext'), 50, 1, false)).toBe(53);
  expect(seekLine(new Uint8Array(5000).fill(0x78), 51, 2, false)).toBe(50);
  expect(seekLine(enc('abc'), 0, 1, false)).toBe(0);
});
test('hex rows', () => {
  const full = hexRow(Uint8Array.from({ length: 16 }, (_, i) => i), 0, 8);
  expect(full.hex).toBe('00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F');
  const r = hexRow(new Uint8Array([0x41, 0x00, 0xff]), 16, 8);
  expect(r).toMatchObject({ offset: '00000010', ascii: 'A..' });
  expect(r.hex.trimEnd()).toBe('41 00 FF'); expect(r.hex).toHaveLength(full.hex.length);
  expect(offsetDigits(1000)).toBe(8); expect(offsetDigits(2 ** 36)).toBe(10);
});
```

- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implementare.
- [ ] **Step 4:** `npm test` → PASS.
- [ ] **Step 5: Commit** `feat(lib): tipo di anteprima, codifiche, righe a blocchi ed esadecimale`

### Task 4: Viewer F3: testo ed esadecimale

**Files:**
- Create: `src/state/viewers.ts`, `src/state/viewers.test.ts`, `src/components/viewer/Viewer.tsx`, `src/components/viewer/TextPane.tsx`, `src/components/viewer/HexPane.tsx`, `src/components/viewer/ScrollRail.tsx`, `src/components/viewer/viewer.css`, `scripts/make-big-files.ps1`
- Modify: `src/components/groups/Group.tsx`, `src/commands/shell.ts`, `src/commands/shell.test.ts`, `src/commands/navigation.ts` (`closeTab` chiama `useViewers.dispose`), i18n

**Interfaces:**
- Consumes: Task 1 `fileInfo`/`readChunk`/`ansiEncoding`; Task 2 `openFileTab`, `closeTab`; Task 3 tutto
- Produces:

```ts
type ViewerMode = 'text' | 'hex' | 'image' | 'pdf' | 'video' | 'audio'
interface ViewerState { mode: ViewerMode; encoding: string; wrap: boolean; top: number }   // top = offset del primo byte visibile
initialViewerState(name: string, sample: Uint8Array, ansi: string, complete: boolean): ViewerState
// previewKind image/pdf/video/audio → quella modalità; markdown, archivi e il resto → looksBinary ? 'hex' : 'text'
useViewers: { states: Record<string, ViewerState>; set(tabId: string, patch: Partial<ViewerState>): void; dispose(tabId: string): void }
<Viewer tabId={string} path={string} />
<ScrollRail fraction={number} onSeek={(fraction: number) => void} />
```

  Lo stato sta nello store e non nel componente: cambiando scheda il corpo del gruppo si smonta, e al ritorno il viewer deve riapparire allo stesso punto.

- [ ] **Step 1: Test**

```ts
test('initial mode and encoding', () => {
  const txt = new TextEncoder().encode('ciao'); const W = 'windows-1252';
  expect(initialViewerState('a.png', txt, W, true).mode).toBe('image');
  expect(initialViewerState('a.dat', new Uint8Array([1, 0, 2]), W, true).mode).toBe('hex');
  expect(initialViewerState('a.txt', new Uint8Array([0xff, 0xfe, 0x61, 0]), W, true))
    .toEqual({ mode: 'text', encoding: 'utf-16le', wrap: false, top: 2 });   // il BOM non si mostra
  expect(initialViewerState('a.md', txt, W, true).mode).toBe('text');         // il Markdown renderizzato è dell'ispettore
});
```

  In `shell.test.ts`: `cm_List` con il cursore su un file chiama `openFileTab` con `mode: 'viewer'`; su una cartella o su `..` non fa niente. I test su `isExecutable` si tolgono insieme alla funzione: con il viewer interno F3 non apre più programmi esterni.
- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implementare `viewers.ts` e il cambio di `cm_List` in `shell.ts` (`EXECUTABLE` e `isExecutable` si eliminano).
- [ ] **Step 4:** `npm test` → PASS.
- [ ] **Step 5:** Componenti:
  - `Viewer`: all'apertura `fileInfo` + `readChunk(path, 0, 65536)` come campione + `ansiEncoding()`, poi `initialViewerState` (`complete` = file non più grande del campione) se lo store non ha già uno stato per la scheda. Barra in alto: nome, dimensione, selettore modalità, selettore codifica (`ENCODINGS`), interruttore a capo, posizione in %. `Group.tsx` mostra `<Viewer>` al posto di breadcrumb e lista quando `tab.file?.mode === 'viewer'`. Le modalità `image`/`pdf`/`video`/`audio` mostrano un segnaposto fino al Task 5.
  - Tasti (sul contenitore del viewer, `tabIndex={0}` e focus all'apertura; `stopPropagation` sui tasti gestiti, gli altri salgono a `useKeyboard`): Esc chiude la scheda; `1` testo, `3` esadecimale, `4` vista nativa (solo se `previewKind` la prevede), come il Lister di TC; `A` ANSI, `7` UTF-8, `6` UTF-16LE; `W` a capo; ↑ ↓ PgSu PgGiù Home Fine, Ctrl+Home e Ctrl+Fine. Rotella: 3 righe.
  - `TextPane`: legge 256 KiB da `top`, `splitLines`, decodifica con `TextDecoder(encoding)` solo le righe visibili più una. Giù di n righe: `splitLines(...)[n]`. Su: `readChunk(top - 65536, 65536)` e `lineUp` ripetuto. Fine: blocco finale di 64 KiB e `lineUp` dall'ultimo byte per tante righe quante ne entrano. Con l'a capo attivo (`white-space: pre-wrap`) lo scorrimento resta per righe logiche. Ogni lettura ha un numero di sequenza e le risposte vecchie si scartano.
  - `HexPane`: riga = 16 byte, `top` sempre multiplo di 16; legge solo le righe visibili; colonne offset (`offsetDigits(size)`), hex e ASCII.
  - `ScrollRail`: barra verticale propria. Una lista virtualizzata del browser non regge: 1 GB a 16 byte per riga supera l'altezza massima di un elemento. La maniglia sta a `top / size`; click e trascinamento chiamano `onSeek`. Il testo risolve la posizione con `readChunk(offset, 65536)` + `seekLine`, l'esadecimale con `floor(fraction * righe) * 16`.
  - Font monospazio dei token, righe alte `var(--mc-row-height)`, tab a 8.
- [ ] **Step 6:** `scripts/make-big-files.ps1 -Path D:\mc-test` crea `big.log` (5 GB di righe numerate, scritto a blocchi), `one-line.json` (200 MB senza a capo) e `zeros.bin` (`fsutil file createnew`, 5 GB).
- [ ] **Step 7: Verifica manuale:**
  - F3 su `big.log`: si apre in meno di 300 ms; Fine mostra l'ultima riga; trascinando la barra a metà si vede una riga intera vicino a 2,5 GB; ↑ e ↓ in fondo e in cima non saltano righe.
  - `one-line.json`: lo scorrimento procede a spezzoni da 4096 byte, senza blocchi della UI.
  - `zeros.bin`: si apre in esadecimale; Ctrl+Fine mostra l'ultimo offset.
  - La memoria del processo WebView2 (Gestione attività) resta sotto 300 MB durante tutte le prove.
  - Un file UTF-16 con BOM e uno ANSI con lettere accentate si leggono bene; `A`/`7`/`6` cambiano la codifica.
  - Le lettere nel viewer non aprono la ricerca rapida; Tab passa all'altro gruppo; Ctrl+W ed Esc chiudono e tornano alla lista sul file di partenza.
- [ ] **Step 8: Commit** `feat(viewer): viewer F3 per testo ed esadecimale con lettura a blocchi`

### Task 5: Viewer: immagini, PDF e media

**Files:**
- Modify: `src-tauri/Cargo.toml` (`cargo add tauri --features protocol-asset`), `src-tauri/tauri.conf.json` (`app.security.assetProtocol: { "enable": true, "scope": ["**"] }`), `package.json` (`npm install pdfjs-dist`)
- Create: `src/components/viewer/ImagePane.tsx`, `src/components/viewer/PdfPane.tsx`, `src/components/viewer/MediaPane.tsx`
- Modify: `src/components/viewer/Viewer.tsx`, i18n

**Interfaces:**
- Consumes: `convertFileSrc` di `@tauri-apps/api/core`; Task 4 `Viewer`
- Produces (riusati dall'ispettore nel Task 11): `<ImagePane path={string} />`, `<PdfPane path={string} />`, `<MediaPane path={string} kind={'video' | 'audio'} autoPlay={boolean} />`

- [ ] **Step 1:** Abilitare l'asset protocol (feature Cargo + configurazione). Verificare in DevTools che `fetch(convertFileSrc('C:\\Windows\\win.ini'))` risponda 200 e che un `fetch` con header `Range: bytes=0-9` risponda 206 con 10 byte. Provare anche un percorso UNC, un nome con `#`, `%` e spazi, e un percorso oltre 260 caratteri. Se uno dei casi fallisce, annotarlo in `docs/superpowers/fase-2-follow-up.md`: per quei file restano le modalità testo ed esadecimale.
- [ ] **Step 2:** `ImagePane`: `<img src={convertFileSrc(path)}>`, adattata al riquadro (default) o 1:1 al click. Su errore di caricamento mostra `viewer.unsupportedImage` (dal Task 9 passa alla miniatura della shell).
- [ ] **Step 3:** `PdfPane`: `import('pdfjs-dist')` dinamico; worker impostato come dice la documentazione della versione installata per i bundler (in Vite, con un URL `?url` o `new URL(..., import.meta.url)`); `getDocument({ url: convertFileSrc(path), isEvalSupported: false })`. Le pagine si disegnano su canvas solo quando entrano nella vista (IntersectionObserver), scalate alla larghezza del riquadro. Niente livello di testo: la selezione e la ricerca nel PDF non sono richieste.
- [ ] **Step 4:** `MediaPane`: `<video controls>` / `<audio controls>`. Sull'evento `error` mostra `viewer.unsupportedMedia` con il pulsante "Apri con il programma predefinito" (`openPath`).
- [ ] **Step 5:** `Viewer` mostra il riquadro della modalità `image`/`pdf`/`video`/`audio`; `1` e `3` restano disponibili per vedere lo stesso file come testo o esadecimale.
- [ ] **Step 6: Verifica manuale:**
  - PNG, JPEG, SVG e WebP si vedono; un TIFF mostra l'avviso.
  - Un PDF di 300 pagine si apre sulla prima pagina in meno di 1 s e scorre senza blocchi.
  - In un MP4 da 2 GB ci si sposta a metà subito e la memoria non cresce della dimensione del file (richieste Range).
  - Un MKV mostra l'avviso con l'apertura esterna.
  - `npm run build`: pdf.js sta in un chunk separato, non nel chunk principale.
- [ ] **Step 7: Commit** `feat(viewer): immagini, PDF e media nel viewer`

### Task 6: Editor F4 con Monaco

**Files:**
- Modify: `src-tauri/src/fs/content.rs`, `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml` (`cargo add encoding_rs`; feature `windows` per `ReplaceFileW`, già in `Win32_Storage_FileSystem`)
- Create: `src/lib/monaco.ts`, `src/lib/monaco.test.ts`, `src/state/editors.ts`, `src/state/editors.test.ts`, `src/components/editor/Editor.tsx`, `src/components/editor/editor.css`, `src/components/dialogs/UnsavedDialog.tsx`, `src/components/dialogs/SaveErrorDialog.tsx`
- Modify: `package.json` (`npm install monaco-editor`), `src/commands/shell.ts` (`cm_Edit`), `src/commands/navigation.ts` (`closeTab`, `initCloseGuard`), `src/state/ui.ts`, `src/components/dialogs/Dialog.tsx`, `src/components/dialogs/ExitConfirm.tsx`, `src/components/groups/Group.tsx`, `src/state/settings.ts`, `src/state/settings.test.ts`, `src/components/settings/SettingsPage.tsx`, `src/ipc/api.ts`, i18n

**Interfaces:**
- Consumes: Task 1 `readFile`, `fileInfo`, `ansiEncoding`, `FileInfo`; Task 2 `openFileTab`, `closeTab`; Task 3 `detectEncoding`, `looksBinary`
- Produces (Rust):

```rust
pub fn encode_text(text: &str, encoding: &str, bom: bool) -> Result<Vec<u8>, AppError>;
pub fn write_text_file(path: &Path, text: &str, encoding: &str, bom: bool, expected_modified_ms: Option<i64>) -> Result<FileInfo, AppError>;
// comando write_text_file(path, text, encoding, bom, expected_modified_ms) -> FileInfo
```

- Produces (TS):

```ts
writeTextFile(path: string, text: string, encoding: string, bom: boolean, expectedModifiedMs: number | null): Promise<FileInfo>
EDITOR_MAX_BYTES = 32 * 1024 * 1024
decodeForEditor(bytes: Uint8Array, ansi: string): { text: string; encoding: string; bom: boolean; readOnly: boolean }
interface EditorDoc { path: string; encoding: string; bom: boolean; modifiedMs: number; dirty: boolean; readOnly: boolean }
useEditors: { docs: Record<string, EditorDoc>; open(tabId: string, path: string): Promise<void>;
  setDirty(tabId: string, dirty: boolean): void; save(tabId: string, opts?: { force?: boolean; encoding?: string }): Promise<void>; dispose(tabId: string): void }
dirtyEditorCount(): number
loadMonaco(): Promise<typeof import('monaco-editor')>
languageFor(name: string, languages: { id: string; extensions?: string[] }[]): string | null
// settings.ts
Settings.useExternalEditor: boolean   // default false
// ui.ts: DialogRequest in più
| { kind: 'unsaved'; tabId: string }
| { kind: 'saveError'; tabId: string; code: 'CHANGED_ON_DISK' | 'UNMAPPABLE_CHARS' }
| { kind: 'exit'; count: number; dirty: number }      // sostituisce { kind: 'exit'; count }
```

  Regole:
  - `encode_text`: etichetta WHATWG risolta con `encoding_rs::Encoding::for_label`; etichetta sconosciuta → `INVALID_REQUEST`. UTF-16LE/BE si codificano a mano con `str::encode_utf16`, perché `encoding_rs` codifica UTF-16 come UTF-8 (regola WHATWG). Con `bom` si antepone il BOM della codifica. Se ci sono caratteri non rappresentabili (`had_unmappable`) → `UNMAPPABLE_CHARS`, e non si scrive niente.
  - `write_text_file`: con `expected_modified_ms` = `Some(t)` e data attuale diversa da `t` (o file sparito) → `CHANGED_ON_DISK`, senza scrivere. Poi codifica, scrittura su `<file>.mcpart` nella stessa cartella e `ReplaceFileW(file, part)`, che conserva attributi e ACL; con un file non esistente `MoveFileExW`. In caso di errore il `.mcpart` si elimina. Restituisce il `FileInfo` dopo la scrittura.
  - `decodeForEditor`: `detectEncoding(bytes, ansi, true)`, testo senza BOM; `readOnly = looksBinary(bytes)`, perché salvare un binario decodificato lo corromperebbe.
  - `dirty` si calcola confrontando `model.getAlternativeVersionId()` con la versione dell'ultimo salvataggio: un undo fino al punto salvato torna pulito.
  - Monaco normalizza gli a capo misti a quello prevalente: è un limite accettato, da scrivere nel README.

- [ ] **Step 1: Test** (Rust in `content.rs`)

```rust
#[test] fn encodes_supported_charsets() {
    assert_eq!(encode_text("€", "windows-1252", false).unwrap(), [0x80]);
    assert_eq!(encode_text("a", "utf-16le", true).unwrap(), [0xff, 0xfe, 0x61, 0x00]);
    assert_eq!(encode_text("a", "utf-16be", false).unwrap(), [0x00, 0x61]);
    assert_eq!(encode_text("a", "utf-8", true).unwrap(), [0xef, 0xbb, 0xbf, 0x61]);
    assert_eq!(encode_text("ł", "windows-1252", false).unwrap_err().code, ErrorCode::UnmappableChars);
    assert_eq!(encode_text("a", "klingon", false).unwrap_err().code, ErrorCode::InvalidRequest);
}
#[test] fn stale_write_is_refused_and_leaves_the_file_alone() {
    /* file "v1"; info = file_info; un altro "programma" riscrive "v2" e sposta la data di 2 s (set_modified);
       write_text_file(.., Some(info.modified_ms)) → CHANGED_ON_DISK; il file contiene "v2"; nessun .mcpart */
}
#[test] fn unmappable_text_never_touches_the_disk() { /* "ł" in windows-1252 → errore; file e cartella invariati */ }
#[test] fn write_replaces_content_and_keeps_attributes() {
    /* file con attributo hidden; write "nuovo" con la data attesa giusta → contenuto nuovo, hidden ancora presente,
       FileInfo.modified_ms aggiornato, nessun .mcpart */
}
```

  TS:

```ts
test('decodeForEditor', () => {
  expect(decodeForEditor(new Uint8Array([0xef, 0xbb, 0xbf, 0x61]), 'windows-1252')).toEqual({ text: 'a', encoding: 'utf-8', bom: true, readOnly: false });
  expect(decodeForEditor(new Uint8Array([0x70, 0xe8]), 'windows-1252')).toMatchObject({ text: 'pè', encoding: 'windows-1252' });
  expect(decodeForEditor(new Uint8Array([1, 0, 2]), 'windows-1252').readOnly).toBe(true);
});
test('languageFor', () => {
  const langs = [{ id: 'typescript', extensions: ['.ts', '.tsx'] }, { id: 'markdown', extensions: ['.md'] }];
  expect(languageFor('A.TSX', langs)).toBe('typescript'); expect(languageFor('a.xyz', langs)).toBeNull();
});
```

  In `settings.test.ts` il default diventa `useExternalEditor: false`.
- [ ] **Step 2:** `cargo test` e `npm test` → FAIL.
- [ ] **Step 3:** Implementare backend, `editors.ts` e `monaco.ts`:
  - `loadMonaco` importa `monaco-editor` una volta sola e imposta `self.MonacoEnvironment.getWorker` con i worker per Vite, come dice la documentazione della versione installata (editor, json, css, html, typescript). Definisce i temi `mc-dark` e `mc-light` dai token `--mc-*` e li applica al cambio di `data-theme`.
  - `open`: `readFile(path, EDITOR_MAX_BYTES)` (su `TOO_LARGE` notice con il suggerimento di usare F3), `fileInfo`, `decodeForEditor`; i modelli Monaco stanno in una `Map<tabId, ITextModel>` fuori dallo store.
  - `save`: `writeTextFile(path, text, opts.encoding ?? doc.encoding, bom, opts.force ? null : doc.modifiedMs)`. Su `CHANGED_ON_DISK` o `UNMAPPABLE_CHARS` apre `saveError`; con gli altri errori mostra la notice.
- [ ] **Step 4:** `cargo test` e `npm test` → PASS.
- [ ] **Step 5:** UI e comandi:
  - `Editor`: crea l'istanza Monaco al montaggio e la distrugge allo smontaggio (cambio scheda), salvando e ripristinando `saveViewState`; il modello resta. Opzioni: font monospazio dei token, minimappa, `readOnly` da `doc.readOnly` (con una barra che lo spiega). Comandi Monaco: Ctrl+S salva, Ctrl+W chiude la scheda passando dal controllo delle modifiche. Lo stato modificato si mostra come pallino nella scheda (TabStrip).
  - `cm_Edit`: con `useExternalEditor` usa `openWith(externalEditor, path)` come nella fase 1; altrimenti `openFileTab(..., { mode: 'editor' })`.
  - `closeTab` (navigation.ts): un editor modificato apre `unsaved` (Salva / Non salvare / Annulla) invece di chiudere; alla chiusura effettiva `useEditors.dispose` libera il modello.
  - `initCloseGuard`: conta anche `dirtyEditorCount()`; `ExitConfirm` cita job e file non salvati.
  - `SaveErrorDialog`: con `CHANGED_ON_DISK` offre Sovrascrivi (`force`) e Annulla; con `UNMAPPABLE_CHARS` offre "Salva in UTF-8" e Annulla.
  - Impostazioni, categoria editor: interruttore `useExternalEditor` accanto al campo `externalEditor`.
- [ ] **Step 6: Verifica manuale:**
  - F4 su un `.ts` evidenzia la sintassi; Ctrl+S salva; la data cambia nella lista (watcher).
  - Modificando il file da Notepad e poi salvando da qui compare il conflitto.
  - Scrivendo "ł" in un file ANSI e salvando compare l'offerta UTF-8.
  - Chiudendo una scheda modificata compare la richiesta; Alt+F4 con un editor modificato chiede conferma.
  - F4 su un `.exe` apre in sola lettura.
  - Con l'opzione esterna attiva F4 apre Notepad.
  - Ctrl+Shift+P e Ctrl+ò funzionano con il focus in Monaco (dopo i Task 7 e 13).
  - `npm run build`: Monaco sta in chunk separati.
- [ ] **Step 7: Commit** `feat(editor): editor F4 con Monaco, salvataggio sicuro ed editor esterno opzionale`

### Task 7: Ricerca fuzzy e command palette

**Files:**
- Create: `src/lib/fuzzy.ts`, `src/lib/fuzzy.test.ts`, `src/state/palette.ts`, `src/state/palette.test.ts`, `src/components/palette/Palette.tsx`, `src/components/palette/palette.css`, `src/commands/palette.ts`
- Modify: `src/commands/tcKeymap.ts`, `src/components/shell/TitleBar.tsx`, `src/App.tsx`, `src/main.tsx`, i18n

**Interfaces:**
- Consumes: Task 2 `isRunnable`; fase 1 `getCommands`, `executeCommand`, `chordsOf`, `chordParts`, `useKeybindings`
- Produces:

```ts
interface FuzzyMatch { score: number; positions: number[] }
fuzzyMatch(query: string, text: string): FuzzyMatch | null
fuzzyFilter<T>(items: T[], query: string, key: (t: T) => string, limit: number): { item: T; match: FuzzyMatch }[]
type PaletteMode = 'commands' | 'jump'
usePalette: { mode: PaletteMode | null; open(mode: PaletteMode): void; close(): void }
interface PaletteItem { id: string; label: string; detail?: string; keys?: string[][]; run(): void | Promise<void> }
paletteCommands(commands: Command[], keymap: Map<string, string>, t: (key: string) => string): PaletteItem[]
// comando mc_CommandPalette (global), Ctrl+Shift+P
```

  Regole:
  - `fuzzyMatch`: sottosequenza senza distinzione di maiuscole. Prima prova a far cadere ogni carattere su un inizio di parola (inizio del testo, dopo `\ / _ - .` o spazio, maiuscola dopo minuscola), poi ripiega sulla prima occorrenza utile. Punteggio: +1 per carattere, +8 per inizio di parola, +5 per carattere consecutivo al precedente, +10 se il primo carattere è in posizione 0.
  - `fuzzyFilter`: punteggio decrescente, a parità l'ordine di ingresso; query vuota → i primi `limit` elementi invariati.
  - `paletteCommands`: solo comandi con `isRunnable` vero, escluso `mc_CommandPalette`; `keys` = `chordsOf(keymap, id).map(chordParts)`.

- [ ] **Step 1: Test**

```ts
test('fuzzyMatch', () => {
  expect(fuzzyMatch('cpy', 'Copy')!.positions).toEqual([0, 2, 3]);
  expect(fuzzyMatch('xyz', 'Copy')).toBeNull();
  expect(fuzzyMatch('fl', 'FileList.tsx')!.positions).toEqual([0, 4]);
});
test('fuzzyFilter ranks prefixes and word starts first', () => {
  expect(fuzzyFilter(['resources', 'my_src_x', 'src'], 'src', (s) => s, 10).map((r) => r.item)).toEqual(['src', 'my_src_x', 'resources']);
  expect(fuzzyFilter(['b', 'a'], '', (s) => s, 1).map((r) => r.item)).toEqual(['b']);
});
test('paletteCommands hides itself and shows chords', () => {
  const cmds = [cmd('cm_Copy', 'fileList'), cmd('mc_CommandPalette', 'global')];
  const items = paletteCommands(cmds, new Map([['F5', 'cm_Copy']]), (k) => k);
  expect(items.map((i) => i.id)).toEqual(['cm_Copy']); expect(items[0].keys).toEqual([['F5']]);
});
```

  (`cmd` è un helper locale; con una scheda cartella attiva nel layout di test `isRunnable` è vero.)
- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** UI:
  - `Palette`: riquadro in alto al centro come in VS Code, `role="dialog"` (così valgono solo i comandi globali), campo con focus, al massimo 50 righe con i caratteri trovati in evidenza (`positions`) e i tasti a destra. ↑ ↓ Invio Esc e mouse. Invio chiude la palette e poi esegue, così il focus torna alla lista prima del comando.
  - `TitleBar`: il campo disabilitato della fase 1 diventa un pulsante con l'aspetto di un campo che apre la palette (`commands`) e mostra il tasto.
  - `tcKeymap.ts`: `'Ctrl+Shift+P': 'mc_CommandPalette'`.
- [ ] **Step 4: Verifica manuale:** Ctrl+Shift+P → "cop" → Invio copia; con un viewer attivo i comandi della lista (Copia, Elimina…) non compaiono; un click sul campo del titolo apre la palette.
- [ ] **Step 5: Commit** `feat(palette): command palette con ricerca fuzzy`

### Task 8: Salto rapido Ctrl+P

**Files:**
- Create: `src-tauri/src/fs/scan.rs`, `src/state/recentDirs.ts`, `src/state/recentDirs.test.ts`
- Modify: `src-tauri/src/fs/mod.rs`, `src-tauri/src/lib.rs`, `src/ipc/api.ts`, `src/ipc/types.ts`, `src/state/tabViews.ts` (registra la cartella dopo un `load` riuscito), `src/state/hotlist.ts` (+ test), `src/state/palette.ts` (+ test), `src/components/palette/Palette.tsx`, `src/commands/palette.ts`, `src/commands/tcKeymap.ts`, i18n

**Interfaces:**
- Consumes: Task 1 `Requests`, `nextRequestId`, `cancelRequest`; Task 2 `ensureDirTab`; Task 7 palette e `fuzzyFilter`; fase 1 `walk_dir`
- Produces (Rust):

```rust
#[serde(rename_all = "camelCase")] pub struct ScanEntry { pub rel: String, pub is_dir: bool }   // rel con '\'
pub struct ScanLimits { pub max_depth: u32, pub max_entries: u32, pub include_hidden: bool }
pub const EXCLUDED_DIRS: [&str; 2] = [".git", "node_modules"];  // ponytail: elenco fisso, configurabile se serve
pub fn scan_tree(root: &Path, limits: &ScanLimits, cancel: &AtomicBool, emit: &mut dyn FnMut(Vec<ScanEntry>)) -> Result<bool, AppError>; // true = troncata
// comando scan_tree(request_id, root, max_depth, max_entries, include_hidden, on_batch: Channel<Vec<ScanEntry>>) -> Result<bool, AppError>
```

- Produces (TS):

```ts
scanTree(requestId: number, root: string, limits: { maxDepth: number; maxEntries: number; includeHidden: boolean }, onBatch: (b: ScanEntry[]) => void): Promise<boolean>
pushRecent(list: string[], path: string, max?: number): string[]          // MRU, doppioni senza distinzione di maiuscole, max 50
useRecentDirs: { dirs: string[]; push(path: string): void }
hotlistDirs(items: HotlistItem[]): { name: string; path: string }[]        // in profondità, nell'ordine del menu
type JumpItem = { kind: 'recent' | 'favorite'; path: string; label: string } | { kind: 'file'; rel: string; isDir: boolean; label: string }
jumpItems(recent: string[], favorites: { name: string; path: string }[], files: ScanEntry[]): JumpItem[]
// comando mc_QuickJump ('group'), Ctrl+P
```

  Regole:
  - Visita in ampiezza (prima tutte le voci a profondità 1, poi 2…), lotti da 500 voci, senza entrare nei reparse point né in `EXCLUDED_DIRS`; le sottocartelle illeggibili si saltano. Con `include_hidden` falso si scartano le voci hidden o system.
  - Limiti del frontend: profondità 8, 20.000 voci, `includeHidden` = `showHidden`.
  - `jumpItems`: prima le recenti, poi i preferiti, poi i file. `fuzzyFilter` è stabile, quindi a parità di punteggio vince quell'ordine.

- [ ] **Step 1: Test** (Rust in `scan.rs`)

```rust
#[test] fn breadth_first_with_depth_limit() {
    /* a\b\c\d.txt, a\x.txt, top.txt; max_depth 2 → tutte le voci a profondità 1 prima di quelle a profondità 2;
       nessuna voce con più di 2 componenti; Ok(false) */
}
#[test] fn entry_limit_truncates() { /* 30 file, max_entries 10 → esattamente 10 voci, Ok(true) */ }
#[test] fn excluded_and_hidden() { /* .git\cfg, node_modules\m.js, h.txt hidden → nessuna delle tre; con include_hidden h.txt compare */ }
#[test] fn junction_is_listed_but_not_followed() {
    /* cmd /c mklink /J link <cartella con f.txt> (non serve l'amministratore) → "link" compare, "link\f.txt" no */
}
#[test] fn cancel_stops() { /* flag già true → Err(Cancelled) */ }
```

  TS:

```ts
test('pushRecent', () => {
  expect(pushRecent(['C:\\a', 'C:\\b'], 'c:\\B')).toEqual(['c:\\B', 'C:\\a']);
  expect(pushRecent(Array.from({ length: 50 }, (_, i) => `C:\\${i}`), 'D:\\x')).toHaveLength(50);
});
test('hotlistDirs flattens submenus in menu order', () => { /* dir A, submenu{dir B, separator}, dir C → [A, B, C] */ });
test('jumpItems keeps recent, favorites, files order', () => {
  expect(jumpItems(['C:\\r'], [{ name: 'Fav', path: 'C:\\f' }], [{ rel: 'x\\y.txt', isDir: false }]).map((i) => i.kind))
    .toEqual(['recent', 'favorite', 'file']);
});
```

- [ ] **Step 2:** `cargo test` e `npm test` → FAIL, poi implementare, poi → PASS.
- [ ] **Step 3:** UI: la palette in modalità `jump` parte con le recenti e i preferiti e avvia `scanTree` dalla cartella della scheda attiva (non per `drives:`); i file arrivano a lotti e la lista si aggiorna mentre si scrive. Alla chiusura `cancelRequest`. Scelta: una cartella → `load(ensureDirTab(), path)`; un file → `load(ensureDirTab(), cartella, { focus: nome })`. In fondo alla lista un'indicazione "scansione in corso" / "risultati troncati". `tcKeymap.ts`: `'Ctrl+P': 'mc_QuickJump'`.
- [ ] **Step 4: Verifica manuale:** su `C:\Windows` (decine di migliaia di voci) la palette resta reattiva mentre arrivano i lotti. Si misura `fuzzyFilter` su 20.000 voci (log `[perf] jump` in dev): se supera 16 ms, si applica un debounce di 50 ms all'input. Scegliendo un file, la lista va nella sua cartella con il cursore sopra.
- [ ] **Step 5: Commit** `feat(palette): salto rapido Ctrl+P tra recenti, preferiti e file`

### Task 9: Vista miniature (cm_SrcThumbs)

**Files:**
- Create: `src-tauri/src/shell/thumbnail.rs`, `src/state/thumbnails.ts`, `src/state/thumbnails.test.ts`
- Modify: `src-tauri/src/shell/mod.rs`, `src-tauri/src/lib.rs`, `src/ipc/api.ts`, `src/state/tabView.ts` (`view: 'full' | 'brief' | 'thumbs'`), `src/components/groups/FileList.tsx` (`ThumbList`), `src/components/groups/groups.css`, `src/commands/navigation.ts` (+ test), `src/commands/tcKeymap.ts`, `src/components/viewer/ImagePane.tsx`, i18n

**Interfaces:**
- Consumes: fase 1 `check_shell_name`, `hr_error`, il pattern STA di `shell/open.rs`
- Produces (Rust):

```rust
pub fn thumbnail(path: &Path, size: u32) -> Result<(u32, u32, Vec<u8>), AppError>;   // RGBA, lato maggiore ≤ size
pub(crate) fn bgra_to_rgba(px: &mut [u8]);   // scambia B e R; se tutti gli alfa sono 0 li porta a 255
// comando thumbnail(path, size) -> tauri::ipc::Response: [larghezza u32 LE][altezza u32 LE][RGBA]
```

- Produces (TS):

```ts
thumbnail(path: string, size: number): Promise<{ width: number; height: number; data: Uint8ClampedArray }>
createThumbQueue<T>(fetch: (path: string, size: number) => Promise<T>, opts: { concurrency: number; cacheSize: number; dispose(v: T): void }):
  { request(owner: string, key: string, path: string, size: number): Promise<T | null>; cancelOwner(owner: string): void }
thumbs  // istanza dell'app: ImageBitmap, concorrenza 4, cache 400, dispose = bitmap.close()
cursorStep(view: TabView['view'], move: 'up' | 'down' | 'left' | 'right' | 'pageUp' | 'pageDown', page: number, cols: number): number
setGridColumns(n: number): void
// comando cm_SrcThumbs (fileList), Ctrl+Shift+F1
```

  Regole:
  - `size` fuori da 16–1024 → `INVALID_REQUEST`. Ogni richiesta gira in `spawn_blocking` su un thread con `CoInitializeEx` STA: `SHCreateItemFromParsingName` → `IShellItemImageFactory::GetImage(size, SIIGBF_RESIZETOFIT)` (miniatura, o icona se non c'è) → `GetDIBits` a 32 bpp top-down → `DeleteObject`. Percorso in forma normale, non `\\?\`.
  - `key` = `path|modifiedMs|size`; un hit in cache non richiama `fetch`; `cancelOwner` toglie dalla coda le richieste non partite e le risolve con `null`.
  - `cursorStep`: `thumbs` → su/giù ∓`cols`, sinistra/destra ∓1, pagina ∓`page × cols`; `brief` → sinistra/destra ∓`page` (come oggi); `full` → su/giù ∓1, sinistra/destra 0. I comandi `mc_Cursor*` passano tutti da qui.

- [ ] **Step 1: Test**

```rust
#[test] fn swaps_channels_and_fixes_missing_alpha() {
    let mut a = vec![1, 2, 3, 0, 4, 5, 6, 0]; bgra_to_rgba(&mut a); assert_eq!(a, [3, 2, 1, 255, 6, 5, 4, 255]);
    let mut b = vec![1, 2, 3, 128]; bgra_to_rgba(&mut b); assert_eq!(b, [3, 2, 1, 128]);
}
#[test] fn thumbnail_of_a_bitmap() {
    /* il test scrive un BMP 24 bit 8×8 (header a mano); thumbnail(p, 32) → 0 < w,h ≤ 32 e data.len() == w*h*4 */
}
#[test] fn thumbnail_errors() { /* file mancante → NOT_FOUND; size 4 → INVALID_REQUEST */ }
```

```ts
test('queue limits concurrency and honours cancel', async () => {
  const started: string[] = []; const gates = new Map<string, () => void>();
  const q = createThumbQueue((p) => { started.push(p); return new Promise<string>((r) => gates.set(p, () => r(p))); },
    { concurrency: 2, cacheSize: 10, dispose: () => {} });
  const res = ['a', 'b', 'c', 'd'].map((p) => q.request(p === 'd' ? 't2' : 't1', p, p, 96));
  expect(started).toEqual(['a', 'b']);
  q.cancelOwner('t2'); gates.get('a')!(); await res[0];
  expect(started).toEqual(['a', 'b', 'c']); expect(await res[3]).toBeNull();
});
test('cache hit and eviction', async () => { /* cacheSize 1: request k1 due volte → fetch 1 volta; poi k2 → dispose(k1) */ });
test('cursorStep', () => {
  expect(cursorStep('thumbs', 'down', 5, 4)).toBe(4); expect(cursorStep('thumbs', 'left', 5, 4)).toBe(-1);
  expect(cursorStep('thumbs', 'pageDown', 5, 4)).toBe(20); expect(cursorStep('brief', 'right', 5, 4)).toBe(5);
  expect(cursorStep('full', 'right', 5, 4)).toBe(0);
});
```

- [ ] **Step 2:** `cargo test` e `npm test` → FAIL, poi implementare, poi → PASS.
- [ ] **Step 3:** UI:
  - `ThumbList`: griglia di riquadri larghi 112 px con l'immagine a 96 px e il nome sotto (2 righe con ellissi), virtualizzata per righe di riquadri con TanStack Virtual. Colonne = `floor(larghezza / 112)`, comunicate con `setGridColumns`. Ogni riquadro visibile chiede `thumbs.request(tabId, …)` e disegna l'`ImageBitmap` su un canvas; cursore e marcatura come nella lista.
  - Cambiando cartella o vista si chiama `cancelOwner(tabId)`.
  - `cm_SrcThumbs` imposta `view: 'thumbs'`; Ctrl+F1/F2 tornano a breve e dettagliata.
  - `ImagePane`: su errore di caricamento disegna `thumbnail(path, 1024)` con la nota `viewer.shellPreview` (TIFF, HEIC, PSD con il codec installato).
- [ ] **Step 4: Verifica manuale:** cartella con 2.000 JPEG: Ctrl+Shift+F1 mostra subito i riquadri, le miniature arrivano senza bloccare lo scorrimento, le frecce si muovono sulla griglia senza ritardo; MP4 e PDF mostrano la miniatura della shell; F3 su un TIFF mostra l'anteprima della shell.
- [ ] **Step 5: Commit** `feat(ui): vista miniature con anteprime della shell`

### Task 10: Backend dell'ispettore: hash e contenuto degli archivi

**Files:**
- Create: `src-tauri/src/fs/hash.rs`, `src-tauri/src/archive/mod.rs`, `src-tauri/src/archive/list.rs`
- Modify: `src-tauri/src/fs/mod.rs`, `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml` (feature `windows` `Win32_Security_Cryptography`; `cargo add zip tar flate2 sevenz-rust2`, con le feature minime che servono a leggere l'indice: le aggiungerà la fase 3), `src/ipc/api.ts`, `src/ipc/types.ts`

**Interfaces:**
- Consumes: Task 1 `Requests`, `open_shared`
- Produces (Rust):

```rust
#[serde(rename_all = "lowercase")] pub enum HashAlgo { Md5, Sha1, Sha256 }
pub fn hash_file(path: &Path, algo: HashAlgo, cancel: &AtomicBool) -> Result<String, AppError>;   // esadecimale minuscolo
// comando async calc_hash(request_id, path, algo) -> Result<String, AppError>

#[serde(rename_all = "camelCase")] pub struct ArchiveEntry { pub path: String, pub is_dir: bool, pub size: u64, pub modified_ms: Option<i64> }
#[serde(rename_all = "camelCase")] pub struct ArchiveListing { pub entries: Vec<ArchiveEntry>, pub total: Option<u64>, pub truncated: bool }
pub enum ArchiveFormat { Zip, SevenZ, Tar, TarGz }
pub fn format_of(path: &Path) -> Option<ArchiveFormat>;
pub fn list_archive(path: &Path, limit: usize, cancel: &AtomicBool) -> Result<ArchiveListing, AppError>;
// comando async list_archive(request_id, path, limit) -> Result<ArchiveListing, AppError>
```

- Produces (TS): `calcHash(requestId, path, algo: HashAlgo): Promise<string>`, `listArchive(requestId, path, limit): Promise<ArchiveListing>`; tipi `HashAlgo`, `ArchiveEntry`, `ArchiveListing`.

  Regole:
  - Hash con BCrypt (`BCryptOpenAlgorithmProvider` / `BCryptHashData` / `BCryptFinishHash`), blocchi da 1 MiB letti con `open_shared`, annullamento controllato a ogni blocco. Niente crate in più.
  - `path` negli archivi usa `\`. `total` è noto per ZIP e 7z (indice centrale), `None` per TAR e TAR.GZ quando l'elenco si ferma al limite. Un archivio illeggibile → `CORRUPT_ARCHIVE`; un 7z con intestazioni cifrate → `UNSUPPORTED` con detail `encrypted`; un'estensione non riconosciuta → `UNSUPPORTED`. RAR resta fuori (§8 dello spec).
  - TAR.GZ si legge a flusso: niente decompressione completa in memoria.

- [ ] **Step 1: Test**

```rust
#[test] fn known_digests() {
    let d = tempfile::tempdir().unwrap(); let f = d.path().join("abc"); std::fs::write(&f, b"abc").unwrap();
    let no = AtomicBool::new(false);
    assert_eq!(hash_file(&f, HashAlgo::Md5, &no).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(hash_file(&f, HashAlgo::Sha1, &no).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(hash_file(&f, HashAlgo::Sha256, &no).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let e = d.path().join("empty"); std::fs::write(&e, b"").unwrap();
    assert_eq!(hash_file(&e, HashAlgo::Sha256, &no).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hash_file(&f, HashAlgo::Md5, &AtomicBool::new(true)).unwrap_err().code, ErrorCode::Cancelled);
}
#[test] fn formats_by_extension() {
    assert!(matches!(format_of(Path::new("a.TAR.GZ")), Some(ArchiveFormat::TarGz)));
    assert!(matches!(format_of(Path::new("a.tgz")), Some(ArchiveFormat::TarGz)));
    assert!(format_of(Path::new("a.gz")).is_none());
}
#[test] fn lists_zip_tar_tgz_and_7z() {
    /* per ogni formato il test crea un archivio con "d/" (cartella), "d/x.txt" (3 byte), "y.bin" (5 byte)
       con lo scrittore del crate (zip::ZipWriter, tar::Builder, tar::Builder su GzEncoder, la compressione di sevenz-rust2);
       list_archive → 3 voci con path "d", "d\\x.txt", "y.bin", dimensioni 0/3/5, is_dir solo per "d" */
}
#[test] fn limit_and_corruption() {
    /* zip da 3 voci con limit 1 → 1 voce, truncated, total Some(3); 100 byte casuali con nome .zip → CORRUPT_ARCHIVE */
}
```

- [ ] **Step 2:** `cargo test` → FAIL, poi implementare, poi `cargo test` → PASS.
- [ ] **Step 3: Commit** `feat(fs): hash dei file con BCrypt ed elenco del contenuto degli archivi`

### Task 11: Ispettore Ctrl+Q

**Files:**
- Create: `src/state/inspector.ts`, `src/state/inspector.test.ts`, `src/lib/markdown.ts`, `src/lib/markdown.test.ts`, `src/components/inspector/Inspector.tsx`, `src/components/inspector/inspector.css`
- Modify: `package.json` (`npm install markdown-it`, `npm install -D @types/markdown-it`), `src/state/ui.ts`, `src/App.tsx`, `src/components/shell/TitleBar.tsx`, `src/components/shell/shell.css`, `src/commands/navigation.ts`, `src/commands/tcKeymap.ts`, i18n

**Interfaces:**
- Consumes: Task 1 `fileInfo`, `readChunk`, `cancelRequest`, `nextRequestId`; Task 3 `previewKind`, `detectEncoding`, `looksBinary`; Task 5 `ImagePane`/`PdfPane`/`MediaPane`; Task 6 `loadMonaco`, `languageFor`; Task 10 `calcHash`, `listArchive`; fase 1 `computeDirSize`, `cancelTabSizes`, `formatAttrs`, `formatSize`, `formatDate`
- Produces:

```ts
// ui.ts
inspectorVisible: boolean; inspectorWidth: number; toggleInspector(): void; setInspectorWidth(w: number): void
// inspector.ts
PREVIEW_BYTES = 256 * 1024; DEBOUNCE_MS = 120; ARCHIVE_LIMIT = 500
type Preview =
  | { kind: 'loading' } | { kind: 'image' | 'pdf' | 'video' | 'audio'; path: string }
  | { kind: 'text'; text: string; language: string | null; truncated: boolean }
  | { kind: 'markdown'; html: string; truncated: boolean }
  | { kind: 'folder'; size: DirSize | null } | { kind: 'drive'; drive: Drive }
  | { kind: 'archive'; listing: ArchiveListing } | { kind: 'binary' } | { kind: 'error'; code: ErrorCode }
interface InspectorTarget { path: string; isDir: boolean; drive?: Drive }
useInspector: { target: InspectorTarget | null; info: FileInfo | null; preview: Preview | null;
  hashes: Partial<Record<HashAlgo, string | 'pending'>>; setTarget(t: InspectorTarget | null): void; requestHash(algo: HashAlgo): void }
// markdown.ts
renderMarkdown(text: string): Promise<string>   // markdown-it importato al primo uso, html: false
// comando cm_SrcQuickview ('group'), Ctrl+Q
```

  Regole:
  - `setTarget` ha un debounce di `DEBOUNCE_MS`. Ogni bersaglio nuovo incrementa un token: un risultato si applica solo se il suo token è ancora quello corrente. Al cambio si annullano il calcolo della dimensione (`cancelTabSizes('inspector')`, che riusa il proprietario fittizio `'inspector'`) e le richieste di hash e archivio in corso (`cancelRequest`).
  - Cartella → `{ kind: 'folder' }` con `computeDirSize('inspector', path)` (byte, file, sottocartelle). Riga di un'unità → `drive`. File: `previewKind` decide image/pdf/video/audio/archive (`listArchive` con `ARCHIVE_LIMIT`); markdown e testo leggono `PREVIEW_BYTES` con `readChunk` (`looksBinary` → `binary`). Il testo si colora con `monaco.editor.colorize` se `languageFor` trova un linguaggio, altrimenti resta testo semplice.
  - L'ispettore lavora solo quando è visibile. Con una scheda di file attiva mostra un messaggio vuoto.

- [ ] **Step 1: Test** (con `vi.mock('../ipc/api')` e timer finti, come in `tabViews.test.ts`)

```ts
// fileInfo simulato con un FileInfo fisso; enc = TextEncoder
test('debounces and only the latest target wins', async () => {
  vi.useFakeTimers();
  readChunk.mockImplementation((p) => new Promise((r) => setTimeout(() => r(enc(p)), p.endsWith('a.txt') ? 500 : 10)));
  useInspector.getState().setTarget({ path: 'C:\\a.txt', isDir: false });
  await vi.advanceTimersByTimeAsync(130);                       // parte la lettura di a (lenta)
  useInspector.getState().setTarget({ path: 'C:\\b.txt', isDir: false });
  await vi.advanceTimersByTimeAsync(1000);
  expect(useInspector.getState().preview).toMatchObject({ kind: 'text', text: 'C:\\b.txt' });
});
test('rapid moves inside the debounce read nothing', async () => {
  /* setTarget su 20 file a 10 ms l'uno dall'altro → readChunk chiamata una volta sola, per l'ultimo */
});
test('changing target cancels folder size and hash', async () => {
  /* setTarget cartella → computeDirSize chiamato; requestHash('sha256'); setTarget altro file →
     cancelTabSizes('inspector') e cancelRequest(id dell'hash) chiamati; hashes vuoto */
});
test('binary files are not shown as text', async () => { /* readChunk → [0x00, 0x01] → preview { kind: 'binary' } */ });
```

```ts
test('markdown never renders raw html', async () => {
  const html = await renderMarkdown('# T\n<script>alert(1)</script>\n[x](javascript:alert(1))');
  expect(html).toContain('<h1>'); expect(html).not.toContain('<script'); expect(html).not.toContain('href="javascript:');
});
```

- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** UI:
  - `Inspector` a destra di `mc-center`, larghezza 320 px ridimensionabile con `Splitter` (minimo 220, massimo metà finestra).
  - Si abbona a cursore, scheda e gruppo attivi e chiama `setTarget` (niente per `..`).
  - In alto l'anteprima (riquadri del Task 5; `autoPlay` falso; testo in `<pre>` monospazio con l'HTML di `colorize`; Markdown in un contenitore il cui click sui link fa `preventDefault`).
  - Sotto i metadati: dimensione (leggibile ed esatta), creazione, modifica, ultimo accesso, attributi (`formatAttrs`), e i pulsanti MD5 / SHA-1 / SHA-256 con il valore selezionabile e "Copia" (`navigator.clipboard.writeText`).
  - Per gli archivi l'elenco delle prime 500 voci e il totale.
  - `cm_SrcQuickview` (Ctrl+Q) → `toggleInspector`; pulsante nella barra del titolo accanto agli altri due.
- [ ] **Step 4: Verifica manuale:**
  - ↓ tenuto premuto su una cartella di 1.000 file misti con l'ispettore aperto: il cursore non rallenta e alla fine l'anteprima è quella del file sotto il cursore.
  - Una cartella enorme (`C:\Windows`): il calcolo parte e si annulla appena ci si sposta.
  - Un `.md` con HTML dentro lo mostra come testo.
  - Un MP4 non parte da solo.
  - Uno ZIP mostra l'elenco; uno ZIP rotto mostra l'errore tradotto.
  - L'SHA-256 di un file da 4 GB si può annullare spostando il cursore.
- [ ] **Step 5: Commit** `feat(ui): ispettore Ctrl+Q con anteprime, metadati e hash`

### Task 12: Terminale: backend ConPTY

**Files:**
- Create: `src-tauri/src/terminal/mod.rs`
- Modify: `src-tauri/src/lib.rs` (gestione dello stato; `.run(ctx)` diventa `.build(ctx)?.run(|app, e| …)` per chiudere i terminali a `RunEvent::Exit`), `src-tauri/Cargo.toml` (`cargo add portable-pty`), `src/ipc/api.ts`, `src/ipc/types.ts`

**Interfaces:**
- Produces (Rust):

```rust
#[serde(rename_all = "lowercase")] pub enum ShellKind { Pwsh, Powershell, Cmd, Wsl }
pub fn available_shells() -> Vec<ShellKind>;                    // eseguibile trovato nel PATH, nell'ordine dell'enum
pub fn default_shell(available: &[ShellKind]) -> ShellKind;     // Pwsh, poi Powershell, poi Cmd
pub fn shell_command(kind: ShellKind, cwd: &Path) -> portable_pty::CommandBuilder;
#[derive(Default)] pub struct Terminals;                        // stato Tauri: id -> sessione (master, writer, child)
impl Terminals { pub fn kill_all(&self); }
// comandi: term_shells() -> Vec<ShellKind>;
//   term_spawn(shell: Option<ShellKind>, cwd: String, cols: u16, rows: u16, on_output: Channel<InvokeResponseBody>) -> Result<u32, AppError>;
//   term_write(id: u32, data: String); term_resize(id: u32, cols: u16, rows: u16); term_kill(id: u32)
// evento "term-exit" { id: u32, code: Option<u32> }
```

- Produces (TS): `termShells()`, `termSpawn(shell: ShellKind | null, cwd: string, cols: number, rows: number, onOutput: (bytes: Uint8Array) => void): Promise<number>` (crea un `Channel` di `@tauri-apps/api/core`; i messaggi grezzi arrivano come `ArrayBuffer`), `termWrite`, `termResize`, `termKill`, `onTermExit(cb: (e: { id: number; code: number | null }) => void)`.

  Regole:
  - `shell_command`: Pwsh → `pwsh.exe -NoLogo`; Powershell → `powershell.exe -NoLogo`; Cmd → `cmd.exe`; Wsl → `wsl.exe --cd <cwd>`. Per le shell Windows la cartella si imposta con `cwd`. Una `cwd` vuota, inesistente o `drives:` diventa `%USERPROFILE%`.
  - Un thread per sessione legge a blocchi di 8 KiB e invia ogni blocco così com'è (`InvokeResponseBody::Raw`): è xterm.js a ricomporre l'UTF-8 spezzato tra due blocchi. A fine flusso aspetta il processo, emette `term-exit` e toglie la sessione.
  - `term_write` / `term_resize` / `term_kill` su un id sconosciuto → `INVALID_REQUEST`.

- [ ] **Step 1: Test**

```rust
#[test] fn shell_commands() {
    let c = shell_command(ShellKind::Wsl, Path::new(r"C:\x"));
    assert_eq!(c.get_argv(), &["wsl.exe", "--cd", r"C:\x"].map(OsString::from));
    assert_eq!(shell_command(ShellKind::Pwsh, Path::new(r"C:\")).get_argv(), &["pwsh.exe", "-NoLogo"].map(OsString::from));
}
#[test] fn default_shell_priority() {
    assert_eq!(default_shell(&[ShellKind::Cmd, ShellKind::Powershell]), ShellKind::Powershell);
    assert_eq!(default_shell(&[]), ShellKind::Cmd);
}
#[test] fn pty_runs_a_command_and_reports_exit() {
    /* con la funzione interna di avvio (senza Tauri): cmd.exe /c echo mc-pty-ok → l'output raccolto contiene
       "mc-pty-ok" e la callback di uscita arriva entro 5 s */
}
#[test] fn pty_accepts_input() { /* cmd.exe interattivo; write "echo mc-in-%OS%\r\nexit\r\n" → output con "mc-in-Windows_NT", poi uscita */ }
```

  (`get_argv` è il nome dell'accessor nella versione di `portable-pty` usata in fase di stesura; se la versione installata lo chiama diversamente si adegua il test, non l'asserzione.)
- [ ] **Step 2:** `cargo test` → FAIL, poi implementare, poi `cargo test` → PASS.
- [ ] **Step 3: Commit** `feat(terminal): terminali ConPTY nel backend`

### Task 13: Terminale a schede nella UI (Ctrl+ò)

**Files:**
- Create: `src/state/terminals.ts`, `src/state/terminals.test.ts`, `src/lib/xterm.ts`, `src/components/panel/TerminalView.tsx`, `src/commands/terminal.ts`
- Modify: `package.json` (`npm install @xterm/xterm @xterm/addon-fit`), `src/components/shell/BottomPanel.tsx`, `src/components/panel/panel.css`, `src/App.tsx` (il pannello resta montato e si nasconde con `hidden`, perché un'istanza xterm non si riapre su un altro elemento), `src/state/ui.ts` (`showBottomPanel(tab)`), `src/commands/tcKeymap.ts`, `src/main.tsx`, `src/state/settings.ts` (+ test), `src/components/settings/SettingsPage.tsx`, i18n

**Interfaces:**
- Consumes: Task 12 API; fase 1 `useUi.bottomPanelTab`, `chordFromEvent`, `getCommand`
- Produces:

```ts
interface TermSession { id: number; title: string; shell: ShellKind }      // id = id del PTY
removeSession(sessions: TermSession[], activeId: number | null, id: number): { sessions: TermSession[]; activeId: number | null }
toggleTerminal(panel: { visible: boolean; tab: 'operations' | 'terminal' }, hasSessions: boolean): { visible: boolean; tab: 'operations' | 'terminal'; create: boolean }
passToShell(e: KeyLike, keymap: Map<string, string>, isGlobal: (commandId: string) => boolean): boolean   // per attachCustomKeyEventHandler
useTerminals: { sessions: TermSession[]; activeId: number | null; create(cwd: string, shell?: ShellKind): Promise<void>; close(id: number): void; activate(id: number): void }
loadXterm(): Promise<{ Terminal: typeof import('@xterm/xterm').Terminal; FitAddon: typeof import('@xterm/addon-fit').FitAddon }>
Settings.terminalShell: 'auto' | ShellKind     // default 'auto'
// comandi mc_ToggleTerminal (global, Ctrl+Oem3) e mc_NewTerminal (global, Ctrl+Shift+Oem3); mc_ToggleBottomPanel resta senza tasto
```

  Regole:
  - `removeSession`: se si chiude l'attiva, diventa attiva la vicina a destra, altrimenti quella a sinistra; `null` se non ne resta nessuna.
  - `toggleTerminal`: pannello visibile sulla scheda terminale → si nasconde; altrimenti si mostra sulla scheda terminale, e `create` è vero se non ci sono sessioni.
  - `passToShell`: falso solo per i chord legati a un comando `global` (Ctrl+ò, Ctrl+Shift+P, Ctrl+Shift+E…), così arrivano a `useKeyboard`; tutto il resto (Ctrl+C, Tab, frecce, Ctrl+P) va alla shell.
  - All'evento `term-exit` la sessione si toglie; se era l'ultima il pannello si nasconde (§5.6). Chiudere una scheda chiama `termKill`.
  - Una sessione nuova parte nella cartella della scheda attiva del gruppo attivo (`tab.path`); `drives:` passa vuota e il backend usa la cartella dell'utente.
  - Le istanze xterm stanno in una `Map<id, …>` fuori dallo store. Un `ResizeObserver` chiama `fit()` e poi `termResize`. Font monospazio e colori dai token (aggiornati al cambio tema). Ctrl+C con una selezione copia invece di inviare ^C.

- [ ] **Step 1: Test**

```ts
const s = (id: number) => ({ id, title: `t${id}`, shell: 'pwsh' as const });
test('removeSession', () => {
  expect(removeSession([s(1), s(2), s(3)], 2, 2)).toEqual({ sessions: [s(1), s(3)], activeId: 3 });
  expect(removeSession([s(1), s(2)], 2, 2).activeId).toBe(1);
  expect(removeSession([s(1)], 1, 1)).toEqual({ sessions: [], activeId: null });
  expect(removeSession([s(1), s(2)], 1, 2).activeId).toBe(1);
});
test('toggleTerminal', () => {
  expect(toggleTerminal({ visible: true, tab: 'terminal' }, true)).toEqual({ visible: false, tab: 'terminal', create: false });
  expect(toggleTerminal({ visible: true, tab: 'operations' }, true)).toEqual({ visible: true, tab: 'terminal', create: false });
  expect(toggleTerminal({ visible: false, tab: 'operations' }, false)).toEqual({ visible: true, tab: 'terminal', create: true });
});
test('passToShell keeps global shortcuts for the app', () => {
  const km = new Map([['Ctrl+Oem3', 'mc_ToggleTerminal'], ['Ctrl+Shift+P', 'mc_CommandPalette'], ['Ctrl+P', 'mc_QuickJump'], ['Tab', 'mc_FocusNextGroup']]);
  const global = (id: string) => id === 'mc_ToggleTerminal' || id === 'mc_CommandPalette';
  const k = (keyCode: number, m: Partial<KeyLike> = {}) => ({ keyCode, ctrlKey: false, shiftKey: false, altKey: false, ...m });
  expect(passToShell(k(192, { ctrlKey: true }), km, global)).toBe(false);
  expect(passToShell(k(80, { ctrlKey: true, shiftKey: true }), km, global)).toBe(false);
  expect(passToShell(k(80, { ctrlKey: true }), km, global)).toBe(true);
  expect(passToShell(k(9), km, global)).toBe(true);
  expect(passToShell(k(67, { ctrlKey: true }), km, global)).toBe(true);
});
```

  In `settings.test.ts` il default diventa `terminalShell: 'auto'`.
- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** UI:
  - `BottomPanel` con due schede nell'intestazione, Terminale e Operazioni. La scheda Terminale contiene `TerminalView`: una striscia di sessioni (titolo = nome della shell e cartella, ×) con "+" e un menu per scegliere la shell (`termShells`), e sotto le istanze xterm (solo l'attiva è visibile, tutte restano montate).
  - Impostazioni: `terminalShell` (automatica / PowerShell 7 / Windows PowerShell / cmd / WSL; si elencano solo quelle di `termShells`).
  - `tcKeymap.ts`: `'Ctrl+Oem3': 'mc_ToggleTerminal'` (sostituisce `mc_ToggleBottomPanel`), `'Ctrl+Shift+Oem3': 'mc_NewTerminal'`.
- [ ] **Step 4: Verifica manuale:**
  - Ctrl+ò apre il pannello con un PowerShell 7 nella cartella del gruppo attivo; Ctrl+ò di nuovo lo nasconde e l'output resta al ritorno.
  - "+" ne apre un secondo; `exit` chiude la scheda; chiudendo l'ultima il pannello si nasconde.
  - Tab completa i percorsi nella shell e non cambia gruppo; Ctrl+Shift+P apre la palette anche dal terminale.
  - Caratteri accentati e emoji si vedono bene; `vim` in WSL funziona e si adatta al ridimensionamento del pannello.
  - Chiudendo l'app non restano processi `pwsh.exe` o `conhost.exe` orfani (Gestione attività).
  - `npm run build`: xterm sta in un chunk separato.
- [ ] **Step 5: Commit** `feat(terminal): terminale a schede nel pannello in basso con Ctrl+ò`

### Task 14: Mappa tasti, documentazione e verifica finale della fase

**Files:**
- Modify: `src/commands/keymap.test.ts`, `README.md`
- Create: `docs/superpowers/fase-2-follow-up.md`

  Tasti aggiunti in questa fase (verificati nel Task 2, Step 1):

  | Tasti | Comando | Contesto |
  |---|---|---|
  | Ctrl+Q | cm_SrcQuickview | group |
  | Ctrl+Shift+F1 | cm_SrcThumbs | fileList |
  | Ctrl+Shift+P | mc_CommandPalette | global |
  | Ctrl+P | mc_QuickJump | group |
  | Ctrl+Oem3 (Ctrl+ò) | mc_ToggleTerminal | global |
  | Ctrl+Shift+Oem3 | mc_NewTerminal | global |

  F3 (`cm_List`) e F4 (`cm_Edit`) mantengono i tasti; cambia il comportamento.

- [ ] **Step 1: Test** in `keymap.test.ts`: ogni riga della tabella è in `DEFAULT_KEYMAP`; nessun chord compare due volte; `mc_ToggleBottomPanel` non ha più un tasto; tutti gli id registrati dai moduli della fase 2 hanno un titolo in `commands.*` di `en.json` (test sulle chiavi come per la fase 1).
- [ ] **Step 2:** `npm test` → PASS.
- [ ] **Step 3:** `npm run build` e controllo di `dist/assets`: il chunk d'ingresso non contiene Monaco, pdf.js, xterm né markdown-it (si cercano nomi come `monaco`, `pdf.worker`, `xterm` nel file d'ingresso e devono mancare). Avvio della build release a finestra pronta < 1 s, come nel Task 22 della fase 1.
- [ ] **Step 4:** `npm test` e `cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks` → tutto PASS.
- [ ] **Step 5:** Ripassare a mano i cinque punti della Review Focus sulla build release.
- [ ] **Step 6:** `README.md`: sezione sulle funzioni della fase 2 e sui limiti accettati (a capo misti normalizzati dall'editor, codifiche DOS assenti nel viewer, risultati dell'asset protocol dello Step 1 del Task 5). `fase-2-follow-up.md`: verifiche manuali non automatizzabili e follow-up emersi, nello stesso formato di `fase-1-follow-up.md`.
- [ ] **Step 7: Commit** `docs: verifiche e follow-up della fase 2`
