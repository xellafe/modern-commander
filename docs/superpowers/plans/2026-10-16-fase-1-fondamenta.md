# Modern Commander — Fase 1 "Fondamenta": piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** costruire un file manager Windows usabile ogni giorno: layout a N gruppi con schede, navigazione e marcatura con le scorciatoie di TC, operazioni su file in coda con gestione dei conflitti ed elevazione UAC, integrazione con la shell, temi A/C, en/it, build portable e installer.

**Architecture:** app Tauri 2. Il backend Rust (`src-tauri/`) è l'unico punto che tocca il disco e le API Win32. Il frontend React + TypeScript (`src/`) contiene solo stato UI, comandi e rendering e parla col backend tramite comandi `invoke` ed eventi. Ogni azione utente è un comando registrato; tastiera, F-key e menu lo invocano per id.

**Tech Stack:** Tauri 2, Rust (crate `windows`, `notify`, `trash`, `serde`, `tempfile` per i test), React + TypeScript + Vite, Zustand, i18next/react-i18next, TanStack Virtual, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-16-modern-commander-design.md`. Questo piano copre la §9 fase 1. Le fasi 2–5 avranno piani separati.

## Global Constraints

- Solo Windows 10/11 x64. Si possono usare Win32/COM.
- Licenza MIT. Le dipendenze devono essere MIT-compatibili; niente GPL.
- Versioni delle dipendenze: si aggiungono solo con `npm install` / `cargo add`, mai scritte a mano. Le API di una libreria vanno lette nella documentazione della versione installata.
- Scaffolding con CLI ufficiale (`npm create tauri-app@latest`).
- **Commit solo con conferma esplicita dell'utente**: i passi "Commit" sono proposte da confermare.
- Documentazione, commenti e messaggi di commit in italiano; identificatori in inglese.
- Nessuna stringa UI scritta nel codice: sempre chiavi i18n, presenti sia in `en.json` sia in `it.json`.
- Il backend restituisce `AppError { code, path, detail }`, mai testo da mostrare; il frontend traduce `errors.<CODE>`.
- Scorciatoie: mappa di fabbrica di TC. Una scorciatoia nuova non può prendere un tasto di TC. I binding si basano sul virtual key (`KeyboardEvent.keyCode`), non sul carattere.
- Token tema A (scuro): bg `#0e1013`, pannelli `#15171c`, bordi `#262a32`, testo `#d6d9e0`, testo attenuato `#7d8391`, accento `#e07a4f`, selezione `rgba(224,122,79,.14)`, riga 22 px, raggio 0, icone monocromatiche.
- Token tema C (chiaro): bg `#faf9f5`, pannelli `#f0eee6`, bordi `#e3e0d6`, testo `#2b2a27`, testo attenuato `#8a877e`, accento `#c96442`, selezione `rgba(201,100,66,.12)`, riga 26 px, raggio 6 px, icone colorate tenui.
- Font UI `"Segoe UI Variable", "Segoe UI", sans-serif`; mono `"Cascadia Code", Consolas, monospace`.
- Prestazioni: avvio a finestra pronta < 1 s; cartella da 10.000 elementi visibile in < 150 ms; cartella da 100.000 utilizzabile; da tasto a cursore < 16 ms.
- Il frontend non accede mai direttamente al disco.
- Identificatore app provvisorio: `io.github.moderncommander`; nome prodotto `Modern Commander`.

## Review Focus

1. **Copia o spostamento di una cartella dentro se stessa o in una sua sottocartella** (`D:\a` → `D:\a\b`): l'operazione va rifiutata con `INTO_ITSELF`, senza ricorsione infinita. → test in Task 13.
2. **F5/F6 con destinazione uguale alla sorgente**: "sovrascrivi" non deve mai applicarsi a un file su se stesso (`SAME_FILE`); "rinomina" produce `nome (2).ext`. → test in Task 13 e Task 14.
3. **Copia fallita o annullata** (disco pieno, annullamento, sorgente sparita): a destinazione non deve restare nessun file troncato. In uno spostamento la sorgente resta intatta finché la copia non è completa. → test in Task 14.
4. **Percorsi oltre 260 caratteri e nomi con punto o spazio finale**: elenco, copia ed eliminazione funzionano (prefisso `\\?\`). → test in Task 3 e Task 14.
5. **Cartella corrente eliminata o rinominata dall'esterno, o unità rimossa**: la scheda risale al primo antenato esistente, e se non ne esiste nessuno mostra la lista delle unità. Nessun crash. → test in Task 4 e Task 11.

---

### Task 0: Prerequisiti di sistema

Rust e MSVC Build Tools non sono installati sulla macchina (verificato il 2026-10-16). Node v22 e WebView2 sono presenti. **Prima di installare serve la conferma dell'utente.**

- [ ] **Step 1:** Cercare gli ID dei pacchetti: `winget search Rustlang.Rustup` e `winget search Microsoft.VisualStudio BuildTools`. Prendere l'ultima versione stabile dei Build Tools.
- [ ] **Step 2:** Installare i Build Tools con il workload "Desktop development with C++" (`--override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive"`), poi `winget install Rustlang.Rustup`.
- [ ] **Step 3:** In una nuova shell: `rustc --version` e `cargo --version` stampano una versione; `rustup default` riporta `stable-x86_64-pc-windows-msvc`.

### Task 1: Scaffolding del progetto

**Files:**
- Create (CLI): `package.json`, `vite.config.ts`, `index.html`, `src/`, `src-tauri/`
- Create: `LICENSE` (MIT, titolare "Federico Xella"), `README.md` (italiano: scopo, prerequisiti, `npm run tauri dev`)
- Modify: `.gitignore` (aggiungere `node_modules/`, `dist/`, `src-tauri/target/`)

- [ ] **Step 1:** Nella root del repo: `npm create tauri-app@latest .` con template React + TypeScript e package manager npm; nome `modern-commander`, identificatore `io.github.moderncommander`. Se la CLI rifiuta la cartella non vuota, generare in una sottocartella temporanea e spostare i file nella root.
- [ ] **Step 2:** `npm install zustand i18next react-i18next @tanstack/react-virtual` e `npm install -D vitest`. Aggiungere `"test": "vitest run"` a `package.json`.
- [ ] **Step 3:** In `src-tauri`: `cargo add serde --features derive`, `cargo add serde_json notify trash`, `cargo add windows` (le feature Win32 si aggiungono nei task che le usano), `cargo add --dev tempfile`.
- [ ] **Step 4:** In `src-tauri/tauri.conf.json`: finestra `decorations: false` (barra del titolo personalizzata), `width 1400`, `height 860`, `minWidth 900`, `minHeight 560`, titolo `Modern Commander`. In `src-tauri/capabilities/default.json` aggiungere i permessi finestra `core:window:allow-minimize`, `allow-toggle-maximize`, `allow-close`, `allow-start-dragging`.
- [ ] **Step 5:** Scrivere lo smoke test `src/smoke.test.ts` (`expect(1 + 1).toBe(2)`) e verificare: `npm test` → PASS; `cargo test --manifest-path src-tauri/Cargo.toml` → PASS; `npm run tauri dev` apre una finestra senza bordi.
- [ ] **Step 6: Commit** `chore: scaffolding Tauri 2 + React + TypeScript`

### Task 2: Spike shell nativa (menu contestuale, proprietà, drag & drop)

È il rischio principale della §8 dello spec, quindi lo si affronta subito. Il task produce i comandi backend definitivi; l'integrazione nella UI arriva con il Task 18.

**Files:**
- Create: `src-tauri/src/shell/mod.rs`, `src-tauri/src/shell/context_menu.rs`, `src-tauri/src/shell/properties.rs`, `src-tauri/src/shell/drag.rs`
- Modify: `src-tauri/src/lib.rs` (registrazione comandi), `src-tauri/Cargo.toml` (feature `windows`: `Win32_UI_Shell`, `Win32_UI_Shell_Common`, `Win32_UI_WindowsAndMessaging`, `Win32_System_Com`, `Win32_Foundation`)

**Interfaces:**
- Produces:
  - `#[tauri::command] show_context_menu(window: tauri::Window, paths: Vec<String>, x: i32, y: i32) -> Result<(), AppError>`, dove `x`/`y` sono coordinate client in pixel fisici
  - `#[tauri::command] show_properties(paths: Vec<String>) -> Result<(), AppError>`
  - `#[tauri::command] start_drag(window: tauri::Window, paths: Vec<String>) -> Result<(), AppError>`
  - Il drop da Explorer arriva al frontend dall'evento nativo Tauri `onDragDropEvent` (nessun comando backend).

- [ ] **Step 1:** Implementare `show_context_menu`. Si esegue sul thread UI con `app.run_on_main_thread`. Sequenza:
  1. `SHParseDisplayName` per ogni percorso
  2. `SHBindToParent` per ottenere il parent `IShellFolder`
  3. `GetUIObjectOf` per ottenere `IContextMenu`
  4. `CreatePopupMenu` + `QueryContextMenu(CMF_NORMAL)`
  5. `ClientToScreen` sull'HWND della finestra, poi `TrackPopupMenuEx(TPM_RETURNCMD)`
  6. `InvokeCommand` con il comando scelto

  Tutti i percorsi devono avere la stessa cartella padre, altrimenti l'errore è `INVALID_REQUEST`. `IContextMenu2`/`3` e `HandleMenuMsg` servono per i sottomenu "Invia a" e "Apri con": se senza quei messaggi i sottomenu restano vuoti, sottoclassare la finestra con `SetWindowSubclass` e inoltrarli.
- [ ] **Step 2:** Implementare `show_properties` con `SHMultiFileProperties` (più file) o `SHObjectProperties` (un file).
- [ ] **Step 3:** Implementare `start_drag` con `DoDragDrop`: `IDataObject` con `CF_HDROP` (ottenuto da `SHCreateDataObject` sui PIDL) ed effetti `DROPEFFECT_COPY | DROPEFFECT_MOVE`. Se il crate `drag` (CrabNebula) copre il caso con licenza compatibile, si può usare quello al posto dell'implementazione a mano.
- [ ] **Step 4: Verifica manuale.** Da `npm run tauri dev`, nella console DevTools:
  - `await window.__TAURI_INTERNALS__.invoke('show_context_menu', {paths:['C:\\Windows\\notepad.exe'], x:200, y:200})` → compare il menu nativo, compresi "Apri con" con le voci popolate e le estensioni installate (per esempio 7-Zip, se presente)
  - `show_properties` apre la finestra Proprietà
  - un drag avviato da un handler temporaneo `onMouseDown` su un div di prova porta `notepad.exe` su Desktop o Explorer come copia
  - trascinando un file da Explorer sulla finestra, la console mostra l'evento `onDragDropEvent` con il percorso

  Rimuovere poi il div di prova.
- [ ] **Step 5:** Se uno dei punti dello Step 4 non si riesce a ottenere, **fermarsi e riferire all'utente** prima di proseguire: cambia il design.
- [ ] **Step 6: Commit** `feat(shell): menu contestuale nativo, proprietà e drag & drop`

### Task 3: Errori e percorsi

**Files:**
- Create: `src-tauri/src/error.rs`, `src-tauri/src/paths.rs`
- Test: test unitari nello stesso file (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub struct AppError { pub code: ErrorCode, pub path: Option<String>, pub detail: Option<String> }`: `Serialize` camelCase, `Debug`, `Clone`, `PartialEq`
  - `pub enum ErrorCode { NotFound, AccessDenied, AlreadyExists, DiskFull, NotReady, SameFile, IntoItself, InvalidName, Cancelled, InvalidRequest, Io }`, serializzato `SCREAMING_SNAKE_CASE`
  - `impl AppError { pub fn new(code, path: Option<&Path>) -> Self; pub fn from_io(err: &std::io::Error, path: &Path) -> Self }`
  - `pub fn to_extended(p: &Path) -> PathBuf`
  - `pub fn is_within(child: &Path, parent: &Path) -> bool`
  - `pub fn volume_key(p: &Path) -> String`

- [ ] **Step 1: Test**

```rust
#[test] fn io_codes_map() {
    let e = |n| AppError::from_io(&std::io::Error::from_raw_os_error(n), Path::new("x")).code;
    assert_eq!(e(2), ErrorCode::NotFound); assert_eq!(e(3), ErrorCode::NotFound);
    assert_eq!(e(5), ErrorCode::AccessDenied); assert_eq!(e(80), ErrorCode::AlreadyExists);
    assert_eq!(e(183), ErrorCode::AlreadyExists); assert_eq!(e(112), ErrorCode::DiskFull);
    assert_eq!(e(21), ErrorCode::NotReady); assert_eq!(e(123), ErrorCode::InvalidName);
    assert_eq!(e(9999), ErrorCode::Io);
}
#[test] fn extended_paths() {
    assert_eq!(to_extended(Path::new(r"C:\a\b")), PathBuf::from(r"\\?\C:\a\b"));
    assert_eq!(to_extended(Path::new(r"\\srv\sh\a")), PathBuf::from(r"\\?\UNC\srv\sh\a"));
    assert_eq!(to_extended(Path::new(r"\\?\C:\a")), PathBuf::from(r"\\?\C:\a"));
}
#[test] fn within_is_case_insensitive_and_component_based() {
    assert!(is_within(Path::new(r"D:\A\b"), Path::new(r"d:\a")));
    assert!(is_within(Path::new(r"D:\a"), Path::new(r"D:\a")));
    assert!(!is_within(Path::new(r"D:\ab"), Path::new(r"D:\a")));
}
#[test] fn volume_keys() {
    assert_eq!(volume_key(Path::new(r"c:\x")), "C:");
    assert_eq!(volume_key(Path::new(r"\\Srv\Share\x")), r"\\SRV\SHARE");
}
#[test] fn long_path_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = dir.path().to_path_buf();
    for _ in 0..30 { p.push("cartella_lunga"); }          // > 260 caratteri
    std::fs::create_dir_all(to_extended(&p)).unwrap();
    let f = to_extended(&p.join("nome con punto finale."));
    std::fs::write(&f, b"x").unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"x");
}
```

- [ ] **Step 2:** `cargo test --manifest-path src-tauri/Cargo.toml` → FAIL (simboli mancanti).
- [ ] **Step 3:** Implementare. `to_extended` lascia invariati i percorsi relativi. `is_within` normalizza i separatori e confronta i componenti senza distinguere maiuscole e minuscole.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(core): AppError e utilità sui percorsi`

