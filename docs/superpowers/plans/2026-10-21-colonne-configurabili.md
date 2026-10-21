# Colonne configurabili ed estensione nel nome — piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** nella vista dettagliata l'utente sceglie, ordina e ridimensiona le colonne con un menu sull'intestazione, può mostrare l'estensione attaccata al nome e ha tre colonne nuove (Creato, Ultimo accesso, Tipo), con una configurazione unica salvata nelle impostazioni.

**Architecture:** il backend aggiunge le due date a `Entry` (stessa `WIN32_FIND_DATAW`) e un comando `type_names` che chiede a Windows la descrizione di un elenco di estensioni. Nel frontend la logica pura delle colonne (catalogo, normalizzazione, griglia, spostamento, ridimensionamento) sta in `src/lib/columns.ts`; la configurazione sta in `Settings`; i testi del Tipo stanno in una cache reattiva `src/state/typeNames.ts` richiesta da `tabViews.ts`. `FileList.tsx` disegna le celle dalla configurazione; il nuovo `ColumnHeader.tsx` gestisce intestazione, menu, ridimensionamento e spostamento.

**Tech Stack:** Tauri 2, Rust (`windows` 0.62 con `Win32_UI_Shell` già attiva), React 19 + TypeScript, Zustand, i18next, Vitest. Nessuna dipendenza nuova.

**Spec:** `docs/superpowers/specs/2026-10-21-colonne-configurabili-design.md`

## Global Constraints

- **Commit solo con conferma esplicita dell'utente**: i passi "Commit" sono proposte da confermare. Niente push.
- Documentazione, commenti e messaggi di commit in italiano; identificatori in inglese.
- Nessuna stringa UI nel codice: chiavi i18n sempre in `en.json` e `it.json` (il test `en and it have identical key sets` lo verifica).
- Input → cursore sotto 16 ms; lettura di 10.000 voci sotto 150 ms (log `[perf] list` in sviluppo). Le righe restano `memo` e virtualizzate.
- Scorciatoie di TC invariate: Ctrl+F3…F6, Ctrl+F1/F2, Shift+F6. Nessun nuovo `cm_*`.
- Trascinamenti con pointer events, non con il drag-and-drop HTML5 (su Windows funziona solo spegnendo il drop da Explorer di Tauri; vedi `TabStrip.tsx`).
- Valori fissati dalla spec: larghezze di default `ext` 52, `size` 72, `modified` 112, `created` 112, `accessed` 112, `attributes` 36, `type` 130 px; icona 16 px fissa; Nome `minmax(80px, 1fr)`; minimo 28 px per le altre colonne.
- I test frontend coprono la logica pura e gli store (Vitest in ambiente node, niente jsdom). Il rendering si verifica a mano con `npm run tauri dev`.

## Review Focus

1. **Impostazioni salvate da un'altra versione:** `columns` assente, con id sconosciuti, duplicati, Nome nascosto, larghezze non numeriche o sotto il minimo → la lista parte comunque con una configurazione valida. Test: `normalizeColumns` (Task 3).
2. **Nomi particolari:** `.gitignore`, `archive.tar.gz`, `README`, una cartella `v1.2` → chiave del Tipo `""`, `"gz"`, `""`, `"/"`, e il Nome con "estensione nel nome" non perde caratteri. Test: `typeKey` (Task 3).
3. **Ordinamento per Tipo con i testi non ancora arrivati e poi una rilettura del watcher:** si riordina una volta all'arrivo, senza richieste ripetute né cicli, e il cursore resta sul file. Test: `sort by type reorders once names arrive` e `reload does not refetch known type names` (Task 4).
4. **Pseudo-cartella delle unità con il Nome spostato in fondo o unica colonna:** etichetta e spazio libero restano leggibili. Test: `driveSpan` (Task 3).
5. **Trascinamento di una larghezza molto a sinistra o fuori finestra:** la colonna si ferma a 28 px, la configurazione si salva una volta al rilascio. Test: `resizeColumn clamps` (Task 3); salvataggio verificato a mano (Task 6).

---

### Task 1: Backend e tipi — date di creazione e di ultimo accesso

