# Viewer F3: righe lunghe intere — piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** con l'a capo spento il viewer F3 mostra ogni riga logica come una riga sola, con scorrimento orizzontale allineato al carattere, senza mai caricare una riga intera.

**Architecture:** il backend Rust aggiunge due famiglie di scansioni a blocchi, annullabili e con avanzamento su `Channel`: la ricerca degli inizi riga (`next_line_start`, `line_start`) e il conteggio delle colonne (`column_offsets`, con `encoding_rs` per le code page a due byte). Il frontend aggiunge un pannello nuovo, `LinesPane`, usato al posto di `TextPane` quando l'a capo è spento. `TextPane` resta invariato per l'a capo attivo. La logica pura (righe nella finestra, punti di controllo, ritaglio con tab, coda di navigazione) sta in `src/lib/longLines.ts` ed è coperta da Vitest. Le cache per scheda (LF trovati, punti di controllo, larghezze) stanno in `src/state/lineCache.ts`.

**Tech Stack:** Tauri 2, Rust (`encoding_rs` già dipendenza diretta, `tempfile` per i test), React 19 + TypeScript, Zustand, i18next, Vitest. Nessuna dipendenza nuova.

**Spec:** `docs/superpowers/specs/2026-10-19-viewer-righe-lunghe-design.md`

## Global Constraints

- Valgono i vincoli globali del piano della fase 2 (`docs/superpowers/plans/2026-10-17-fase-2-vedere.md`, "Global Constraints"), in particolare i seguenti.
- **Commit solo con conferma esplicita dell'utente**: i passi "Commit" sono proposte da confermare. Niente push.
- Documentazione, commenti e messaggi di commit in italiano; identificatori in inglese.
- Nessuna stringa UI nel codice: chiavi i18n sempre in `en.json` e `it.json`.
- Errori dal backend come `AppError { code, path, detail }`. Un'annullata è `CANCELLED` e il frontend non la mostra come errore.
- Nessuna riga caricata intera in memoria, né nel frontend né nel backend. Una lettura resta al massimo `MAX_CHUNK` = 4 MiB, e le scansioni leggono blocchi da 1 MiB.
- `big.log` non peggiora: apertura sotto 300 ms, tasto → cursore sotto 16 ms, memoria della WebView sotto 300 MB.
- Colonna: 1 per carattere decodificato. Il tab porta al multiplo di 8 successivo, contato dall'inizio vero della riga. Il CR prima dell'LF non si mostra e non conta. Il conteggio coincide con l'output di `TextDecoder` / `encoding_rs` per la stessa codifica.
- L'indicatore compare dopo circa 200 ms di scansione: "Scansione {{percent}} % · Esc annulla".
- I test frontend coprono la logica pura (Vitest in ambiente node, niente jsdom). Il rendering si verifica a mano.

## Review Focus

1. **File con CRLF:** il CR non si vede, non conta come colonna e non compare a fine riga dopo Fine orizzontale. Test: `crlf_is_not_a_column` (Task 2), `wholeLines: CRLF end excludes the CR` (Task 3).
2. **Fine del file:** file vuoto, file che finisce senza LF, file che finisce con LF. La barra verticale si nasconde correttamente e Ctrl+Fine non entra in ciclo. Test: `wholeLines: end of file cases` (Task 3).
3. **UTF-16 con BOM:** `top` parte da 2, gli LF si cercano solo sui confini di unità e un `0A` in posizione dispari non è un LF. Test: `utf16_lf_only_on_unit_boundary` (Task 1), `wholeLines: utf-16le with BOM` (Task 3).
4. **Cambio di codifica con punti di controllo in cache:** offset e colonne di un'altra codifica non vanno riusati. Test: `lineCache resets marks and widths on encoding change` (Task 3).
5. **Colonna `left` dentro un tab all'inizio del ritaglio:** a sinistra compaiono gli spazi giusti e le colonne dopo restano allineate. Test: `renderSlice: left inside a tab pads with spaces` (Task 3).

---

### Task 1: Backend — ricerca degli inizi riga

**Files:**
- Create: `src-tauri/src/fs/lines.rs`
- Modify: `src-tauri/src/fs/mod.rs` (comandi), `src-tauri/src/lib.rs` (`generate_handler!`), `src/ipc/api.ts`, `src/ipc/api.test.ts`