### Task 4: Elenco cartelle e unità

**Files:**
- Create: `src-tauri/src/fs/mod.rs`, `src-tauri/src/fs/listing.rs`, `src-tauri/src/fs/drives.rs`
- Create: `src/ipc/types.ts`, `src/ipc/api.ts`
- Modify: `src-tauri/src/lib.rs`, `Cargo.toml` (feature `Win32_Storage_FileSystem`)

**Interfaces:**
- Consumes: `AppError`, `to_extended` (Task 3)
- Produces (Rust, comandi Tauri con gli stessi nomi):
  - `pub struct Entry { name: String, is_dir: bool, size: u64, modified_ms: i64, attributes: u32, is_symlink: bool }` (camelCase in JSON)
  - `pub fn list_dir(path: &Path) -> Result<Vec<Entry>, AppError>`, senza `..`
  - `pub fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf>`
  - `pub struct Drive { root: String, label: String, kind: DriveKind, total: u64, free: u64, ready: bool }`
  - `pub enum DriveKind { Fixed, Removable, Network, Cdrom, Other }`
  - `pub fn list_drives() -> Vec<Drive>`
- Produces (TS):
  - `src/ipc/types.ts`: `Entry`, `Drive`, `AppError`, `ErrorCode`, che rispecchiano i tipi Rust
  - `src/ipc/api.ts`: `listDir(path: string): Promise<Entry[]>`, `nearestExistingAncestor(path: string): Promise<string | null>`, `listDrives(): Promise<Drive[]>`

- [ ] **Step 1: Test** (in `listing.rs`)

```rust
#[test] fn lists_files_and_dirs_with_metadata() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("sub")).unwrap();
    std::fs::write(d.path().join("a.txt"), b"hello").unwrap();
    let mut v = list_dir(d.path()).unwrap(); v.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].name.as_str(), v[0].is_dir, v[0].size), ("a.txt", false, 5));
    assert!(v[1].is_dir && v[1].name == "sub");
    assert!(v[0].modified_ms > 0);
}
#[test] fn missing_dir_is_not_found() {
    assert_eq!(list_dir(Path::new(r"C:\non\esiste\proprio")).unwrap_err().code, ErrorCode::NotFound);
}
#[test] fn nearest_ancestor_walks_up() {
    let d = tempfile::tempdir().unwrap();
    let gone = d.path().join("x").join("y");
    assert_eq!(nearest_existing_ancestor(&gone).unwrap(), d.path());
}
#[test] fn drives_include_system_drive_and_never_hang() {
    let t = std::time::Instant::now();
    let v = list_drives();
    assert!(v.iter().any(|d| d.root.eq_ignore_ascii_case(r"C:\") && d.ready && d.total > 0));
    assert!(t.elapsed().as_secs() < 3);
}
```

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implementare:
  - `list_dir` usa `FindFirstFileExW` con `FindExInfoBasic` e `FIND_FIRST_EX_LARGE_FETCH` sul percorso esteso, perché è più veloce di `read_dir` + `metadata`. Per `modified_ms` si convertono i FILETIME in epoch ms.
  - `list_drives`: `GetLogicalDrives` + `GetDriveTypeW`. Solo per le unità pronte si chiamano `GetVolumeInformationW` e `GetDiskFreeSpaceExW`. Le unità di rete si interrogano in un thread con timeout di 1 s; se scade il timeout si restituisce `ready: false`.
- [ ] **Step 4:** `cargo test` → PASS. Esporre i comandi e scrivere i wrapper TS.
- [ ] **Step 5: Commit** `feat(fs): elenco cartelle e unità`

### Task 5: Watcher del file system

**Files:**
- Create: `src-tauri/src/fs/watch.rs`
- Modify: `src-tauri/src/lib.rs`, `src/ipc/api.ts`

**Interfaces:**
- Produces:
  - `pub struct Watchers` (stato Tauri gestito)
  - `pub fn watch(&self, key: String, path: PathBuf, on_change: impl Fn(String) + Send + 'static) -> Result<(), AppError>`
  - `pub fn unwatch(&self, key: &str)`
  - comandi `watch_dir(tab_id: String, path: String)` e `unwatch_dir(tab_id: String)`
  - evento `"fs-changed"` con payload `{ tabId: string }`
- TS: `watchDir(tabId, path)`, `unwatchDir(tabId)`, `onFsChanged(cb: (tabId: string) => void): Promise<UnlistenFn>`

- [ ] **Step 1: Test**