**Files:**
- Modify: `src-tauri/src/fs/listing.rs` (`Entry`, `entry_from`, test), `src/ipc/types.ts` (`Entry`), `src/state/tabView.ts:105` (riga `..`), `src/state/tabViews.ts:69` (`driveRow`)
- Modify (solo letterali `Entry` dei test, segnalati da `tsc`): `src/commands/dirSize.test.ts`, `src/commands/navigation.test.ts`, `src/commands/shell.test.ts`, `src/lib/status.test.ts`, `src/state/folderTree.test.ts`, `src/state/tabView.test.ts`, `src/state/tabViews.test.ts`

**Interfaces:**
- Produces: Rust `Entry { …, created_ms: i64, accessed_ms: i64 }` (dopo `modified_ms`, stessa conversione FILETIME → ms Unix); TS `Entry { …, createdMs: number; accessedMs: number }`, entrambi obbligatori. Le righe sintetiche (`..`, unità) li mettono a 0.

- [ ] **Step 1: Test Rust che fallisce** in `listing.rs`:

```rust
#[test]
fn lists_creation_and_access_times() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), b"x").unwrap();
    let e = &list_dir(d.path()).unwrap()[0];
    assert!(e.created_ms > 0 && e.accessed_ms > 0);
    assert!(e.created_ms <= e.modified_ms + 2_000); // stessa scala temporale della modifica
}
```

- [ ] **Step 2: Verificare che fallisca:** `cargo test --manifest-path src-tauri/Cargo.toml fs::listing` → errore di compilazione (campi assenti).
- [ ] **Step 3: Implementare** i due campi da `ftCreationTime` e `ftLastAccessTime`, estraendo la conversione FILETIME → ms in una funzione locale `fn filetime_ms(ft: FILETIME) -> i64` usata anche per `modified_ms`.
- [ ] **Step 4: Aggiungere i campi in TS** (`types.ts`, riga `..`, `driveRow`) e nei letterali dei test elencati, finché `npx tsc --noEmit` passa.
- [ ] **Step 5: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml` e `npm test` verdi, `npx tsc --noEmit` senza errori.
- [ ] **Step 6: Misurare** con `npm run tauri dev` il log `[perf] list` su una cartella da almeno 10.000 file, prima (su `main`) e dopo. Se `ipc` + `buildRows` supera 150 ms, fermarsi e riferire: la spec prevede in quel caso un parametro di `list_dir` (vedi spec, "Date di creazione e di ultimo accesso").
- [ ] **Step 7: Commit** (con conferma): `feat(listing): date di creazione e di ultimo accesso nelle voci`

---

### Task 2: Backend — comando `type_names`

**Files:**
- Create: `src-tauri/src/shell/types.rs`
- Modify: `src-tauri/src/shell/mod.rs` (`pub mod types;`), `src-tauri/src/lib.rs` (`generate_handler!`), `src/ipc/api.ts`, `src/ipc/api.test.ts`

**Interfaces:**
- Produces:
  - Rust `pub fn type_name(key: &str) -> Option<String>` e `#[tauri::command] pub async fn type_names(keys: Vec<String>) -> Result<Vec<String>, AppError>` (stessa lunghezza e ordine di `keys`; `""` dove Windows non risponde o la chiave non è valida).
  - Chiave: `"/"` = cartella; `""` = file senza estensione; altrimenti l'estensione senza punto, minuscola. Una chiave con `\ / : * ? " < > |` o NUL (tranne `"/"` esatta) non è valida.
  - TS `export const typeNames = (keys: string[]) => invoke<string[]>("type_names", { keys });`

- [ ] **Step 1: Test Rust che fallisce** in `types.rs`:

```rust
#[test]
fn folder_and_extensions_have_type_names() {
    let folder = type_name("/").unwrap();
    let txt = type_name("txt").unwrap();
    assert!(!folder.is_empty() && !txt.is_empty() && folder != txt); // testi localizzati: niente confronto letterale
    assert!(type_name("").is_some_and(|s| !s.is_empty()));
    assert_eq!(type_name("a\\b"), None);
    assert_eq!(type_name("x:y"), None);
}
```

E in `api.test.ts`: `typeNames passa le chiavi a type_names` (`invoke` chiamato con `"type_names", { keys: ["/", "txt"] }`).