**Interfaces:**
- Consumes: `content::open_shared`, `Requests::register`, il pattern `Channel` di `scan_tree` in `fs/mod.rs`.
- Produces (Rust, `fs::lines`):
  - `pub fn next_line_start(path: &Path, from: u64, unit: u8, big_endian: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<u64, AppError>`. Restituisce il byte dopo il primo LF in posizione `p ≥ from`, con `p` multiplo di `unit`. Senza LF restituisce la dimensione del file. `progress(p)` riceve l'offset fino a cui si è già cercato senza trovare LF.
  - `pub fn line_start(path: &Path, at: u64, unit: u8, big_endian: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<u64, AppError>`. Restituisce l'inizio della riga che contiene `at`, cioè `p + unit` per l'ultimo LF con `p + unit ≤ at`. Senza LF restituisce 0. `progress(p)` riceve l'offset più basso già esaminato.
  - LF: `0A` (unit 1), `0A 00` (UTF-16LE), `00 0A` (UTF-16BE), come `isLf` in `src/lib/textPaging.ts`.
- Produces (comandi): `next_line_start(request_id: u64, path: String, from: u64, unit: u8, big_endian: bool, on_progress: Channel<u64>) -> u64`, e `line_start` con gli stessi parametri e `at` al posto di `from`.
- Produces (TS, `src/ipc/api.ts`):
  - `nextLineStart(requestId: number, path: string, from: number, unit: 1 | 2, bigEndian: boolean, onProgress: (offset: number) => void): Promise<number>`
  - `lineStart(requestId: number, path: string, at: number, unit: 1 | 2, bigEndian: boolean, onProgress: (offset: number) => void): Promise<number>`

- [ ] **Step 1: Scrivere i test Rust che falliscono** in `lines.rs` (`#[cfg(test)]`, file con `tempfile`). Le funzioni pubbliche delegano a varianti private con il blocco come parametro (`next_line_start_in(..., block: usize)`), così i test usano blocchi di 4 byte:
  - `next_finds_lf_across_block_boundary`: `"abc\r\ndef"`, blocco 4, `from` 0 → 5.
  - `next_without_lf_returns_size`: `"abcdef"` → 6; `from` = dimensione → dimensione.
  - `line_start_backward_across_blocks`: `"ab\ncdefghij"`, `at` 10, blocco 4 → 3.
  - `line_start_at_a_line_start_returns_it`: `"ab\ncd"`, `at` 3 → 3; `at` 0 → 0.
  - `line_start_without_lf_is_zero`: `"abcdef"`, `at` 5 → 0.
  - `utf16_lf_only_on_unit_boundary`: in LE, `[0x41,0x0A, 0x00,0x42, 0x0A,0x00, 0x43,0x00]` con `from` 0 → 6 (lo `0A 00` alle posizioni 1–2 è a cavallo di due unità e non conta). In BE, `[0x00,0x41, 0x00,0x0A, 0x00,0x42]` → 4.
  - `progress_reports_scanned_offsets`: offset crescenti in avanti e decrescenti all'indietro.
  - `cancel_returns_cancelled`: flag già alzato → `ErrorCode::Cancelled`.
- [ ] **Step 2: Verificare che falliscano:** `cargo test --manifest-path src-tauri/Cargo.toml fs::lines` dà errore di compilazione (funzioni assenti).
- [ ] **Step 3: Implementare `lines.rs`.** Blocchi da 1 MiB (`const BLOCK: usize = 1 << 20`). Annullamento e `progress` si controllano a ogni blocco. Con unità 2, l'offset di ogni blocco resta pari. All'indietro, l'LF a cavallo tra due blocchi va trovato: il blocco precedente si legge con `unit` byte di sovrapposizione.
- [ ] **Step 4: Verificare che passino:** stesso comando, tutti verdi.
- [ ] **Step 5: Scrivere i comandi** in `fs/mod.rs`, sul modello di `scan_tree` (`requests.register`, `blocking`, invio su `Channel` con errore ignorato), e registrarli in `lib.rs`. Poi i wrapper TS in `api.ts`.
- [ ] **Step 6: Scrivere il test dei wrapper** in `api.test.ts`: `test("nextLineStart e lineStart passano i parametri al backend")`. Con `invoke` mockato verifica `invoke("next_line_start", { requestId: 1, path: "C:\\a", from: 4, unit: 2, bigEndian: false, onProgress: expect.anything() })` e l'analogo con `at` per `line_start`.
- [ ] **Step 7: Verificare:** `npm test -- src/ipc/api.test.ts` e `cargo test --manifest-path src-tauri/Cargo.toml` verdi.
- [ ] **Step 8: Commit** (da confermare): `feat(fs): ricerca degli inizi riga a blocchi, annullabile`.