```rust
#[test] fn notifies_once_per_burst() {
    let d = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let w = Watchers::default();
    w.watch("t1".into(), d.path().into(), move |k| { tx.send(k).unwrap(); }).unwrap();
    for i in 0..20 { std::fs::write(d.path().join(format!("f{i}")), b"").unwrap(); }
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(), "t1");
    assert!(rx.recv_timeout(std::time::Duration::from_millis(400)).is_err()); // debounce
}
```

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implementare con `notify` (non ricorsivo, solo la cartella). Debounce di 150 ms per chiave: si invia una sola notifica al termine di un burst di eventi.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(fs): watcher con debounce`

### Task 6: Configurazione, impostazioni, temi e i18n

**Files:**
- Create: `src-tauri/src/config.rs`
- Create: `src/state/settings.ts`, `src/state/settings.test.ts`, `src/styles/tokens.css`, `src/i18n/index.ts`, `src/i18n/en.json`, `src/i18n/it.json`
- Modify: `src-tauri/src/lib.rs`, `src/main.tsx`, `src/ipc/api.ts`

**Interfaces:**
- Produces (Rust):
  - `pub fn config_dir(exe_dir: &Path, appdata: &Path) -> PathBuf`
  - comandi `read_config(name: String) -> Result<Option<serde_json::Value>, AppError>` e `write_config(name: String, value: serde_json::Value) -> Result<(), AppError>`
  - nomi ammessi: `"settings" | "keybindings" | "hotlist"`; ogni altro nome dà `INVALID_REQUEST`
- Produces (TS):
  - `interface Settings { theme: 'dark' | 'light' | 'system'; language: 'en' | 'it'; showHidden: boolean; showSystem: boolean; confirmDelete: boolean; externalEditor: string }`
  - `defaultSettings(osLocale: string): Settings`
  - `resolveTheme(theme: Settings['theme'], prefersDark: boolean): 'dark' | 'light'`
  - store Zustand `useSettings`, con `load()` e `update(patch)` che salva tramite `write_config`
  - `readConfig`, `writeConfig` in `api.ts`

- [ ] **Step 1: Test**

```rust
#[test] fn portable_marker_switches_dir() {
    let exe = tempfile::tempdir().unwrap(); let app = tempfile::tempdir().unwrap();
    assert_eq!(config_dir(exe.path(), app.path()), app.path().join("ModernCommander"));
    std::fs::write(exe.path().join("portable"), b"").unwrap();
    assert_eq!(config_dir(exe.path(), app.path()), exe.path().join("config"));
}
#[test] fn rejects_unknown_config_names() {
    assert_eq!(validate_name("../evil").unwrap_err().code, ErrorCode::InvalidRequest);
    assert!(validate_name("settings").is_ok());
}
```

```ts
test('defaults follow OS locale', () => {
  expect(defaultSettings('it-IT').language).toBe('it');
  expect(defaultSettings('de-DE').language).toBe('en');
  expect(defaultSettings('en-US')).toMatchObject({ theme: 'system', showHidden: false, showSystem: false, confirmDelete: true, externalEditor: 'notepad.exe' });
});
test('resolveTheme', () => {
  expect(resolveTheme('system', true)).toBe('dark');
  expect(resolveTheme('system', false)).toBe('light');
  expect(resolveTheme('light', true)).toBe('light');
});
test('en and it have identical key sets', () => {
  const keys = (o: object, p = ''): string[] => Object.entries(o).flatMap(([k, v]) => typeof v === 'object' ? keys(v, p + k + '.') : [p + k]);
  expect(keys(it).sort()).toEqual(keys(en).sort());
});
```

- [ ] **Step 2:** `cargo test` e `npm test` → FAIL.
- [ ] **Step 3:** Implementare:
  - `write_config` scrive su `<name>.json.tmp` e poi rinomina, per non lasciare il file a metà.
  - `tokens.css` definisce le variabili `--mc-*` sotto `[data-theme="dark"]` e `[data-theme="light"]` con i valori delle Global Constraints, comprese `--mc-row-height` e `--mc-radius`.
  - `main.tsx` imposta `document.documentElement.dataset.theme` da `resolveTheme` e ascolta `matchMedia('(prefers-color-scheme: dark)')`.
  - i18next si inizializza con la lingua delle impostazioni.
  - Il test sulle chiavi i18n resta per tutto il progetto.
- [ ] **Step 4:** `cargo test` e `npm test` → PASS.
- [ ] **Step 5: Commit** `feat(config): impostazioni, modalità portable, temi e i18n`

### Task 7: Guscio della finestra

**Files:**
- Create: `src/components/shell/TitleBar.tsx`, `ActivityBar.tsx`, `Sidebar.tsx`, `BottomPanel.tsx`, `FKeyBar.tsx`, `StatusBar.tsx`, `src/components/shell/shell.css`
- Create: `src/state/ui.ts`
- Modify: `src/App.tsx`

**Interfaces:**
- Produces:
  - `useUi` (Zustand) `{ sidebarVisible: boolean; sidebarView: 'explorer' | 'search' | 'git' | 'favorites' | 'settings'; bottomPanelVisible: boolean; bottomPanelTab: 'operations' | 'terminal'; toggleSidebar(); toggleBottomPanel(); setSidebarView(v) }`
  - in `App.tsx`, uno slot `GroupsArea` (riempito dal Task 11)

- [ ] **Step 1:** Comporre il layout della §5.1 dello spec con CSS grid:
  - **TitleBar**, alta 30 px:
    - area trascinabile `data-tauri-drag-region`
    - campo di ricerca centrale come segnaposto disabilitato (la palette arriva in fase 2)
    - interruttori per barra laterale e pannello in basso
    - minimizza, massimizza e chiudi via API finestra Tauri
  - **ActivityBar**, larga 38 px, con le icone Esplora/Cerca/Git/Preferiti/Impostazioni. In questa fase sono attive Esplora, Preferiti e Impostazioni; le altre sono nascoste.
  - **Sidebar**, larga 220 px e ridimensionabile.
  - **BottomPanel**, alto 180 px e ridimensionabile, con la sola scheda "Operazioni".
  - **FKeyBar** e **StatusBar** (22 px).

  Le icone sono un set monocromatico con licenza MIT/ISC (per esempio `@vscode/codicons` o Lucide, installato via npm). Il tema C ne colora solo le icone di tipo file (Task 11).
- [ ] **Step 2:** Verificare con `npm run tauri dev`:
  - struttura uguale alla bozza `layout-v2.html`, senza ispettore e senza terminale
  - la finestra si trascina dalla barra del titolo
  - gli interruttori mostrano e nascondono barra laterale e pannello
  - cambiando `theme` nelle impostazioni (a mano in `settings.json`) si passa da A a C
- [ ] **Step 3: Commit** `feat(ui): guscio della finestra`

### Task 8: Stato di gruppi e schede

**Files:**
- Create: `src/state/layout.ts`, `src/state/layout.test.ts`

**Interfaces:**
- Produces:

```ts
interface Tab { id: string; path: string }
interface Group { id: string; tabIds: string[]; activeTabId: string }
interface LayoutState { groups: Group[]; tabs: Record<string, Tab>; activeGroupId: string; activationOrder: string[] } // più recente per primo
initialLayout(paths: string[]): LayoutState            // un gruppo per path (default 2)
activateGroup(s, groupId): LayoutState
newTab(s, groupId, path): LayoutState                   // aggiunge e attiva
closeTab(s, tabId): LayoutState
activateTab(s, tabId): LayoutState
cycleTab(s, groupId, delta: 1 | -1): LayoutState
splitGroup(s, groupId): LayoutState                     // nuovo gruppo a destra con la stessa cartella, attivo
moveTab(s, tabId, toGroupId, index): LayoutState
setTabPath(s, tabId, path): LayoutState
targetGroup(s): string | null
focusAdjacentGroup(s, delta: 1 | -1): LayoutState       // Tab / Shift+Tab, circolare
```

  Tutte le funzioni sono pure e restituiscono un nuovo stato. Lo store `useLayout` le avvolge.

- [ ] **Step 1: Test** (scelte di dominio)

```ts
test('targetGroup is most recently active other group', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = splitGroup(s, s.groups[1].id);                   // 3 gruppi, attivo il terzo
  const [g1, g2, g3] = s.groups.map(g => g.id);
  s = activateGroup(activateGroup(s, g1), g3);
  expect(targetGroup(s)).toBe(g1);
  s = activateGroup(s, g2);
  expect(targetGroup(s)).toBe(g3);
});
test('targetGroup is null with a single group', () => {
  expect(targetGroup(initialLayout(['C:\\']))).toBeNull();
});
test('closing last tab removes the group, but never the last group', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = closeTab(s, s.groups[1].activeTabId);
  expect(s.groups).toHaveLength(1);
  const before = s; s = closeTab(s, s.groups[0].activeTabId);
  expect(s).toEqual(before);
});
test('moving the only tab out of a group removes that group', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = moveTab(s, s.groups[1].activeTabId, s.groups[0].id, 1);
  expect(s.groups).toHaveLength(1); expect(s.groups[0].tabIds).toHaveLength(2);
});
test('focusAdjacentGroup wraps', () => {
  let s = initialLayout(['C:\\', 'D:\\']);
  s = activateGroup(s, s.groups[1].id);
  expect(focusAdjacentGroup(s, 1).activeGroupId).toBe(s.groups[0].id);
});
```

- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implementare. Gli id si generano con `crypto.randomUUID()`.
- [ ] **Step 4:** `npm test` → PASS.
- [ ] **Step 5: Commit** `feat(state): gruppi e schede`

### Task 9: Stato della vista di una scheda e maschere

**Files:**
- Create: `src/lib/mask.ts`, `src/lib/mask.test.ts`, `src/state/tabView.ts`, `src/state/tabView.test.ts`

**Interfaces:**
- Produces:

```ts
matchMask(name: string, mask: string): boolean
type SortKey = 'name' | 'ext' | 'size' | 'date' | 'none'
interface Row extends Entry { isParent?: boolean }
interface TabView { rows: Row[]; cursor: number; marked: Set<string>; sort: { key: SortKey; desc: boolean };
  history: string[]; historyIndex: number; filter: string; view: 'full' | 'brief'; dirSizes: Record<string, number> }