- [ ] **Step 2: Verificare che falliscano:** `cargo test --manifest-path src-tauri/Cargo.toml shell::types` (errore di compilazione) e `npm test -- src/ipc/api.test.ts`.
- [ ] **Step 3: Implementare `type_name`** con `SHGetFileInfoW(nome, attr, &mut SHFILEINFOW, size, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES)`: per `"/"` nome `"folder"` con `FILE_ATTRIBUTE_DIRECTORY`, per `""` nome `"file"`, altrimenti `"file.<ext>"`, con `FILE_ATTRIBUTE_NORMAL`. Testo da `szTypeName` fino al NUL; `None` se la chiamata restituisce 0 o il testo è vuoto. Non tocca il disco.
- [ ] **Step 4: Implementare il comando** con `crate::fs::blocking`, inizializzando COM sul thread come `thumbnail()` (`CoInitializeEx` STA, `CoUninitialize` solo se `is_ok()`), una volta per tutto l'elenco. Registrarlo in `lib.rs` e aggiungere `typeNames` in `api.ts`.
- [ ] **Step 5: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml` e `npm test -- src/ipc/api.test.ts` verdi.
- [ ] **Step 6: Commit** (con conferma): `feat(shell): descrizione di Windows del tipo di file per estensione`

---

### Task 3: Catalogo delle colonne e impostazioni

**Files:**
- Create: `src/lib/columns.ts`, `src/lib/columns.test.ts`
- Modify: `src/state/settings.ts`, `src/state/settings.test.ts`, `src/state/tabView.ts` (solo il tipo `SortKey`)

**Interfaces:**
- Consumes: `splitName` da `src/lib/format.ts`.
- Produces (in `src/lib/columns.ts`):
  - `export type ColumnId = "name" | "ext" | "size" | "modified" | "created" | "accessed" | "attributes" | "type";`
  - `export interface ColumnConfig { id: ColumnId; visible: boolean; width: number | null }` (`width` è `null` solo per `name`).
  - `export const COLUMN_DEFS: Record<ColumnId, { width: number | null; visible: boolean; align: "left" | "right"; sortKey: SortKey | null }>`; ordine delle chiavi = ordine di default (tabella della spec). `sortKey`: `name`→`"name"`, `ext`→`"ext"`, `size`→`"size"`, `modified`→`"date"`, `created`→`"created"`, `accessed`→`"accessed"`, `type`→`"type"`, `attributes`→`null`.
  - `export const MIN_COL_WIDTH = 28;` `export const DEFAULT_COLUMNS: ColumnConfig[]`
  - `export function normalizeColumns(raw: unknown): ColumnConfig[]`
  - `export function visibleColumns(cols: ColumnConfig[], extInName: boolean): ColumnConfig[]` (toglie le nascoste e, con `extInName`, `ext`)
  - `export function gridTemplate(visible: ColumnConfig[]): string`
  - `export function toggleColumn(cols: ColumnConfig[], id: ColumnId): ColumnConfig[]` (`name` invariato)
  - `export function moveColumn(cols: ColumnConfig[], id: ColumnId, beforeId: ColumnId | null): ColumnConfig[]` (`null` = in fondo; lavora sull'elenco completo, nascoste comprese)
  - `export function resizeColumn(cols: ColumnConfig[], id: ColumnId, width: number): ColumnConfig[]` (arrotonda, minimo `MIN_COL_WIDTH`, `name` invariato)
  - `export function typeKey(name: string, isDir: boolean): string` (`"/"` per le cartelle, altrimenti l'estensione di `splitName` in minuscolo)
  - `export function driveSpan(ids: ColumnId[]): string | null` (`grid-column` della cella unità, con l'icona in colonna 1)
- Produces (in `Settings`): `columns: ColumnConfig[]` (default `DEFAULT_COLUMNS`), `extInName: boolean` (default `false`); `SortKey` diventa `"name" | "ext" | "size" | "date" | "created" | "accessed" | "type" | "none"`.

- [ ] **Step 1: Test che falliscono** in `columns.test.ts`:

```ts
test("gridTemplate dei default è la griglia di oggi col minimo del Nome", () => {
  expect(gridTemplate(visibleColumns(DEFAULT_COLUMNS, false))).toBe("16px minmax(80px, 1fr) 52px 72px 112px 36px");
  expect(gridTemplate(visibleColumns(DEFAULT_COLUMNS, true))).toBe("16px minmax(80px, 1fr) 72px 112px 36px");
});
test("normalizeColumns", () => {
  expect(normalizeColumns(undefined)).toEqual(DEFAULT_COLUMNS);
  const n = normalizeColumns([
    { id: "type", visible: true, width: 200 }, { id: "bogus", visible: true, width: 10 },
    { id: "type", visible: false, width: 50 }, { id: "name", visible: false, width: 300 },
    { id: "size", visible: true, width: "x" }, { id: "ext", visible: true, width: 3 },
  ]);
  expect(n.slice(0, 4)).toEqual([
    { id: "type", visible: true, width: 200 }, { id: "name", visible: true, width: null },
    { id: "size", visible: true, width: 72 }, { id: "ext", visible: true, width: 28 },
  ]);
  expect(n.map((c) => c.id).sort()).toEqual(DEFAULT_COLUMNS.map((c) => c.id).sort()); // mancanti in fondo coi default
});
test("moveColumn e toggleColumn", () => {
  const ids = (c: ColumnConfig[]) => c.map((x) => x.id);
  expect(ids(moveColumn(DEFAULT_COLUMNS, "size", "name")).slice(0, 2)).toEqual(["size", "name"]);
  expect(ids(moveColumn(DEFAULT_COLUMNS, "name", null)).at(-1)).toBe("name");
  expect(toggleColumn(DEFAULT_COLUMNS, "name")).toEqual(DEFAULT_COLUMNS);
  expect(toggleColumn(DEFAULT_COLUMNS, "type").find((c) => c.id === "type")!.visible).toBe(true);
});
test("resizeColumn clamps", () => {
  expect(resizeColumn(DEFAULT_COLUMNS, "size", -500).find((c) => c.id === "size")!.width).toBe(28);
  expect(resizeColumn(DEFAULT_COLUMNS, "size", 90.6).find((c) => c.id === "size")!.width).toBe(91);
  expect(resizeColumn(DEFAULT_COLUMNS, "name", 300)).toEqual(DEFAULT_COLUMNS);
});
test("typeKey", () => {
  expect(typeKey(".gitignore", false)).toBe("");
  expect(typeKey("archive.tar.GZ", false)).toBe("gz");
  expect(typeKey("README", false)).toBe("");
  expect(typeKey("v1.2", true)).toBe("/");
});
test("driveSpan", () => {
  expect(driveSpan(["name", "ext", "size"])).toBe("3 / -1");
  expect(driveSpan(["size", "ext", "name"])).toBe("2 / 4");
  expect(driveSpan(["ext", "name", "size"])).toBe("4 / -1");
  expect(driveSpan(["name"])).toBeNull();
});
```

In `settings.test.ts`: il test dei default verifica anche `columns: DEFAULT_COLUMNS, extInName: false`; nuovo test `load normalizza columns` in `settings.store.test.ts` (`readConfig` restituisce `{ columns: [{ id: "name", visible: false, width: 1 }] }` → `useSettings.getState().columns` ha `name` visibile con `width: null` e tutte le colonne del catalogo).

- [ ] **Step 2: Verificare che falliscano:** `npm test -- src/lib/columns.test.ts src/state/settings.test.ts src/state/settings.store.test.ts`.
- [ ] **Step 3: Implementare `columns.ts`.** Regola di `driveSpan`: se ci sono colonne dopo il Nome, da quella subito dopo fino a `-1`; altrimenti, se ce ne sono prima, da 2 fino alla colonna del Nome esclusa; altrimenti `null`. Regola di `normalizeColumns`: input non array → `DEFAULT_COLUMNS`; si tiene la prima occorrenza di ogni id noto; `visible` non booleano → default; `width` non numerica finita → default, poi minimo `MIN_COL_WIDTH`; `name` sempre `visible: true, width: null`; le mancanti in fondo nell'ordine del catalogo.
- [ ] **Step 4: Estendere `Settings`** (`columns`, `extInName`, `defaultSettings`, `pickSettings`) e in `load` applicare `normalizeColumns` a `merged.columns`. Estendere `SortKey`.
- [ ] **Step 5: Verificare:** `npm test` verde e `npx tsc --noEmit` senza errori.
- [ ] **Step 6: Commit** (con conferma): `feat(columns): catalogo delle colonne e configurazione nelle impostazioni`

---

### Task 4: Ordinamento per le colonne nuove e cache del Tipo

**Files:**
- Create: `src/state/typeNames.ts`, `src/state/typeNames.test.ts`
- Modify: `src/state/tabView.ts` (`BuildOpts`, `buildRows`), `src/state/tabView.test.ts`, `src/state/tabViews.ts`, `src/state/tabViews.test.ts`, `src/main.tsx` (sottoscrizione a `useSettings`)

**Interfaces:**
- Consumes: `typeNames` (Task 2); `typeKey`, `visibleColumns` (Task 3); `Settings.columns`, `Settings.extInName`.
- Produces:
  - `export const useTypeNames = create<{ names: Record<string, string> }>(…)` — testi per chiave di `typeKey`; `""` = chiesto ma senza risposta.
  - `export function ensureTypeNames(keys: Iterable<string>): Promise<boolean>` — chiede in **una** chiamata le chiavi non in cache e non già in volo; `true` se ha aggiunto testi nuovi. Un errore di `typeNames` registra `""` per quelle chiavi.
  - `BuildOpts.typeNames?: Record<string, string>`; `buildRows` ordina per `created`/`accessed` numericamente e per `type` col `collator` su `typeNames[typeKey(…)] ?? ""`; a parità il nome, cartelle prima, come oggi.
  - In `tabViews.ts`: `export function requestTypes(tabId: string): Promise<void>` — se la scheda ordina per `type` o la colonna `type` è in `visibleColumns(settings)`, chiama `ensureTypeNames` sulle chiavi del listing in memoria; se ha aggiunto testi e la scheda ordina per `type`, `rebuild(tabId, {})`. Chiamata alla fine di `load` e di `rebuild` (senza attenderla). `rowsFor` passa `typeNames: useTypeNames.getState().names`.

- [ ] **Step 1: Test che falliscono.**
  - `tabView.test.ts`: `sorts by created, accessed and type` — tre file con `createdMs`/`accessedMs` diversi ordinati per `created` e `accessed` (crescente e decrescente); per `type` con `typeNames: { txt: "Documento di testo", png: "Immagine PNG" }`, le cartelle restano prima, e un'estensione senza testo va in testa (stringa vuota).
  - `typeNames.test.ts` (con `vi.mock("../ipc/api")`): `fetches only missing keys in one call` (seconda chiamata con chiavi già note → nessuna `invoke`, risultato `false`); `concurrent calls do not duplicate keys`; `failure caches empty names` (`typeNames` rifiuta → `names` ha `""` per quelle chiavi, una nuova `ensureTypeNames` non richiama).
  - `tabViews.test.ts`: `sort by type reorders once names arrive` (sort `type`, `typeNames` risolve dopo `load` → le righe cambiano ordine, il cursore resta sullo stesso nome); `reload does not refetch known type names` (dopo un `reload`, `typeNames` chiamato una volta sola in tutto); `type names are not requested when the column is hidden and sort is not type`.
- [ ] **Step 2: Verificare che falliscano:** `npm test -- src/state/tabView.test.ts src/state/typeNames.test.ts src/state/tabViews.test.ts`.
- [ ] **Step 3: Implementare** `typeNames.ts`, l'estensione di `buildRows`, `requestTypes` e le due chiamate in `tabViews.ts`.
- [ ] **Step 4: In `main.tsx`,** nella sottoscrizione esistente a `useSettings`: se cambiano `columns` o `extInName`, `requestTypes` per ogni scheda in `useTabViews.getState().views` (accendere la colonna Tipo con schede già caricate).
- [ ] **Step 5: Verificare:** `npm test` verde e `npx tsc --noEmit` senza errori.
- [ ] **Step 6: Commit** (con conferma): `feat(list): ordinamento per date nuove e per tipo, con cache dei tipi`

---

### Task 5: Righe e intestazione disegnate dalla configurazione

**Files:**
- Modify: `src/components/groups/FileList.tsx` (`FullList`, `FullRow`), `src/components/groups/groups.css`, `src/i18n/en.json`, `src/i18n/it.json`

**Interfaces:**
- Consumes: `useSettings` (`columns`, `extInName`); `visibleColumns`, `gridTemplate`, `driveSpan`, `typeKey`, `COLUMN_DEFS` (Task 3); `useTypeNames` (Task 4); `formatDate`.
- Produces: classi di cella `c-<id>` (`c-name`, `c-ext`, `c-size`, `c-modified`, `c-created`, `c-accessed`, `c-attributes`, `c-type`) più `c-right` per l'allineamento a destra; chiavi i18n `groups.columns.{name,ext,size,modified,created,accessed,attributes,type}` (etichette brevi; `date` diventa `modified`) usate anche dal Task 6.

- [ ] **Step 1: `FullList`** calcola una volta `visible = visibleColumns(columns, extInName)`, `template = gridTemplate(visible)` e `ids = visible.map((c) => c.id).join(",")`; applica `template` come `style.gridTemplateColumns` a intestazione e righe (via `.mc-cols` senza `grid-template-columns` fisso in CSS). A `FullRow` passa `template`, `ids` (stringa, così `memo` resta efficace), `extInName` e il testo del Tipo della riga (`names[typeKey(row.name, row.isDir)]`, letto da `useTypeNames` in `FullList`).
- [ ] **Step 2: L'intestazione** (provvisoria in `FullList`, spostata nel Task 6) mostra un `span` per colonna visibile con l'etichetta breve; click → `sortBy(tabId, COLUMN_DEFS[id].sortKey)` se non `null`; freccia del verso quando `view.sort.key` coincide.
- [ ] **Step 3: `FullRow`** disegna una cella per id nell'ordine di `ids`: `name` (con `extInName` e un'estensione, `base` + `<span className="c-ext-inline">.{ext}</span>`; la rinomina in place resta `RenameInput`), `ext`, `size` (com'è oggi, con avviso e animazione), `modified`/`created`/`accessed` (`formatDate`), `attributes`, `type`. Riga `..`: solo icona e nome. Riga unità: icona, cella nome con `gridColumn` esplicito, una cella `c-drive` con `gridColumn: driveSpan(idsArray)` se non `null`; entrambe con `gridRow: 1`.
- [ ] **Step 4: CSS:** colore attenuato e 11 px per tutte le celle tranne il Nome; `.c-right { text-align: right }` (e `justify-content: flex-end` nell'intestazione); `.c-ext-inline` attenuato, nel colore di risalto con `.mc-row.mk`; `.mc-list-full .mc-list-scroll { overflow-x: hidden; }`; togliere `grid-column: 3 / -1` da `.c-drive`.
- [ ] **Step 5: i18n:** etichette brevi in `en.json`/`it.json` (testi scelti dal builder; italiano coerente con quelli di oggi: "Nome", "Est.", "Dim.", "Modificato", "Creato", "Ultimo accesso", "Attr.", "Tipo"; in inglese "Created", "Accessed", "Type").
- [ ] **Step 6: Verificare:** `npm test` verde, `npm run build` senza errori. A mano (`npm run tauri dev`): con i default la lista è identica a oggi; mettendo a mano in `settings` `extInName: true` e la colonna `type` visibile, il nome mostra l'estensione attenuata e il Tipo compare poco dopo le righe; unità e `..` corrette; tasto ↓ tenuto premuto su una cartella grande fluido come prima.
- [ ] **Step 7: Commit** (con conferma): `feat(list): colonne disegnate dalla configurazione ed estensione nel nome`

---

### Task 6: Intestazione interattiva — menu, larghezza, ordine

**Files:**
- Create: `src/components/groups/ColumnHeader.tsx`
- Modify: `src/components/groups/FileList.tsx` (usa `ColumnHeader` al posto dell'intestazione provvisoria), `src/components/groups/groups.css`, `src/i18n/en.json`, `src/i18n/it.json`

**Interfaces:**
- Consumes: `toggleColumn`, `moveColumn`, `resizeColumn`, `DEFAULT_COLUMNS`, `COLUMN_DEFS` (Task 3); `useSettings().update`; le classi e le chiavi i18n del Task 5.
- Produces: `export function ColumnHeader(props: { tabId: string; sort: TabView["sort"]; visible: ColumnConfig[]; template: string; onLiveTemplate: (template: string | null) => void }): JSX.Element`. `onLiveTemplate` passa a `FullList` la griglia provvisoria durante il ridimensionamento (`null` al rilascio), così righe e intestazione si allargano insieme senza scrivere le impostazioni.

- [ ] **Step 1: Menu.** `onContextMenu` sull'intestazione (con `preventDefault` e `stopPropagation`, così non parte il menu della shell) apre un `.mc-menu` posizionato al puntatore, titolo `groups.columnsMenu.title`: una voce per colonna del catalogo (etichetta estesa, spunta se visibile; `name` disattivata con "sempre visibile"; `ext` disattivata e senza spunta con `extInName`), separatore, "Estensione nel nome" con spunta, separatore, "Ripristina colonne predefinite" (`update({ columns: DEFAULT_COLUMNS, extInName: false })`). Il click su una voce aggiorna le impostazioni e lascia il menu aperto; Esc o un `mousedown` fuori lo chiudono. Ruoli ARIA come in `Group.tsx` (`role="menu"`, `menuitemcheckbox` con `aria-checked`).
- [ ] **Step 2: Larghezza.** Maniglia `.mc-col-grip` sul bordo destro di ogni intestazione tranne il Nome; `pointerdown` con `setPointerCapture`, `pointermove` → `onLiveTemplate(gridTemplate(...resizeColumn...))`, `pointerup`/`pointercancel` → una sola `update({ columns: resizeColumn(...) })` e `onLiveTemplate(null)`. Il `pointerdown` sulla maniglia non avvia il riordino né l'ordinamento.
- [ ] **Step 3: Ordine.** Come `TabStrip.tsx`: `pointerdown` su un'intestazione, soglia di 5 px prima di considerarlo un trascinamento; sotto soglia il rilascio è un click (ordina). Durante il trascinamento la cella sotto il puntatore riceve `drop-before`/`drop-after` (metà sinistra/destra); al rilascio `update({ columns: moveColumn(columns, id, beforeId) })`, dove `beforeId` è la colonna visibile davanti a cui cade (`null` oltre l'ultima). L'icona non è trascinabile né bersaglio.
- [ ] **Step 4: i18n:** `groups.columnsMenu.{title, alwaysVisible, extInName, reset}` e le etichette estese `groups.columnsMenu.columns.{name,ext,size,modified,created,accessed,attributes,type}` ("Data di modifica", "Data di creazione", "Data di ultimo accesso"…).
- [ ] **Step 5: Verificare:** `npm test` verde, `npm run build` senza errori. A mano: menu (spunte, Nome bloccato, Estensione disattivata con estensione nel nome, menu che resta aperto, Esc), ridimensionamento fino al minimo e rilascio fuori finestra, spostamento del Nome in fondo, click sull'intestazione che ordina ancora e trascinamento che non ordina, click destro sulle righe che apre ancora il menu della shell, riavvio con configurazione conservata, seconda scheda e secondo gruppo con le stesse colonne.
- [ ] **Step 6: Commit** (con conferma): `feat(list): menu delle colonne, ridimensionamento e riordino dall'intestazione`

---

### Task 7: Verifica finale

**Files:** nessuno, salvo correzioni.

- [ ] **Step 1:** `npm test`, `cargo test --manifest-path src-tauri/Cargo.toml`, `npm run build` verdi.
- [ ] **Step 2:** checklist manuale della spec ("Verifica"), compresi Tipo su una cartella con molte estensioni, `drives:` con il Nome spostato, Shift+F6 con estensione nel nome, Ctrl+F3…F6 che ordinano come prima anche con le colonne nascoste.
- [ ] **Step 3:** confronto `[perf] list` e tasto tenuto premuto su 10.000 voci con tutte le colonne accese rispetto a `main`; riportare i numeri.