### Task 2: Backend — conteggio delle colonne

**Files:**
- Create: `src-tauri/src/fs/columns.rs`
- Modify: `src-tauri/src/fs/mod.rs`, `src-tauri/src/lib.rs`, `src/ipc/types.ts`, `src/ipc/api.ts`, `src/ipc/api.test.ts`

**Interfaces:**
- Consumes: `content::open_shared`, `Requests`, `encoding_rs::Encoding::for_label`.
- Produces (Rust, `fs::columns`):
  - `#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub struct Mark { pub offset: u64, pub column: u64 }`. È un confine di carattere sulla riga, con la colonna dove inizia quel carattere.
  - `#[derive(Debug, Clone, Copy, PartialEq, Serialize)] #[serde(rename_all = "camelCase")] pub struct ColumnHit { pub offset: u64, pub column: u64, pub eol: bool }`.
  - `#[derive(Debug, Clone, Copy, Serialize)] #[serde(rename_all = "camelCase")] pub struct ColumnProgress { pub index: u32, pub mark: Mark }`.
  - `pub const TAB_WIDTH: u64 = 8;`
  - `pub fn column_offset(path: &Path, encoding: &str, from: Mark, target: u64, cancel: &AtomicBool, progress: &mut dyn FnMut(Mark)) -> Result<ColumnHit, AppError>`. Conta da `from`. Il primo carattere che occupa la colonna `target` dà `{ offset: inizio del carattere, column: sua colonna iniziale, eol: false }`: la colonna iniziale è minore di `target` solo dentro un tab. Se prima arriva un LF, un CR+LF o la fine del file, dà `{ offset: posizione del CR o dell'LF o dimensione, column: colonne della riga, eol: true }`. `target = u64::MAX` chiede la fine riga. Ogni 1 MiB letto chiama `progress` con un `Mark` sul confine di carattere corrente. Un'etichetta sconosciuta dà `INVALID_REQUEST`.
- Produces (comando): `column_offsets(request_id: u64, path: String, encoding: String, from: Vec<Mark>, target: u64, on_progress: Channel<ColumnProgress>) -> Vec<ColumnHit>`. Conta per ogni `from` in sequenza; `index` è la posizione in `from`.
- Produces (TS, `src/ipc/types.ts`): `interface Mark { offset: number; column: number }`, `interface ColumnHit { offset: number; column: number; eol: boolean }`, `interface ColumnProgress { index: number; mark: Mark }`.
- Produces (TS, `src/ipc/api.ts`): `export const LINE_END = Number.MAX_SAFE_INTEGER;` e `columnOffsets(requestId: number, path: string, encoding: string, from: Mark[], target: number, onProgress: (p: ColumnProgress) => void): Promise<ColumnHit[]>`.

- [ ] **Step 1: Scrivere i test Rust che falliscono** in `columns.rs`. Per brevità `col(bytes, enc, target)` = `column_offset` da `Mark { offset: 0, column: 0 }`.
  - `ascii`: `"abcdef"`, `"utf-8"`, 3 → `{3, 3, false}`.
  - `utf8_accents`: `"Perché è già"`, 8 → `{10, 8, false}`.
  - `tab_stops`: `"a\tb"`, 3 → `{1, 1, false}` (dentro il tab); 8 → `{2, 8, false}`; `u64::MAX` → `{3, 9, true}`.
  - `crlf_is_not_a_column`: `"ab\r\ncd"`, `u64::MAX` → `{2, 2, true}`. Un CR da solo (`"a\rb"`) vale 1.
  - `shorter_line_is_eol`: `"abc\ndef"`, 10 → `{3, 3, true}`.
  - `utf16_surrogate_pair_is_one`: `"a😀b"` in UTF-16LE, 2 → `{6, 2, false}`; in UTF-16BE lo stesso.
  - `shift_jis_double_byte`: `"日本a"` in Shift_JIS (4 + 1 byte), 1 → `{2, 1, false}`, 2 → `{4, 2, false}`.
  - `matches_encoding_rs_decode`: con casi oracolo per `utf-8`, `utf-16le`, `windows-1252`, `shift_jis`, il totale delle colonne (`u64::MAX`, senza tab né LF) è uguale a `encoding.decode_without_bom_handling(bytes).0.chars().count()`. I casi: UTF-8 non valido `[0x61, 0xF0, 0x9F, 0x61, 0x62]`, `[0xC3]` finale, `[0xED, 0xA0, 0x80]`; un surrogato UTF-16 spaiato; un byte Shift_JIS iniziale senza seguito.
  - `resume_from_mark_equals_full_count`: riga ASCII e accentata da 3 MiB. Prima `col(…, u64::MAX)` raccogliendo i `Mark` di `progress`, poi `column_offset` da un `Mark` intermedio verso un `target` più avanti: il risultato è uguale a quello del conteggio da zero.
  - `unknown_label_is_invalid_request` e `cancel_returns_cancelled`.