buildRows(entries: Entry[], path: string, sort, opts: { showHidden: boolean; showSystem: boolean; filter: string }): Row[]
toggleMark(v, name): TabView        // '..' non è mai marcabile
markRange(v, from, to): TabView
selectByMask(v, mask, select: boolean): TabView   // solo file, come TC; '*.*' / '*' include le cartelle
invertMarks(v): TabView
navigateTo(v, path): TabView        // tronca la history in avanti
back(v) / forward(v): TabView
quickSearch(rows, prefix, from): number   // indice o -1, case-insensitive, circolare
parentPath(path): string | null     // null alla radice dell'unità
```

- [ ] **Step 1: Test**

```ts
test('mask semantics (TC)', () => {
  expect(matchMask('a.TXT', '*.txt')).toBe(true);
  expect(matchMask('a.md', '*.txt;*.md')).toBe(true);
  expect(matchMask('a.md', '*.txt *.md')).toBe(true);
  expect(matchMask('a.bak.txt', '*.txt|*.bak.*')).toBe(false);   // '|' esclude
  expect(matchMask('ab', 'a?')).toBe(true);
  expect(matchMask('noext', '*.*')).toBe(true);
});
test('rows: parent first, dirs before files, then sort', () => {
  const e = [f('b.txt', 2), d('zdir'), f('a.md', 9), d('adir')];
  const rows = buildRows(e, 'C:\\x', { key: 'size', desc: true }, opts);
  expect(rows.map(r => r.name)).toEqual(['..', 'adir', 'zdir', 'a.md', 'b.txt']);
});
test('no parent row at drive root', () => {
  expect(buildRows([], 'C:\\', sortName, opts)[0]).toBeUndefined();
});
test('hidden/system filtered unless enabled', () => {
  const rows = buildRows([f('h', 1, 0x2), f('s', 1, 0x4), f('n', 1)], 'C:\\x', sortName, opts);
  expect(rows.map(r => r.name)).toEqual(['..', 'n']);
});
test('parent row is never marked', () => {
  expect(toggleMark(view(['..', 'a']), '..').marked.size).toBe(0);
});
test('history truncates forward on navigate', () => {
  let v = navigateTo(navigateTo(emptyView('C:\\'), 'C:\\a'), 'C:\\b');
  v = navigateTo(back(v), 'C:\\c');
  expect(v.history).toEqual(['C:\\', 'C:\\a', 'C:\\c']);
});
test('quickSearch wraps', () => {
  expect(quickSearch(rowsNamed(['..', 'alpha', 'beta', 'abc']), 'a', 2)).toBe(3);
});
```

  (`f`, `d`, `opts`, `sortName`, `view`, `emptyView`, `rowsNamed` sono helper locali al file di test.)
- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implementare. Ordinamento per nome con `Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })`. Attributi: hidden `0x2`, system `0x4`.
- [ ] **Step 4:** `npm test` → PASS.
- [ ] **Step 5: Commit** `feat(state): vista scheda, ordinamento, marcatura e maschere`

### Task 10: Comandi e mappa tasti

**Files:**
- Create: `src/commands/registry.ts`, `src/commands/keys.ts`, `src/commands/keys.test.ts`, `src/commands/tcKeymap.ts`, `src/commands/keymap.ts`, `src/commands/keymap.test.ts`, `src/commands/useKeyboard.ts`

**Interfaces:**
- Produces:

```ts
interface Command { id: string; titleKey: string; when: 'fileList' | 'global'; run(): void | Promise<void> }
registerCommand(c: Command): void; executeCommand(id: string): Promise<void>; getCommands(): Command[]
chordFromEvent(e: { keyCode: number; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }): string
interface KeybindingOverride { key: string; command: string }     // command '-id' = rimuovi
buildKeymap(defaults: Record<string, string>, overrides: KeybindingOverride[]): Map<string, string>
DEFAULT_KEYMAP: Record<string, string>   // chord -> command id
```

  Gli id dei comandi sono **i nomi interni di TC** (`cm_*`) quando esistono, così l'import da `wincmd.ini` in fase 5 è diretto; altrimenti si usa il prefisso `mc_`. I nomi `cm_*` vanno verificati sulla documentazione TC (`TOTALCMD.INC`/help "Keyboard"): se un nome non esiste, si usa `mc_`.

  Mappa di default della fase 1 (chord → id):

  | Tasti | Comando | Tasti | Comando |
  |---|---|---|---|
  | F3 | cm_List | F4 | cm_Edit |
  | F5 | cm_Copy | Shift+F5 | cm_CopySamepanel |
  | F6 | cm_RenMov | Shift+F6 | cm_RenameOnly |
  | F7 | cm_MkDir | F8, Delete | cm_Delete |
  | Shift+F8, Shift+Delete | mc_DeletePermanent | Alt+F4 | cm_Exit |
  | Tab / Shift+Tab | mc_FocusNextGroup / mc_FocusPrevGroup | Enter | cm_Return |
  | Backspace, Ctrl+PageUp | cm_GoToParent | Ctrl+Oem5 (Ctrl+\\) | cm_GoToRoot |
  | Alt+Left / Alt+Right | cm_GotoPreviousDir / cm_GotoNextDir | Alt+Down | cm_DirectoryHistory |
  | Insert | cm_Reverse | Space | mc_MarkOrSize |
  | NumAdd / NumSubtract / NumMultiply | cm_SpreadSelection / cm_ShrinkSelection / cm_ExchangeSelection | Ctrl+A, Ctrl+NumAdd / Ctrl+NumSubtract | cm_SelectAll / cm_ClearAll |
  | Ctrl+T / Ctrl+W | cm_OpenNewTab / cm_CloseCurrentTab | Ctrl+Tab / Ctrl+Shift+Tab | cm_SwitchToNextTab / cm_SwitchToPreviousTab |
  | Ctrl+R | cm_RereadSource | Ctrl+D | cm_DirectoryHotlist |
  | Ctrl+F1 / Ctrl+F2 | cm_SrcShort / cm_SrcLong | Ctrl+F3…F7 | cm_SrcByName / ByExt / ByDateTime / BySize / Unsorted |
  | Ctrl+S | cm_QuickFilter | Ctrl+U | cm_Exchange |
  | Ctrl+Left / Ctrl+Right | mc_OpenInPrevGroup / mc_OpenInNextGroup | Alt+F1 | cm_SrcOpenDrives |
  | Alt+Enter | cm_Properties | Shift+F10 | cm_ContextMenu |
  | Alt+Shift+Enter | cm_CountDirContent | Ctrl+Shift+E | mc_ToggleSidebar |
  | Ctrl+Oem3 (Ctrl+ò) | mc_ToggleBottomPanel | Ctrl+Comma | cm_Config |

  La vista miniature (`cm_SrcThumbs`, Ctrl+Shift+F1) passa alla fase 2, perché usa la stessa infrastruttura di anteprime dell'ispettore. In fase 1 `TabView.view` può valere solo `'full'` o `'brief'`.

  `mc_SplitGroup` = Ctrl+Shift+Oem5 (Ctrl+Shift+\\), scelto in T10 perché Ctrl+\\ è `cm_GoToRoot`. Gli id sono stati verificati in T10 sulla lista ufficiale dei comandi TC; quelli `cm_*` di questo piano sono già i nomi definitivi. Le lettere senza modificatori e Ctrl+Alt+lettera vanno alla ricerca rapida (Task 11), non a un comando.

- [ ] **Step 1: Test**

```ts
test('chord normalization uses virtual keys', () => {
  expect(chordFromEvent({ keyCode: 192, ctrlKey: true, shiftKey: false, altKey: false })).toBe('Ctrl+Oem3');
  expect(chordFromEvent({ keyCode: 116, ctrlKey: false, shiftKey: true, altKey: false })).toBe('Shift+F5');
  expect(chordFromEvent({ keyCode: 107, ctrlKey: false, shiftKey: false, altKey: false })).toBe('NumAdd');
  expect(chordFromEvent({ keyCode: 13, ctrlKey: false, shiftKey: true, altKey: true })).toBe('Shift+Alt+Enter');
  expect(chordFromEvent({ keyCode: 65, ctrlKey: true, shiftKey: true, altKey: true })).toBe('Ctrl+Shift+Alt+A');
});
test('overrides replace and remove', () => {
  const m = buildKeymap({ F5: 'cm_Copy', F6: 'cm_RenMov' }, [{ key: 'Ctrl+K', command: 'cm_Copy' }, { key: 'F6', command: '-cm_RenMov' }]);
  expect(m.get('Ctrl+K')).toBe('cm_Copy'); expect(m.get('F5')).toBe('cm_Copy'); expect(m.has('F6')).toBe(false);
});
test('every default command id is namespaced', () => {
  for (const id of Object.values(DEFAULT_KEYMAP)) expect(id).toMatch(/^(cm|mc)_/);
});
```

- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implementare:
  - L'ordine dei modificatori è sempre `Ctrl+Shift+Alt+`.
  - Nomi dei tasti: `A`–`Z`, `0`–`9`, `F1`–`F24`, `Enter`, `Tab`, `Space`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, `Left`, `Right`, `Up`, `Down`, `Escape`, `NumAdd`(107), `NumSubtract`(109), `NumMultiply`(106), `Comma`(188), `Oem3`(192), `Oem5`(220). Per tutti gli altri si usa `VK<n>`.
  - `useKeyboard` installa un listener `keydown` su `window`. Se il focus è in un input o in una finestra di dialogo, esegue solo i comandi `global`. Quando trova un binding chiama `preventDefault`, compreso Alt+F4, che deve passare da `cm_Exit`.
  - Il keymap si costruisce da `DEFAULT_KEYMAP` + `readConfig('keybindings')`.
- [ ] **Step 4:** `npm test` → PASS.
- [ ] **Step 5: Commit** `feat(commands): registro comandi e mappa tasti TC`

### Task 11: Gruppi, schede e lista file nella UI

**Files:**
- Create: `src/components/groups/GroupsArea.tsx`, `Group.tsx`, `TabStrip.tsx`, `Breadcrumb.tsx`, `FileList.tsx`, `FileIcon.tsx`, `groups.css`
- Create: `src/commands/navigation.ts` (registra i comandi di navigazione, selezione, schede, ordinamento e vista)
- Create: `src/state/tabViews.ts` (store Zustand `Record<tabId, TabView>` + azioni async `load(tabId, path)`)
- Create: `scripts/make-test-dir.ps1`

**Interfaces:**
- Consumes: Task 4 `listDir`/`nearestExistingAncestor`/`listDrives`, Task 5 `watchDir`/`onFsChanged`, Task 8 `useLayout`, Task 9 funzioni di `TabView`, Task 10 `registerCommand`
- Produces:
  - `useTabViews` con `load(tabId: string, path: string): Promise<void>`
  - `activeTabContext(): { tabId: string; path: string; view: TabView; cursorRow: Row | undefined; markedOrCursor: Row[] }`, che usano tutti i comandi successivi

- [ ] **Step 1:** `load`:
  1. chiama `listDir`
  2. se l'errore è `NOT_FOUND` o `NOT_READY`, chiama `nearestExistingAncestor` e carica quel percorso, mostrando una notifica tradotta in `errors.<CODE>`
  3. se nessun antenato esiste, mostra la lista delle unità come contenuto della scheda (percorso speciale `drives:`)

  Inoltre `load` aggiorna `watchDir`. Su `fs-changed` ricarica conservando il cursore sul nome corrente.
- [ ] **Step 2:** Test in `src/state/tabViews.test.ts`, con `api` simulato tramite `vi.mock('../ipc/api')`:

```ts
test('missing folder falls back to nearest ancestor', async () => {
  listDir.mockRejectedValueOnce({ code: 'NOT_FOUND' }).mockResolvedValueOnce([]);
  nearestExistingAncestor.mockResolvedValue('D:\\a');
  await useTabViews.getState().load('t1', 'D:\\a\\gone');
  expect(useLayout.getState().tabs['t1'].path).toBe('D:\\a');
});
test('no ancestor shows drives', async () => {
  listDir.mockRejectedValue({ code: 'NOT_READY' }); nearestExistingAncestor.mockResolvedValue(null);
  await useTabViews.getState().load('t1', 'E:\\x');
  expect(useLayout.getState().tabs['t1'].path).toBe('drives:');
});
test('reload keeps cursor on the same name', async () => { /* rows a,b,c con cursore su 'b'; il nuovo listing inserisce 'aa' → il cursore resta su 'b' */ });
```

- [ ] **Step 3:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 4:** UI:
  - `FileList` virtualizzata con `@tanstack/react-virtual`, righe alte `var(--mc-row-height)`.
  - Colonne: icona, Nome, Est., Dim. (`‹dir›` o la dimensione calcolata), Modificato, Attributi.
  - Click sull'intestazione = ordinamento.
  - Il cursore ha lo sfondo `--mc-selection`; i nomi marcati hanno il colore `--mc-accent`.
  - Mouse: click = cursore; Ctrl+click = marca; Shift+click = intervallo; doppio click = `cm_Return`.
  - `TabStrip`: schede con il nome della cartella, "+", chiusura con click centrale, drag per riordinare e per spostare tra gruppi (HTML5 DnD).
  - `Breadcrumb`: segmenti cliccabili; click sullo spazio vuoto → input modificabile, Invio naviga, Esc annulla.
  - `GroupsArea`: gruppi affiancati con separatori trascinabili, larghezza minima 240 px.
  - Gruppo inattivo attenuato come nella bozza.
  - Ricerca rapida: lettere e Ctrl+Alt+lettera → popup con il prefisso, cursore su `quickSearch`, Esc chiude.
  - `cm_QuickFilter` (Ctrl+S) apre un input che imposta `filter`.
  - `cm_Return`:
    - su una cartella naviga
    - su `..` va alla cartella superiore
    - su un file chiama `open_path` (Task 18)
  - `cm_SrcOpenDrives` apre un menu con le unità per il gruppo attivo.
  - `cm_DirectoryHistory` (Alt+↓) apre un menu con la `history` della scheda, la più recente in alto.
  - `cm_Exchange` (Ctrl+U) scambia le cartelle della scheda attiva e della scheda attiva di `targetGroup`; con un solo gruppo non fa niente.
  - `mc_OpenInPrevGroup` / `mc_OpenInNextGroup` (Ctrl+←/→) aprono la cartella sotto il cursore, o quella corrente se il cursore è su un file, nella scheda attiva del gruppo adiacente.
- [ ] **Step 5: Verifica prestazioni.** `scripts/make-test-dir.ps1 -Path D:\mc-test -Count 100000` crea N file vuoti, più una sottocartella `10k` con 10.000 file.
  - Aprendo `D:\mc-test\10k`, il tempo da `load` al primo frame con righe (misurato con `performance.now()` in dev, log `[perf] list`) deve essere < 150 ms.
  - Su `D:\mc-test` lo scorrimento deve restare fluido e la navigazione da tastiera non deve avere ritardi visibili.
  - Se si superano i 150 ms, profilare prima di proseguire.
- [ ] **Step 6:** Verifica manuale:
  - tutte le scorciatoie della tabella del Task 10 legate a navigazione, selezione, schede, ordinamento e vista funzionano
  - eliminando da Explorer la cartella aperta, la scheda risale all'antenato
  - togliendo una chiavetta USB aperta in una scheda, la scheda passa alle unità
- [ ] **Step 7: Commit** `feat(ui): gruppi, schede e lista file`

### Task 12: Barra laterale e hotlist

**Files:**
- Create: `src/state/hotlist.ts`, `src/state/hotlist.test.ts`, `src/components/sidebar/ExplorerView.tsx`, `FolderTree.tsx`, `FavoritesView.tsx`, `src/components/dialogs/HotlistMenu.tsx`

**Interfaces:**
- Produces:

```ts
type HotlistItem = { type: 'dir'; id: string; name: string; path: string }
  | { type: 'submenu'; id: string; name: string; items: HotlistItem[] } | { type: 'separator'; id: string }