- [ ] **Step 2: Verificare che falliscano:** `cargo test --manifest-path src-tauri/Cargo.toml fs::columns`, errore di compilazione.
- [ ] **Step 3: Implementare `columns.rs`.** Legge a blocchi da 1 MiB e servono i confini dei caratteri, non il testo:
  - **UTF-8:** l'algoritmo di decodifica WHATWG (byte necessari e limiti inferiore e superiore), un carattere o U+FFFD per sottoparte massimale.
  - **UTF-16LE/BE:** a unità; la coppia surrogata vale 1, il surrogato spaiato 1.
  - **Code page a un byte** (`Encoding::is_single_byte()`): 1 byte = 1 colonna.
  - **Altre (`shift_jis`, `gbk`, `big5`, `euc-kr`):** un `encoding_rs::Decoder` alimentato un byte alla volta; un carattere finisce quando l'output cresce.
  - Il carattere a cavallo di due blocchi va gestito.
- [ ] **Step 4: Verificare che passino:** stesso comando, verdi.
- [ ] **Step 5: Scrivere comando, registrazione e wrapper TS** come in Task 1. Nel comando l'annullamento vale per tutta la richiesta. `progress` si inoltra come `ColumnProgress { index, mark }`.
- [ ] **Step 6: Scrivere il test del wrapper** in `api.test.ts`: `test("columnOffsets passa encoding, punti di partenza e target")`. Verifica `invoke("column_offsets", { requestId: 1, path: "C:\\a", encoding: "utf-8", from: [{ offset: 0, column: 0 }], target: LINE_END, onProgress: expect.anything() })`.
- [ ] **Step 7: Verificare:** `npm test -- src/ipc/api.test.ts` e `cargo test --manifest-path src-tauri/Cargo.toml` verdi.
- [ ] **Step 8: Commit** (da confermare): `feat(fs): conteggio delle colonne a blocchi con punti di controllo`.

### Task 3: Logica pura delle righe intere e cache per scheda

**Files:**
- Create: `src/lib/longLines.ts`, `src/lib/longLines.test.ts`, `src/state/lineCache.ts`, `src/state/lineCache.test.ts`
- Modify: `src/lib/textPaging.ts` (esportare `isLf` e spostarci `interface Win` da `TextPane.tsx`), `src/components/viewer/TextPane.tsx` (importare `Win`), `src/state/viewers.ts` (`dispose` chiama `disposeLineCache`)

**Interfaces:**
- Consumes: `Mark` da `src/ipc/types.ts` (Task 2).
- Produces (`src/lib/textPaging.ts`): `export interface Win { base: number; buf: Uint8Array; eof: boolean }`, `export function isLf(...)`.
- Produces (`src/lib/longLines.ts`):
  - `export interface Line { start: number; /** Fine del testo (CR o LF esclusi), null se oltre la finestra. */ end: number | null; /** Inizio della riga successiva, null se ignoto. */ next: number | null }`
  - `export function wholeLines(win: Win, top: number, max: number, unit: 1 | 2, bigEndian: boolean): Line[]`. Fino a `max` righe da `top`, senza `MAX_LINE`. Si ferma alla prima riga il cui LF non è nella finestra, che risulta con `end`/`next` null salvo `win.eof`: in quel caso `end = next = base + buf.length` per l'ultima riga. Con `top` fuori dalla finestra restituisce `[]`.
  - `export function prevLineStart(win: Win, top: number, unit: 1 | 2, bigEndian: boolean): number | null`. L'inizio della riga sopra `top`; null se non è determinabile dalla finestra (nessun LF e `win.base > 0`).
  - `export function isLineStart(win: Win, at: number, unit: 1 | 2, bigEndian: boolean): boolean | null`. Vero se `at` è 0 o segue un LF; null se il byte prima è fuori dalla finestra.
  - `export function nearestMark(marks: Mark[] | undefined, lineStart: number, target: number): Mark`. Il punto con `column ≤ target` più avanzato, oppure `{ offset: lineStart, column: 0 }`.
  - `export function mergeMarks(marks: Mark[] | undefined, add: Mark[]): Mark[]`. Ordinati per `offset`, senza duplicati.
  - `export function renderSlice(text: string, column: number, left: number, width: number): string`. `text` è decodificato da un carattere che inizia alla colonna assoluta `column ≤ left`. Espande i tab con colonne assolute, scarta le colonne `< left` (un tab tagliato lascia i suoi spazi residui) e restituisce al massimo `width` colonne. Un code point vale 1 (`for…of`).
  - `export type Nav = { dir: 1 | -1; n: number }` e `export function queueNav(pending: Nav | null, dir: 1 | -1, n: number): { pending: Nav; cancel: boolean }`. Nella stessa direzione somma; nella direzione opposta dà `cancel: true` e `pending = { dir, n }`.