addDir(list: HotlistItem[], path: string, name: string, parentId?: string): HotlistItem[]
removeItem(list, id): HotlistItem[]; renameItem(list, id, name): HotlistItem[]; moveItem(list, id, toParentId: string | null, index: number): HotlistItem[]
```

  La hotlist si salva con `writeConfig('hotlist', items)`.

- [ ] **Step 1: Test:** `addDir` dentro un sottomenu annidato; `removeItem` di un sottomenu rimuove anche i figli; `moveItem` da un sottomenu alla radice; `moveItem` di un sottomenu dentro se stesso → lista invariata.
- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** UI:
  - `ExplorerView`:
    - sezione Unità (`listDrives`, etichetta e spazio libero, aggiornata ogni 10 s e al focus della finestra)
    - `FolderTree` a caricamento pigro (`listDir` filtrata sulle cartelle all'espansione)
    - sezione Preferiti
  - Un click su un nodo chiama `load(activeTab, path)` (spec §5.1 **[U]**).
  - `cm_DirectoryHotlist` (Ctrl+D) apre `HotlistMenu` alla posizione del cursore, con:
    - le voci
    - "Aggiungi cartella corrente"
    - "Configura…", che apre `FavoritesView` in modalità modifica: rinomina, elimina, trascina, crea sottomenu
- [ ] **Step 4:** Verifica manuale: un click sull'albero apre la cartella nella scheda attiva del gruppo attivo; Ctrl+D → aggiungi → la voce compare nella vista Preferiti e c'è ancora dopo il riavvio.
- [ ] **Step 5: Commit** `feat(ui): barra laterale e hotlist`

### Task 13: Modello delle operazioni, espansione e conflitti

**Files:**
- Create: `src-tauri/src/ops/mod.rs`, `ops/model.rs`, `ops/plan.rs`, `ops/conflict.rs`, `src-tauri/src/mask.rs`

**Interfaces:**
- Consumes: `AppError`, `is_within`, `to_extended` (Task 3)
- Produces:

```rust
pub enum JobKind { Copy, Move, Delete { permanent: bool }, Mkdir, Rename }
pub enum ConflictPolicy { Ask, Overwrite, Skip, OverwriteIfNewer, OverwriteIfLarger, RenameNew }
pub struct JobSpec { pub kind: JobKind, pub sources: Vec<PathBuf>, pub dest: Option<PathBuf>, pub filter: Option<String>, pub policy: ConflictPolicy }
pub struct FileItem { pub src: PathBuf, pub dst: Option<PathBuf>, pub is_dir: bool, pub size: u64 }
pub fn expand(spec: &JobSpec) -> Result<Vec<FileItem>, AppError>
pub struct Meta { pub size: u64, pub modified_ms: i64 }
pub enum Decision { Overwrite, Skip, Rename(PathBuf), Cancel }
pub fn resolve(policy: &ConflictPolicy, src: &Path, dst: &Path, src_meta: &Meta, dst_meta: &Meta) -> Result<Option<Decision>, AppError> // None = chiedere all'utente
pub fn next_free_name(dst: &Path) -> PathBuf
pub fn match_mask(name: &str, mask: &str) -> bool   // stessa semantica di src/lib/mask.ts
```

  Tutti i tipi sono `Serialize`/`Deserialize` in camelCase, con tag `"type"` sugli enum.

  - Per Copy/Move `dest` è la cartella di destinazione. Per Rename è il nuovo percorso completo. Per Mkdir è `sources[0]`.
  - L'espansione segue l'ordine: per Copy/Move la cartella prima del suo contenuto, per Delete il contenuto prima della cartella.
  - `filter` si applica solo ai file; le cartelle che restano vuote dopo il filtro non si creano.
  - Uno spostamento sulla stessa unità non viene espanso: produce un solo `FileItem` per sorgente, perché è un rename.

- [ ] **Step 1: Test**

```rust
#[test] fn copy_into_itself_is_rejected() {
    let d = tempfile::tempdir().unwrap(); let a = d.path().join("a"); std::fs::create_dir_all(a.join("b")).unwrap();
    let spec = JobSpec { kind: JobKind::Copy, sources: vec![a.clone()], dest: Some(a.join("b")), filter: None, policy: ConflictPolicy::Ask };
    assert_eq!(expand(&spec).unwrap_err().code, ErrorCode::IntoItself);
}
#[test] fn copy_expands_dirs_before_children_and_maps_dest() {
    /* a/{x.txt, s/y.txt} → dest D: elementi [a, a/x.txt, a/s, a/s/y.txt], dst = D/a/... */
}
#[test] fn delete_expands_children_before_dirs() { /* l'ultimo elemento è la cartella radice */ }
#[test] fn filter_applies_to_files_only() { /* filtro '*.txt' su a/{x.txt, y.md, s/z.md} → nessun y.md e nessuna cartella 's' */ }
#[test] fn same_file_never_overwritten() {
    let p = Path::new(r"C:\x\f.txt"); let m = Meta { size: 1, modified_ms: 1 };
    assert_eq!(resolve(&ConflictPolicy::Overwrite, p, p, &m, &m).unwrap_err().code, ErrorCode::SameFile);
    assert!(matches!(resolve(&ConflictPolicy::RenameNew, p, p, &m, &m).unwrap(), Some(Decision::Rename(_))));
}
#[test] fn policies() {
    let (old, new) = (Meta { size: 1, modified_ms: 1 }, Meta { size: 2, modified_ms: 2 });
    let (s, d) = (Path::new(r"C:\a\f"), Path::new(r"D:\a\f"));
    assert_eq!(resolve(&ConflictPolicy::OverwriteIfNewer, s, d, &new, &old).unwrap(), Some(Decision::Overwrite));
    assert_eq!(resolve(&ConflictPolicy::OverwriteIfNewer, s, d, &old, &new).unwrap(), Some(Decision::Skip));
    assert_eq!(resolve(&ConflictPolicy::OverwriteIfLarger, s, d, &old, &new).unwrap(), Some(Decision::Skip));
    assert_eq!(resolve(&ConflictPolicy::Ask, s, d, &old, &new).unwrap(), None);
}
#[test] fn free_name() {
    let d = tempfile::tempdir().unwrap(); let f = d.path().join("f.txt");
    std::fs::write(&f, b"").unwrap(); std::fs::write(d.path().join("f (2).txt"), b"").unwrap();
    assert_eq!(next_free_name(&f), d.path().join("f (3).txt"));
}
#[test] fn mask_matches_ts_semantics() { /* stessi sei casi di src/lib/mask.test.ts */ }
```

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implementare. "Stesso file" si determina con `std::fs::canonicalize` di entrambi; se la destinazione non esiste si confronta il percorso normalizzato.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(ops): modello, espansione e politiche di conflitto`