- Produces (`src/state/lineCache.ts`):
  - `export interface LineCache { encoding: string; next: Map<number, number>; fwd: Map<number, number>; back: Map<number, number>; marks: Map<number, Mark[]>; widths: Map<number, number> }`.
  - `next` mappa inizio riga → inizio della successiva. `fwd` mappa inizio riga → offset fino a cui non c'è LF. `back` mappa inizio di una riga → offset più basso esaminato senza LF cercando la riga sopra. `marks` mappa inizio riga → punti di controllo. `widths` mappa inizio riga → colonne totali.
  - `export function lineCache(tabId: string, encoding: string): LineCache`. Crea la cache se manca. Se cambia `encoding` svuota `marks` e `widths` e tiene `next`, `fwd` e `back`, che non dipendono dalla codifica salvo l'unità: se cambia `codeUnit(encoding)` svuota tutto.
  - `export function disposeLineCache(tabId: string): void`.

- [ ] **Step 1: Scrivere i test che falliscono** in `longLines.test.ts`:
  - `wholeLines: lines longer than MAX_LINE stay whole`: una riga da 10.000 `x` più `"\nb\n"`, finestra completa (`eof`) → `[{start 0, end 10000, next 10001}, {start 10001, end 10002, next 10003}]`.
  - `wholeLines: a line whose LF is beyond the window has unknown end`: finestra `"ab\ncdef"` con `eof: false` → la seconda riga è `{3, null, null}` ed è l'ultima.
  - `wholeLines: CRLF end excludes the CR`: `"ab\r\ncd"` → `end` 2, `next` 4.
  - `wholeLines: end of file cases`: file vuoto → `[{0, 0, 0}]`. `"a\nb"` → due righe, la seconda `{2, 3, 3}`. `"a\n"` → `[{0, 1, 2}, {2, 2, 2}]`, una riga vuota finale come in un editor.
  - `wholeLines: utf-16le with BOM`: buffer `FF FE` + `"a\nb"` in UTF-16LE, `top` 2 → righe che iniziano a 2 e 6.
  - `prevLineStart and isLineStart inside and at the edge of the window`.
  - `nearestMark and mergeMarks`.
  - `renderSlice: tabs expand on absolute columns`: `renderSlice("a\tb", 0, 0, 20)` → `"a       b"`.
  - `renderSlice: left inside a tab pads with spaces`: `renderSlice("\tb", 1, 3, 20)` → `"     b"` (colonne da 3 a 7 del tab, poi `b` alla 8).
  - `renderSlice: width cuts by columns, a surrogate pair is one`.
  - `queueNav: same direction accumulates, opposite cancels`.

  E in `lineCache.test.ts`:
  - `lineCache resets marks and widths on encoding change`: da `windows-1252` a `utf-8` tiene `next`; verso `utf-16le` svuota tutto.
  - `disposeLineCache drops the tab`.
- [ ] **Step 2: Verificare che falliscano:** `npm test -- src/lib/longLines.test.ts src/state/lineCache.test.ts`, moduli assenti.
- [ ] **Step 3: Implementare** `longLines.ts` e `lineCache.ts`. La cache è una `Map` di modulo, non uno store reattivo: nessun rendering dipende dai suoi cambiamenti. Poi lo spostamento di `Win`/`isLf` e la chiamata in `useViewers.dispose`.
- [ ] **Step 4: Verificare:** `npm test` interamente verde, compresi i test esistenti di `TextPane` e `viewers`.
- [ ] **Step 5: Commit** (da confermare): `feat(viewer): logica delle righe intere e cache per scheda`.

### Task 4: `LinesPane` — navigazione verticale per righe intere

**Files:**
- Create: `src/components/viewer/LinesPane.tsx`
- Modify: `src/components/viewer/pane.ts`, `src/components/viewer/Viewer.tsx` (solo la scelta del pannello), `src/components/viewer/viewer.css`

**Interfaces:**
- Consumes: Task 1 (`nextLineStart`, `lineStart`), Task 3 (tutto), `readChunk`, `nextRequestId`, `cancelRequest`, `usePaneRows`, `ScrollRail`.
- Produces (`pane.ts`): `PaneHandle` ottiene tre membri opzionali, usati in Task 5 e Task 6:
  - `hscroll?(cols: number): void`
  - `hpage?(dir: 1 | -1): void` (di colonne visibili meno una)
  - `hedge?(end: boolean): void`
  - `cancel?(): boolean` (vero se c'era qualcosa da annullare)
- Produces: `export function LinesPane(props: { tabId: string; path: string; size: number; top: number; left: number; encoding: string; onBusy: (fraction: number | null) => void; onError: (e: unknown) => void; ref?: Ref<PaneHandle> })`.
- Produces (`Viewer.tsx`): in modalità testo usa `LinesPane` se `!state.wrap`, altrimenti `TextPane`. `left` arriva in Task 5: fino ad allora si passa `0`, e `onBusy` è un no-op fino al Task 6.

- [ ] **Step 1: Finestra e righe visibili.**
  - Finestra da 256 KiB come `TextPane` (`loadWindow`, numeri di sequenza).
  - Le righe visibili si compongono così: `wholeLines` dalla finestra, poi `lineCache(tabId, encoding).next` per proseguire oltre la finestra.
  - Una riga senza `next` noto avvia da sola `nextLineStart(start da fwd.get(s) ?? max(s, fine finestra))`. `onProgress` aggiorna `fwd`; il risultato aggiorna `next`.
  - Le righe dopo una riga non risolta sono segnaposto `<div className="mc-vline pending">…</div>`.
  - Il testo di ogni riga si legge dalla finestra se ci sta, altrimenti con `readChunk(start, BYTES)`, dove `const BYTES = 4096` basta per il viewport senza scorrimento orizzontale. Si taglia a `end`, si decodifica con `TextDecoder(encoding)` e si rende con `renderSlice(text, 0, 0, cols)`. `cols` è una stima larga finché il Task 5 non misura le colonne: per ora `BYTES`.
- [ ] **Step 2: Normalizzare `top` al montaggio.** Se `isLineStart` è falso (si arriva dall'a capo attivo a metà di una riga), `top` diventa `prevLineStart` oppure `lineStart(top)` dal backend, senza cambiare `left`. Con `null` (byte precedente fuori dalla finestra) si legge la finestra prima.
- [ ] **Step 3: ↓ / PgGiù.**
  - `scroll(n > 0)` passa per `queueNav`: con una scansione in corso nella stessa direzione si somma, nella direzione opposta si annulla.
  - Si avanza riga per riga usando `next` noto, oppure aspettando `nextLineStart`, fino a `n` righe o fino all'ultima pagina.
  - Ultima pagina: non si scende oltre il punto in cui l'ultima riga del file è l'ultima del viewport, come `maxScroll(…, wrap: false)`.
- [ ] **Step 4: ↑ / PgSu.** Stessa coda. Ogni passo usa `prevLineStart` dalla finestra; se è `null`, `lineStart(top - unit, …)` partendo da `back.get(top)` se noto. `onProgress` aggiorna `back`.
- [ ] **Step 5: Ctrl+Home / Ctrl+Fine (`edge`).**
  - `edge(false)`: `top = 0`.
  - `edge(true)`: dalla fine del file risale di `rows - 1` righe con la stessa logica di ↑, partendo da `lineStart(size)`. Il risultato è lo stesso di `endRows(rows, false)` in `TextPane`.
  - Entrambi annullano le scansioni pendenti di navigazione.
- [ ] **Step 6: Barra verticale.**
  - `ScrollRail` con `fraction = top / size`.
  - Si nasconde quando `top === 0`, la riga che finisce a `size` è tra le visibili e le righe sono al massimo `rows`.
  - `onSeek(f)`: `top = lineStart(floor(f * size) allineato all'unità)`, dalla finestra se possibile. `left` non cambia e le scansioni pendenti si annullano.
- [ ] **Step 7: Annullamento.**
  - `cancel()` chiama `cancelRequest` per ogni richiesta pendente e svuota la coda. Restituisce `true` se ce n'era almeno una.
  - Le risposte `CANCELLED` non arrivano a `onError`.
  - Allo smontaggio (cambio scheda, chiusura) si annulla tutto.
- [ ] **Step 8: Avanzamento.**
  - `onBusy(frazione)` mentre c'è almeno una scansione: byte fatti su byte da fare, sommati sulle richieste attive.
  - La distanza è `size - from` in avanti e `at` all'indietro.
  - A scansioni finite `onBusy(null)`.
- [ ] **Step 9: Verificare:** `npm test` verde e `npm run build` senza errori di tipo.
- [ ] **Step 10: Verifica manuale** (`npm run tauri dev`):
  - `one-line.json` con l'a capo spento mostra una riga sola e nessuna barra verticale.
  - `big.log`: ↓ tenuto premuto è fluido, e Ctrl+Fine e la barra vanno in fondo e a metà come prima.
  - Un file con una riga da 200 MB in mezzo a righe normali: ↓ oltre la riga enorme mostra il segnaposto e poi la riga dopo.
- [ ] **Step 11: Commit** (da confermare): `feat(viewer): righe intere con l'a capo spento, navigazione verticale`.

### Task 5: Scorrimento orizzontale allineato al carattere

**Files:**
- Modify: `src/state/viewers.ts` (+ `src/state/viewers.test.ts`), `src/components/viewer/LinesPane.tsx`, `src/components/viewer/pane.ts`, `src/components/viewer/ScrollRail.tsx`, `src/components/viewer/viewer.css`, `src/components/viewer/Viewer.tsx` (passa `left`)

**Interfaces:**
- Consumes: Task 2 (`columnOffsets`, `LINE_END`), Task 3 (`nearestMark`, `mergeMarks`, `renderSlice`, `lineCache.marks`/`widths`), Task 4 (`LinesPane`, `PaneHandle.hscroll`/`hedge`).
- Produces:
  - `ViewerState.left: number`; `initialViewerState` lo inizializza a 0.
  - `export function usePaneCols(ref: RefObject<HTMLElement | null>): number` in `pane.ts`: colonne intere che entrano, misurate con una sonda di 10 caratteri `0` nel font mono del pannello, aggiornate come `usePaneRows`.
  - `ScrollRail` ottiene `orientation?: "vertical" | "horizontal"`, `vertical` di default, con `aria-orientation` e classe `.mc-rail.h`.
  - `export const WHEEL_COLS = 6` in `pane.ts`.

- [ ] **Step 1: Aggiornare il test di `viewers`** (`initial mode and encoding`: l'oggetto atteso ha `left: 0`), verificare che fallisca, aggiungere `left` e verificare che passi: `npm test -- src/state/viewers.test.ts`.
- [ ] **Step 2: Offset della colonna `left`.**
  - Con `left > 0`, per le righe visibili con inizio noto si chiama una sola `columnOffsets(…, from = righe.map(r => nearestMark(marks.get(r.start), r.start, left)), target = left)`.
  - `onProgress` fa `mergeMarks` nella cache. I risultati non `eol` entrano anch'essi nella cache come punti di controllo.
  - Le richieste superate da una più recente si scartano con un numero di sequenza, come `seq` in `TextPane`, e si annullano.
  - Con `left === 0` l'offset è `start` e la colonna 0, senza backend.
- [ ] **Step 3: Rendering.**
  - Per ogni riga si leggono `(cols + 1) * 4` byte da `hit.offset`, dalla finestra se ci sta, troncati a `end` se noto.
  - Si decodifica e si mostra `renderSlice(text, hit.column, left, cols)`.
  - Una riga con `eol` (più corta di `left`) resta vuota.
  - Finché il risultato non arriva, la riga mostra il segnaposto.
- [ ] **Step 4: Tasti e mouse.**
  - `hscroll(cols)` sposta `left` di `cols`, limitato a `[0, max(0, estensione - colonne visibili + 1)]`. `hpage(dir)` equivale a `hscroll(dir * max(1, cols - 1))`.
  - `hedge(false)` porta `left = 0`.
  - `hedge(true)`: `columnOffsets(…, target = LINE_END)` per le righe visibili senza `widths` noto, salva i totali in `widths` e porta `left = max(0, max(widths) - cols + 1)`.
  - Rotella con Maiusc oppure `deltaX`: `hscroll(± WHEEL_COLS)`.
- [ ] **Step 5: Barra orizzontale.**
  - `ScrollRail orientation="horizontal"` sotto il testo, solo se l'estensione supera `cols`.
  - Estensione: il massimo, sulle righe visibili, di `widths.get(start)`. Se manca si stima: `(end ?? size) - start` byte, diviso 2 per UTF-16.
  - `fraction = left / max(1, estensione - cols)`; `onSeek` imposta `left` in proporzione.
- [ ] **Step 6: Verificare:** `npm test` verde e `npm run build` senza errori.
- [ ] **Step 7: Verifica manuale.**
  - `one-line.json`: → e ← fluidi; Fine porta alla fine della riga; la barra orizzontale trascinata a metà mostra testo coerente.
  - `D:\mc-test\asset` + un TSV con tab e un testo accentato UTF-8 e ANSI: dopo lo scorrimento le colonne restano allineate tra le righe.
- [ ] **Step 8: Commit** (da confermare): `feat(viewer): scorrimento orizzontale allineato al carattere`.

### Task 6: Integrazione nel Viewer, indicatore, tasti e documentazione

**Files:**
- Modify: `src/components/viewer/Viewer.tsx`, `src/i18n/it.json`, `src/i18n/en.json`, `docs/superpowers/fase-2-follow-up.md`, `README.md` (se ha la tabella dei tasti del viewer)

**Interfaces:**
- Consumes: `PaneHandle.cancel`/`hscroll`/`hedge` (Task 4, 5), `LinesPane.onBusy` (Task 4).
- Produces: chiavi i18n `viewer.scanning` (it: `"Scansione {{percent}} % · Esc annulla"`, en: `"Scanning {{percent}}% · Esc cancels"`) e `viewer.lineScrollbar` (it: `"Posizione nella riga"`, en: `"Position in line"`).

- [ ] **Step 1: Tasti in `onKeyDown`.**
  - ← e → chiamano `p.hscroll(∓1)`.
  - Ctrl+← e Ctrl+→ chiamano `p.hpage(∓1)`.
  - Home e Fine senza Ctrl chiamano `p.hedge` se esiste, altrimenti `p.edge` come oggi.
  - Ctrl+Home e Ctrl+Fine chiamano `p.edge`.
  - Esc chiama prima `p.cancel?.()` e chiude la scheda solo se restituisce falso.
- [ ] **Step 2: Indicatore.** `onBusy` imposta uno stato nel `Viewer`. Dopo 200 ms continuativi di stato non nullo, nella barra `mc-viewer-pos` mostra `t("viewer.scanning", { percent })` al posto della posizione. Il timer si azzera a `null`.
- [ ] **Step 3: Chiavi i18n** in `it.json` ed `en.json`. Il test esistente sulle chiavi deve passare (`npm test`).
- [ ] **Step 4: Verifica manuale completa** con `npm run tauri dev`:
  - Esc durante Fine orizzontale su `one-line.json`: la scansione si ferma, la vista resta, la scheda resta aperta. Un secondo Esc chiude la scheda.
  - L'indicatore compare solo sulle scansioni lunghe.
  - Passare dall'a capo attivo a quello spento a metà di `one-line.json`: si torna all'inizio della riga, senza errori.
  - Cambiare codifica con `A`, `7` e `6` su un testo accentato scorso a destra: le colonne si ricalcolano.
  - `big.log` invariato per tempi e memoria (Gestione attività sotto 300 MB).
- [ ] **Step 5: Documentazione.**
  - In `fase-2-follow-up.md` la riga sul punto `one-line.json` diventa la nuova verifica, con le voci della sezione "Verifica" della spec.
  - Segnare come risolti i follow-up superati, cioè gli spezzoni da 4096 byte con l'a capo spento.
  - Aggiornare la tabella dei tasti del viewer nel README se c'è.
- [ ] **Step 6: Verifica finale:** `npm test`, `npm run build` e `cargo test --manifest-path src-tauri/Cargo.toml` tutti verdi.
- [ ] **Step 7: Commit** (da confermare): `feat(viewer): indicatore delle scansioni, Esc annulla, tasti orizzontali`.