### Task 14: Esecutore locale

**Files:**
- Create: `src-tauri/src/ops/exec.rs`
- Modify: `Cargo.toml` (feature `Win32_Storage_FileSystem`, già presente)

**Interfaces:**
- Consumes: `AppError`, `to_extended`
- Produces:

```rust
pub trait Executor: Send + Sync {
    fn copy_file(&self, src: &Path, dst: &Path, overwrite: bool, progress: &mut dyn FnMut(u64) -> bool) -> Result<(), AppError>; // false = annulla
    fn move_path(&self, src: &Path, dst: &Path, overwrite: bool) -> Result<(), AppError>;      // solo stessa unità
    fn delete(&self, path: &Path, is_dir: bool, permanent: bool) -> Result<(), AppError>;
    fn mkdir(&self, path: &Path) -> Result<(), AppError>;
    fn rename(&self, src: &Path, dst: &Path) -> Result<(), AppError>;
}
pub struct LocalExecutor;
pub const PART_SUFFIX: &str = ".mcpart";
```

- [ ] **Step 1: Test**

```rust
#[test] fn copy_writes_full_file_and_reports_progress() { /* 3 MB; progress chiamato più volte; l'ultimo valore è 3 MB; contenuti uguali */ }
#[test] fn cancelled_copy_leaves_nothing_at_destination() {
    /* progress restituisce false alla prima chiamata → Err(Cancelled); né dst né dst+".mcpart" esistono */
}
#[test] fn copy_without_overwrite_fails_on_existing() { /* Err(AlreadyExists); dst invariato */ }
#[test] fn overwrite_replaces_atomically() { /* dst preesistente "old" → dopo la copia contiene i nuovi byte, nessun .mcpart residuo */ }
#[test] fn long_path_and_trailing_dot_copy_and_delete() { /* percorso > 260 + nome "x." : copy_file, poi delete permanent → non esiste più */ }
#[test] fn delete_to_recycle_bin_removes_from_disk() { /* delete(permanent=false) su un file temporaneo → !exists */ }
#[test] fn readonly_file_permanent_delete() { /* file read-only: delete(permanent=true) → Ok: l'attributo viene tolto prima */ }
```

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implementare:
  - `copy_file`:
    1. `CopyFileExW(src_ext, dst_ext + PART_SUFFIX, callback, cancel_flag, COPY_FILE_FAIL_IF_EXISTS)`; il callback chiama `progress(total_transferred)` e restituisce `PROGRESS_CANCEL` se riceve false
    2. poi `MoveFileExW(part, dst, MOVEFILE_REPLACE_EXISTING se overwrite)`
    3. in caso di errore o annullamento elimina il `.mcpart`
    4. se `!overwrite` e `dst` esiste, restituisce subito `AlreadyExists`
  - `delete(permanent=false)` usa il crate `trash`.
  - `delete(permanent=true)` toglie l'attributo read-only e poi chiama `remove_file` / `remove_dir` (le cartelle si cancellano già vuote, perché `expand` mette i figli prima).
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(ops): esecutore locale con copia atomica`

### Task 15: Coda operazioni

**Files:**
- Create: `src-tauri/src/ops/queue.rs`, `src-tauri/src/ops/runner.rs`
- Modify: `src-tauri/src/lib.rs`, `src/ipc/types.ts`, `src/ipc/api.ts`

**Interfaces:**
- Consumes: Task 13 (`expand`, `resolve`, `next_free_name`), Task 14 (`Executor`)
- Produces (Rust):

```rust
pub type JobId = u64;
pub enum JobState { Queued, Running, Paused, Waiting, Done, Failed, Cancelled }
pub struct JobView { id: JobId, kind: JobKind, state: JobState, files_done: u64, files_total: u64, bytes_done: u64, bytes_total: u64,
                     current: Option<String>, speed_bps: u64, volumes: Vec<String>, errors: Vec<AppError> }
pub fn next_runnable(jobs: &[JobView], busy: &HashSet<String>) -> Option<JobId>
pub enum Prompt { Conflict { prompt_id: u64, job_id: JobId, src: String, dst: String, src_meta: Meta, dst_meta: Meta },
                  Error { prompt_id: u64, job_id: JobId, error: AppError, can_elevate: bool } }
pub enum PromptReply { Conflict { decision: Decision, apply_to_all: bool }, Error { action: ErrorAction, apply_to_all: bool } }
pub enum ErrorAction { Retry, Skip, Cancel, RetryElevated }
```

  Comandi:
  - `enqueue_job(spec: JobSpec) -> JobId`
  - `pause_job(id)`, `resume_job(id)`, `cancel_job(id)`
  - `move_job(id, index: usize)`
  - `reply_prompt(prompt_id: u64, reply: PromptReply)`
  - `list_jobs() -> Vec<JobView>`

  Eventi:
  - `"job-updated"` (`JobView`), al massimo ogni 100 ms per job, più uno sempre al cambio di stato
  - `"job-prompt"` (`Prompt`)
- Produces (TS): wrapper con gli stessi nomi in camelCase, più `onJobUpdated(cb)` e `onJobPrompt(cb)`

- [ ] **Step 1: Test**

```rust
#[test] fn scheduler_serializes_per_volume_and_parallelizes_across() {
    let j = |id, v: &[&str], st| view(id, v, st);
    let jobs = [j(1, &["C:", "D:"], Running), j(2, &["D:"], Queued), j(3, &["E:"], Queued)];
    let busy: HashSet<_> = ["C:".into(), "D:".into()].into();
    assert_eq!(next_runnable(&jobs, &busy), Some(3));
}
#[test] fn scheduler_respects_queue_order() { /* due job in coda su volumi liberi → restituisce il primo nell'ordine */ }
#[test] fn conflict_prompt_roundtrip_with_apply_to_all() {
    /* runner con executor locale e due file in conflitto, policy Ask: arriva un Prompt::Conflict; risposta Skip + apply_to_all;
       nessun secondo prompt; job Done; i file di destinazione sono invariati */
}
#[test] fn move_across_volumes_deletes_source_only_after_successful_copy() {
    /* executor finto con copy_file che fallisce al 2° file: il sorgente del 2° file esiste ancora, quello del 1° è stato rimosso */
}
#[test] fn cancel_stops_after_current_file() { /* cancel_job durante il primo di 3 file → stato Cancelled, file 2 e 3 non copiati */ }
```

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implementare:
  - Un thread scheduler con `Mutex<Vec<Job>>` + `Condvar`. Ogni job avviato gira in un suo thread worker.
  - I volumi di un job sono `volume_key` delle sorgenti e della destinazione.
  - Il runner applica le regole:
    - Move sulla stessa unità → `move_path`
    - Move tra unità diverse → `copy_file` + `delete(permanent=true)` del sorgente; le cartelle sorgente si rimuovono alla fine, se vuote
    - i conflitti si risolvono con `resolve`; se serve chiedere, si invia un `Prompt` e il worker aspetta la risposta (stato `Waiting`)
    - gli errori diventano `Prompt::Error`; `can_elevate` è true quando il codice è `ACCESS_DENIED`
  - Pausa: si controlla tra un file e l'altro e durante la copia tramite il callback di avanzamento.
  - Velocità: media mobile su 2 s.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(ops): coda con scheduler per volume, conflitti ed errori`

### Task 16: UI delle operazioni

**Files:**
- Create: `src/components/dialogs/CopyMoveDialog.tsx`, `ConflictDialog.tsx`, `ErrorDialog.tsx`, `MkdirDialog.tsx`, `DeleteConfirm.tsx`, `MaskDialog.tsx`, `src/components/panel/OperationsView.tsx`, `src/state/jobs.ts`, `src/lib/format.ts`, `src/lib/format.test.ts`, `src/commands/fileOps.ts`

**Interfaces:**
- Consumes: Task 15 API ed eventi; Task 8 `targetGroup`; Task 11 `activeTabContext`
- Produces:
  - `formatSize(bytes: number, lang: 'en' | 'it'): string`
  - `formatEta(seconds: number): string`
  - `useJobs` (`Record<JobId, JobView>` + la coda dei prompt in attesa)
  - i comandi `cm_Copy`, `cm_CopySamepanel`, `cm_RenMov`, `cm_RenameOnly`, `cm_MkDir`, `cm_Delete`, `mc_DeletePermanent`, `cm_SpreadSelection`, `cm_ShrinkSelection`

- [ ] **Step 1: Test:** `formatSize(0,'it') === '0 B'`, `formatSize(1536,'it') === '1,5 KB'`, `formatSize(1536,'en') === '1.5 KB'`, `formatSize(2.4*1024**3,'it') === '2,4 GB'`; `formatEta(65) === '1:05'`, `formatEta(3725) === '1:02:05'`.
- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** `CopyMoveDialog`:
  - destinazione modificabile, precompilata con il percorso di `targetGroup`
  - con un solo gruppo: precompilata con la cartella corrente e pulsante OK disabilitato finché non la si cambia
  - con 3 o più gruppi: elenco dei gruppi selezionabili con 1…9
  - filtro maschera e politica di conflitto di default
  - Invio conferma, Esc annulla
- [ ] **Step 4:** `ConflictDialog` mostra sorgente e destinazione con dimensione e data. Pulsanti: Sovrascrivi, Salta, Rinomina, Solo se più recente, Sovrascrivi se più grande, Annulla, con la casella "Applica a tutti". `ErrorDialog` mostra `errors.<CODE>` e il percorso, con Riprova / Salta / Annulla, più "Riprova come amministratore" se `canElevate` (collegato nel Task 17).
- [ ] **Step 5:** Comandi e dialoghi:
  - `cm_RenameOnly` (Shift+F6) attiva la modifica in place del nome sulla riga.
  - `cm_MkDir` apre `MkdirDialog`; un percorso relativo con `\` crea le cartelle intermedie. Dopo la creazione il cursore va sulla nuova cartella.
  - `cm_Delete` apre `DeleteConfirm` (se `confirmDelete`), che dice "nel cestino" oppure "definitivamente".
  - Num+ e Num- aprono `MaskDialog`.
- [ ] **Step 6:** `OperationsView` elenca i job con barra di avanzamento, velocità, ETA, pausa/riprendi/annulla, riordino con drag e gli errori del riepilogo finale. Ogni nuovo job apre automaticamente il pannello in basso. `StatusBar` mostra il job in corso (Task 20).
- [ ] **Step 7:** Il drop da Explorer su un gruppo (`onDragDropEvent`, dal Task 2) apre `CopyMoveDialog` con le sorgenti trascinate e la destinazione uguale alla cartella del gruppo sotto il puntatore.
- [ ] **Step 8:** Verifica manuale:
  - F5 di 2 GB tra due unità: la pausa funziona
  - annullando la copia non resta nessun `.mcpart`
  - F5 nella stessa cartella → conflitto senza "Sovrascrivi"; "Rinomina" crea `f (2).txt`
  - F6 di una cartella dentro se stessa → errore `INTO_ITSELF` tradotto
  - due copie su unità diverse procedono in parallelo; due copie sulla stessa unità vanno in sequenza
- [ ] **Step 9: Commit** `feat(ui): finestre e coda delle operazioni`

### Task 17: Helper elevato (UAC)

**Files:**
- Create: `src-tauri/src/elevation/mod.rs`, `elevation/protocol.rs`, `elevation/helper.rs`, `elevation/client.rs`, `src-tauri/tests/elevation_pipe.rs`
- Modify: `src-tauri/src/main.rs` (se `args[1] == "--elevated-helper"` → `elevation::helper::run(pipe, token)` e uscita, senza avviare Tauri), `src-tauri/src/ops/runner.rs` (`RetryElevated`), `Cargo.toml` (feature `Win32_System_Pipes`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Threading`)

**Interfaces:**
- Consumes: `Executor`, `LocalExecutor` (Task 14), `ErrorAction::RetryElevated` (Task 15)
- Produces:

```rust
pub enum Request { Hello { token: String }, Copy { src: PathBuf, dst: PathBuf, overwrite: bool }, Move { src: PathBuf, dst: PathBuf, overwrite: bool },
                   Delete { path: PathBuf, is_dir: bool, permanent: bool }, Mkdir { path: PathBuf }, Rename { src: PathBuf, dst: PathBuf }, Cancel }
pub enum Response { Progress { bytes: u64 }, Ok, Err(AppError) }
pub fn validate(req: &Request) -> Result<(), AppError>
pub struct ElevatedExecutor  // impl Executor
impl ElevatedExecutor { pub fn spawn() -> Result<Self, AppError>; fn spawn_with(launch: impl FnOnce(&str, &str) -> Result<u32, AppError>) -> Result<Self, AppError> }
```

  Protocollo: JSON a righe sulla named pipe. Il primo messaggio deve essere `Hello` con il token, altrimenti la connessione si chiude.

- [ ] **Step 1: Test**

```rust
#[test] fn validate_rejects_unsafe_paths() {
    for p in [r"relative\x", r"C:\a\..\Windows", r"\\.\PhysicalDrive0", r"\\?\GLOBALROOT\x", ""] {
        assert_eq!(validate(&Request::Mkdir { path: p.into() }).unwrap_err().code, ErrorCode::InvalidRequest, "{p}");
    }
    assert!(validate(&Request::Mkdir { path: r"C:\Program Files\X".into() }).is_ok());
    assert!(validate(&Request::Mkdir { path: r"\\srv\share\x".into() }).is_ok());
}
```

  `tests/elevation_pipe.rs`, con un helper avviato **senza** elevazione tramite `spawn_with`, che lancia l'eseguibile di test con `--elevated-helper`:
  - `helper_executes_mkdir_and_copy`: `mkdir` e `copy_file` in un tempdir → i file esistono
  - `wrong_token_is_rejected`: un client con token errato → connessione chiusa, nessuna operazione eseguita
  - `foreign_process_cannot_connect`: un secondo client (un altro processo) prova ad aprire la pipe → fallisce, perché il server accetta solo il PID lanciato
  - `helper_exits_when_idle`: con il timeout di inattività impostato a 1 s nei test → il processo termina

- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implementare:
  - **App = server della pipe**:
    - nome `\\.\pipe\mc-elev-<uuid>`
    - `CreateNamedPipeW` con `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE`, 1 istanza
    - security descriptor SDDL `D:P(A;;GA;;;<SID utente corrente>)(A;;GA;;;BA)`; `BA` serve quando l'amministratore UAC è un altro account
  - **Lancio:** `ShellExecuteExW` con verbo `runas`, `SEE_MASK_NOCLOSEPROCESS` e argomenti `--elevated-helper <pipe> <token>`. Il token è di 32 byte casuali (`getrandom` o `BCryptGenRandom`) in esadecimale.
  - **Verifica del client:** dopo `ConnectNamedPipe`, `GetNamedPipeClientProcessId` deve coincidere con `GetProcessId(hProcess)` del processo lanciato, altrimenti si disconnette.
  - Se l'utente rifiuta il prompt UAC, si restituisce `AppError{ code: AccessDenied, detail: "uac_declined" }`.
  - **Helper:**
    - si connette, invia `Hello`
    - per ogni richiesta esegue `validate` e poi `LocalExecutor`
    - `Cancel` interrompe la copia in corso
    - esce dopo 60 s senza richieste
  - Il runner, quando riceve `RetryElevated`, passa a `ElevatedExecutor` (creato una sola volta per sessione e condiviso) per il file corrente e, se `apply_to_all`, per il resto del job.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5:** Verifica manuale:
  - F7 in `C:\Program Files` → `ACCESS_DENIED` → "Riprova come amministratore" → compare il prompt UAC → la cartella viene creata
  - una seconda operazione elevata entro 60 s non mostra un nuovo prompt
  - rifiutando l'UAC si torna alla finestra di errore
- [ ] **Step 6: Commit** `feat(elevation): helper elevato con pipe autenticata`

### Task 18: Integrazione shell nella UI e apertura file

**Files:**
- Create: `src/commands/shell.ts`, `src-tauri/src/shell/open.rs`
- Modify: `src/components/groups/FileList.tsx`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 `show_context_menu`/`show_properties`/`start_drag`; Task 6 `Settings.externalEditor`
- Produces:
  - `#[tauri::command] open_path(path: String) -> Result<(), AppError>`, con `ShellExecuteExW` verbo `open`
  - `#[tauri::command] open_with(exe: String, path: String) -> Result<(), AppError>`, con `std::process::Command`, senza shell
  - i comandi `cm_ContextMenu`, `cm_Properties`, `cm_List`, `cm_Edit`

- [ ] **Step 1:** Comandi:
  - Il tasto destro su una riga sposta il cursore lì (o mantiene la marcatura se la riga è già marcata) e chiama `show_context_menu` con `markedOrCursor` e le coordinate × `devicePixelRatio`.
  - `cm_ContextMenu` (Shift+F10, tasto Menu) apre il menu alla posizione della riga col cursore.
  - `cm_Properties` → `show_properties`.
  - Il drag delle righe fuori dalla finestra chiama `start_drag`; le righe trascinate sono quelle marcate oppure quella sotto il mouse.
  - Fase 1: `cm_List` (F3) apre il file con il programma di default (`open_path`), `cm_Edit` (F4) con `externalEditor` (`open_with`). In fase 2 verranno sostituiti dal viewer e dall'editor interni.
  - Dopo un'azione del menu nativo (per esempio "Elimina" o "Rinomina" di Explorer) basta il watcher: nessuna logica in più.
- [ ] **Step 2:** Verifica manuale:
  - il menu nativo su 3 file marcati agisce su tutti e 3
  - "Apri con" mostra le applicazioni
  - trascinando 2 file marcati su Explorer vengono copiati
  - F3 su un PDF lo apre nel lettore di default; F4 su un `.txt` lo apre in Notepad
- [ ] **Step 3: Commit** `feat(ui): menu contestuale nativo, proprietà, drag in uscita, F3/F4 esterni`

### Task 19: Dimensione delle cartelle

**Files:**
- Create: `src-tauri/src/fs/size.rs`
- Modify: `src/commands/navigation.ts`, `src/ipc/api.ts`, `src-tauri/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct DirSize { bytes: u64, files: u64, dirs: u64 }`
  - `pub fn dir_size(path: &Path, cancel: &AtomicBool) -> Result<DirSize, AppError>`
  - comandi `calc_dir_size(request_id: u64, path: String)` (asincrono, evento `"dir-size"` `{ requestId, path, result: DirSize | null, error: AppError | null }`) e `cancel_dir_size(request_id: u64)`
  - i comandi `mc_MarkOrSize` e `cm_CountDirContent`

- [ ] **Step 1: Test:** su un albero di 3 file (1 + 2 + 3 byte) in 2 sottocartelle → `DirSize{bytes:6, files:3, dirs:2}`; con `cancel` già true → `Err(Cancelled)`; una sottocartella senza permessi viene saltata senza fallire (le cartelle inaccessibili si contano a parte, non è un errore fatale).
- [ ] **Step 2:** `cargo test` → FAIL, poi implementare (iterativo con `FindFirstFileExW`, senza ricorsione sullo stack, senza seguire i reparse point), poi `cargo test` → PASS.
- [ ] **Step 3:**
  - `mc_MarkOrSize` (Spazio): su una cartella la marca, calcola la dimensione e la mostra nella colonna Dim. (`dirSizes`); su un file la marca e basta. In entrambi i casi il cursore non si sposta (come in TC con Spazio).
  - `cm_CountDirContent` calcola tutte le cartelle della vista.
  - Cambiare cartella annulla i calcoli in corso.
- [ ] **Step 4: Commit** `feat(fs): calcolo dimensione cartelle`

### Task 20: Barra F-key e barra di stato

**Files:**
- Create: `src/lib/fkeys.ts`, `src/lib/fkeys.test.ts`
- Modify: `src/components/shell/FKeyBar.tsx`, `src/components/shell/StatusBar.tsx`

**Interfaces:**
- Produces:
  - `interface FKeySlot { key: string; commandId: string; labelKey: string }`
  - `fkeyLabels(mods: { shift: boolean; ctrl: boolean; alt: boolean }, available: Set<string>): FKeySlot[]`, che restituisce solo i comandi registrati

- [ ] **Step 1: Test:**
  - senza modificatori → F3 cm_List, F4 cm_Edit, F5 cm_Copy, F6 cm_RenMov, F7 cm_MkDir, F8 cm_Delete, Alt+F4 cm_Exit
  - con Shift → F5 cm_CopySamepanel, F6 cm_RenameOnly, F8 mc_DeletePermanent
  - un comando non disponibile (per esempio Alt+F5, pacchetto della fase 3) non compare
- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** UI:
  - `FKeyBar` ascolta `keydown`/`keyup` per Shift/Ctrl/Alt e aggiorna le etichette. Un click esegue il comando.
  - `StatusBar` mostra:
    - numero di marcati e dimensione totale (oppure "n file, m cartelle" se non c'è niente di marcato)
    - il job in corso con mini barra e percentuale (click → pannello Operazioni)
    - lo spazio libero dell'unità del gruppo attivo
    - il branch git resta nascosto fino alla fase 5
- [ ] **Step 4: Commit** `feat(ui): barra F-key dinamica e barra di stato`

### Task 21: Pagine Impostazioni e Scorciatoie

**Files:**
- Create: `src/components/settings/SettingsPage.tsx`, `KeybindingsPage.tsx`, `src/commands/keymap.conflicts.test.ts`

**Interfaces:**
- Consumes: Task 6 `useSettings`; Task 10 `getCommands`, `buildKeymap`, `chordFromEvent`
- Produces:
  - `findConflicts(map: Map<string, string>, chord: string, commandId: string): string | null`, aggiunta a `keymap.ts`
  - comandi `cm_Config` (Ctrl+,) e `mc_Keybindings`

- [ ] **Step 1: Test:** `findConflicts` con un chord già assegnato a un altro comando restituisce quel comando; con lo stesso comando o con un chord libero restituisce `null`.
- [ ] **Step 2:** `npm test` → FAIL, poi implementare, poi `npm test` → PASS.
- [ ] **Step 3:** UI: pagina a schermo intero sopra l'area dei gruppi (Esc chiude), con campo di ricerca e categorie, stile VS Code.
  - **Impostazioni:** tema (scuro / chiaro / segui Windows), lingua, mostra file nascosti, mostra file di sistema, conferma eliminazione, editor esterno. Le modifiche si applicano subito.
  - **Scorciatoie:** tabella con comando (titolo tradotto), tasti e origine (default/utente), con ricerca.
    - Doppio click → cattura del tasto → avviso se c'è un conflitto → salvataggio in `keybindings.json` come override.
    - "Ripristina" toglie l'override.
- [ ] **Step 4:** Verifica manuale: rimappando F5 su Ctrl+K, la copia parte con Ctrl+K e la mappatura sopravvive al riavvio; cambiando lingua in italiano, tutta la UI è tradotta senza riavviare.
- [ ] **Step 5: Commit** `feat(ui): impostazioni e scorciatoie`

### Task 22: Pacchetti e verifica finale della fase

**Files:**
- Create: `scripts/package-portable.ps1`
- Modify: `src-tauri/tauri.conf.json` (bundle `targets: ["nsis"]`, icona provvisoria), `README.md` (sezione build)

- [ ] **Step 1:** `npm run tauri build` → installer NSIS in `src-tauri/target/release/bundle/nsis/`.
- [ ] **Step 2:** `scripts/package-portable.ps1` crea `dist-portable/ModernCommander-<versione>-portable.zip` con l'exe e un file vuoto `portable`. La versione si legge da `tauri.conf.json`. Si assume WebView2 presente (Windows 11), e il README lo dice.
- [ ] **Step 3:** Verifica della versione portable:
  - si estrae in una cartella nuova e si avvia: la configurazione viene creata in `<cartella>\config\`
  - avvio a finestra pronta < 1 s, misurato con un cronometro su build release a cache calda
- [ ] **Step 4:** Verifica dell'installer: si installa, si avvia, la configurazione finisce in `%APPDATA%\ModernCommander`; la disinstallazione funziona.
- [ ] **Step 5:** `npm test` e `cargo test` → tutto PASS. Ripassare a mano i cinque punti della Review Focus su build release.
- [ ] **Step 6: Commit** `chore: pacchetti NSIS e portable`
