# Fase 2: verifiche manuali e follow-up

Data: 2026-10-19 · Branch: `claude/objective-napier-6e71d2` (lavoro sopra la punta della fase 1; piano: `docs/superpowers/plans/2026-10-17-fase-2-vedere.md`)

## Verifiche manuali da fare

Non si possono automatizzare: richiedono la GUI, file da GB o l'osservazione dello schermo. Nessuna è stata eseguita: i test automatici coprono solo la logica pura e il backend.

**Prima di tutto: asset protocol** (Task 5, Step 1). In DevTools, con l'app avviata:
- `fetch(convertFileSrc('C:\\Windows\\win.ini'))` risponde 200;
- lo stesso con header `Range: bytes=0-9` risponde 206 con 10 byte;
- provare un percorso UNC, un nome con `#`, `%` e spazi, un percorso oltre 260 caratteri.

Se un caso fallisce, per quei file restano le modalità testo ed esadecimale: annotarlo qui e nel README (sezione "Limiti accettati").

✅ Verificato il 2026-10-19 (`tauri dev`, URL `http://asset.localhost/` + `encodeURIComponent`): `win.ini` 200 (92 byte), con Range 206 (10 byte), UNC `\\localhost\C$` 200, nome con `#`, `%25` e spazi 200, percorso di 270 caratteri 200.

**Viewer testo ed esadecimale e file da GB** (Task 4)
- File di prova: `scripts\make-big-files.ps1 -Path D:\mc-test` crea `big.log` (5 GB), `one-line.json` (200 MB, una riga) e `zeros.bin` (5 GB). Per una prova veloce: `-LogBytes 5MB -JsonBytes 1MB -ZerosBytes 1MB`. Lo script non è rieseguibile due volte nella stessa sessione PowerShell (`Add-Type`).
- `big.log`: F3 si apre in meno di 300 ms; Fine mostra l'ultima riga; trascinando la barra a metà si vede una riga intera vicino a 2,5 GB; ↑ e ↓ in fondo e in cima non saltano righe.
- `one-line.json` con l'a capo attivo: Fine e ↓ su questo file e su uno di righe lunghe (una riga logica più alta del viewport resta tagliata in basso). Con l'a capo spento vedi "Viewer righe lunghe".
- `zeros.bin`: si apre in esadecimale; Ctrl+Fine mostra l'ultimo offset.
- Memoria del processo WebView2 (Gestione attività) sotto 300 MB in tutte le prove.
- Un file che cambia su disco mentre è aperto nel viewer (log in scrittura) mantiene la dimensione vista all'apertura e non si ricarica da solo: verificare che sia accettabile. Il costo per tasto di `layoutAt` non è stato misurato (solo il tasto tenuto premuto, vedi sopra).
- Un file UTF-16 con BOM e uno ANSI con lettere accentate si leggono bene; `A`, `7`, `6` cambiano la codifica. Passando da esadecimale a testo la prima riga può comparire a metà.
- Le lettere nel viewer non aprono la ricerca rapida; Tab passa all'altro gruppo; Ctrl+W ed Esc chiudono e tornano alla lista sul file di partenza.
- Tasto tenuto premuto (↓/↑) fluido (tasto → cursore < 16 ms), rotella da 3 righe, cambio scheda e ritorno alla stessa posizione, viewer in entrambi i gruppi, tema chiaro e scuro, selettori in barra.

**Viewer righe lunghe** (a capo spento, `LinesPane`; spec `specs/2026-10-19-viewer-righe-lunghe-design.md`). Sostituisce la prova a spezzoni da 4096 byte su `one-line.json`. Nessuna è stata eseguita.
- `one-line.json`: una riga sola; la barra verticale resta visibile finché la fine della riga non è nota, poi sparisce. Fine porta in fondo alla riga con l'indicatore "Scansione … % · Esc annulla" (solo dopo circa 200 ms); Esc ferma la scansione, la vista resta e la scheda resta aperta; un secondo Esc chiude la scheda. Dopo, → e ← sono fluidi e ← subito dopo Fine resta sotto 16 ms (punti d'avanzamento ogni 1 MiB); la barra orizzontale trascinata a metà mostra testo coerente; trascinata lontano e poi Esc compaiono i segnaposto, non il testo di prima.
- ← subito dopo Fine su una riga enorme in Shift_JIS/GBK: misurare (il conteggio multibyte costa di più).
- File con poche righe enormi: la barra verticale trascinata a metà porta all'inizio della riga che contiene quel punto. Con una riga da 200 MB tra righe normali, ↓ oltre la riga enorme mostra il segnaposto e poi la riga successiva. ↑ durante la scansione di `one-line.json`, poi ↓; ↓ tenuto premuto con `top` a metà di una riga enorme.
- Passando dall'a capo attivo allo spento a metà di `one-line.json` si torna all'inizio della riga, senza errori. Se Esc annulla quella normalizzazione compare un segnaposto "…" finché ↑, ↓, PgSu, PgGiù o la rotella non la rilanciano: ←, →, Home e Fine non la rilanciano.
- `big.log` invariato: apertura sotto 300 ms, ↓ tenuto premuto fluido (anche con `left > 0`, al massimo la riga in fondo mostra un attimo il segnaposto), Ctrl+Fine e barra come prima, WebView sotto 300 MB.
- Un TSV con tab e un testo accentato (UTF-8 e ANSI, `D:\mc-test\asset`) restano allineati dopo lo scorrimento; → arriva alla fine anche su un TSV corto in byte ma largo per i tab. `A`, `7` e `6` su un testo accentato scorso a destra ricalcolano le colonne.
- → ripetuti oltre la stima su un TSV con tab, con un ← durante il conteggio: vince il ←.
- File UTF-16 e UTF-8 con BOM: Ctrl+Home e ↑ non vanno sotto il BOM.
- Barra orizzontale: una riga lunga che entra dall'ultima riga non fa lampeggiare la barra; Maiusc+rotella e trackpad scorrono in orizzontale. Ctrl+←/→ scorrono di una pagina, Home e Fine vanno a inizio riga e alla fine della più lunga visibile.
- Con `left > 0` dopo uno spostamento verticale su righe più corte `left` può superare il limite: il primo ← o → lo riporta al limite con un salto (da confermare accettabile). Tra `left` diverse le righe già mostrate tengono il testo di prima finché arriva il nuovo (decisione di Task 5).

**Immagini, PDF e media** (Task 5)
- PNG, JPEG, SVG e WebP si vedono; un TIFF mostra l'avviso (e, con il Task 9, la miniatura della shell).
- Immagine adattata al riquadro e 1:1 al click (`max-height: 100%` nel flex).
- PDF di 300 pagine: prima pagina in meno di 1 s, scorrimento senza blocchi; ridimensionando il riquadro con un PDF aperto; aprendo e chiudendo subito un PDF grosso non restano richieste in DevTools → Network.
- MP4 da 2 GB: salto a metà immediato, la memoria non cresce della dimensione del file (richieste Range).
- MKV: avviso con "Apri con il programma predefinito".
- Tasti `1` e `3` dalle modalità native; tema chiaro e scuro.
- "Apri con il programma predefinito" nel `MediaPane` ignora gli errori di `openPath` (`.catch(() => {})`): con un file senza associazione o in uso non compare nulla. Provarlo e decidere se mostrare un avviso.

**Editor F4** (Task 6)
- F4 su un `.ts`: sintassi evidenziata; Ctrl+S salva e la data cambia nella lista (watcher). Verificare che Monaco e i suoi worker si carichino nella WebView (finora controllata solo la separazione dei chunk).
- File modificato da Notepad e poi salvato da qui: compare il conflitto; "Sovrascrivi" poi salva davvero.
- "ł" in un file ANSI e salvataggio: compare l'offerta UTF-8; con il file cambiato su disco, Sovrascrivi poi "Salva in UTF-8" salva davvero.
- Chiusura di una scheda modificata: compare la richiesta; Alt+F4 con un editor modificato chiede conferma.
- F4 su un `.exe` e su un file con byte non validi: sola lettura, con la barra che lo spiega.
- Con `useExternalEditor` attivo F4 apre Notepad.
- Due editor in due gruppi: Ctrl+S e Ctrl+W agiscono solo sull'istanza col focus; chiuso uno, Ctrl+S nell'altro funziona ancora.
- Ctrl+Shift+P e Ctrl+ò funzionano con il focus in Monaco.
- F4, digitare qualcosa, poi F5 e Ctrl+R (anche con il focus in Monaco e dopo un clic sulla barra delle schede): la pagina non si ricarica e l'editor resta modificato. Lo stesso con F5 e Ctrl+R su un viewer. Ricaricando comunque (per esempio da DevTools) con un editor modificato compare la richiesta di conferma.
- Chiudere l'ultima scheda dell'ultimo gruppo quando è un file (Esc, Ctrl+W, X sulla scheda, anche con "Non salvare"): al suo posto resta una scheda cartella sulla cartella del file.
- Ctrl+S dopo un clic sulla barra delle schede (focus sul body, non in Monaco) salva comunque; con il focus in un dialogo o in un campo di testo non cambia nulla.
- Ctrl+Tab e Ctrl+Shift+Tab con il focus in Monaco cambiano scheda.
- Salvare un file che un altro programma tiene aperto senza condivisione di cancellazione: il messaggio è "file in uso" (`ReplaceFileW` 1175), non un errore generico.

**Command palette e Ctrl+P** (Task 7, Task 8)
- Ctrl+Shift+P → "cop" → Invio esegue Copia; il focus torna alla lista prima del dialogo. Con un viewer o un editor attivo i comandi della lista non compaiono. Il click sul campo del titolo apre la palette. Resa visiva (posizione, `mark`, `kbd`, scorrimento della riga selezionata, hover contro tastiera).
- Ctrl+P su `C:\Windows` (decine di migliaia di voci): la palette resta reattiva mentre arrivano i lotti. Misurare `fuzzyFilter` su 20.000 voci (log `[perf] jump` in dev): se supera 16 ms, applicare il debounce di 50 ms all'input (non applicato, vedi "Scelte da confermare").
- Scegliendo un file la lista va nella sua cartella con il cursore sopra, anche da una scheda viewer o editor. Giunzioni e `node_modules` assenti dai risultati; con "mostra nascosti" compaiono i nascosti. Esc o chiusura annullano la scansione.
- Con Ctrl+P a palette aperta, e con il focus in un campo di testo o in Monaco, non deve aprirsi la finestra di stampa di WebView2 (`useKeyboard` inghiotte sempre Ctrl+P: da confermare nella GUI).
- Il pulsante nella barra del titolo mostra Ctrl+Shift+P; un clic fuori dalla palette la chiude; Esc con la palette aperta da Monaco riporta il focus a Monaco.
- Aspetto dell'indicatore di stato della scansione ("scansione in corso") nella palette.

**Miniature** (Task 9)
- Cartella con 2.000 JPEG: Ctrl+Shift+F1 mostra subito i riquadri; le miniature arrivano senza bloccare lo scorrimento; le frecce si muovono sulla griglia senza ritardo (< 16 ms).
- MP4 e PDF mostrano la miniatura della shell; F3 su un TIFF mostra l'anteprima della shell con la nota.
- A occhio: allineamento dei riquadri, nome su 2 righe con ellissi, rinomina in un riquadro, scala di Windows al 125% e 150%, bordi di PNG con alfa (possibili bordi scuri, vedi Follow-up).

**Ispettore Ctrl+Q** (Task 11)
- ↓ tenuto premuto su 1.000 file misti con l'ispettore aperto: il cursore non rallenta e alla fine l'anteprima è quella del file sotto il cursore.
- Una cartella enorme (`C:\Windows`): il calcolo parte e si annulla appena ci si sposta.
- Un `.md` con HTML dentro lo mostra come testo; clic destro su un link nel Markdown: nessun menu del browser.
- Un MP4 non parte da solo. Uno ZIP mostra l'elenco; uno ZIP rotto mostra l'errore tradotto. Un 7z mostra i metadati e il pulsante "Mostra contenuto": l'elenco compare solo dopo il clic e si annulla spostando il cursore.
- SHA-256 di un file da 4 GB: si annulla spostando il cursore.
- Aspetto del pannello (splitter sul bordo sinistro, temi chiaro e scuro, colori di `colorize`), Ctrl+Q e pulsante nella barra del titolo.
- Tempo di `colorize` su 256 KiB con righe corte (vedi Follow-up).

**Terminale** (Task 12, Task 13)
- Ctrl+ò apre il pannello con PowerShell 7 nella cartella del gruppo attivo; di nuovo Ctrl+ò lo nasconde e l'output resta al ritorno. Prima apertura con il pannello nascosto; clic sulla scheda "Terminale" senza sessioni (ne apre una).
- "+" ne apre un secondo; `exit` chiude la scheda; chiudendo l'ultima il pannello si nasconde.
- Tab completa i percorsi nella shell e non cambia gruppo; Ctrl+C con selezione copia, senza selezione interrompe; Ctrl+Shift+P apre la palette anche dal terminale.
- Caratteri accentati ed emoji; `vim` in WSL e adattamento al ridimensionamento del pannello e dello splitter; cambio tema con terminali aperti.
- Ctrl+V e Shift+Insert incollano in cmd, pwsh e WSL (l'handler non cancella più il tasto: l'evento `paste` arriva a xterm).
- Premendo Ctrl+ò due volte di seguito durante l'avvio del primo terminale non se ne apre un secondo. Un link OSC 8 `https://…` nell'output si apre nel browser; altri schemi non fanno nulla.
- Chiudendo la finestra mentre un Channel (terminale, scansione, dimensione) è ancora vivo l'app esce senza errori né processi orfani.
- `pwsh.exe`, `powershell.exe`, `wsl.exe --cd` avviati davvero (finora provato solo `cmd.exe`); il canale binario verso la WebView e l'evento `term-exit`.
- Chiudendo l'app non restano `pwsh.exe`, `conhost.exe` o `OpenConsole.exe` orfani (`RunEvent::Exit` → `kill_all`; verificare che l'uscita arrivi a `RunEvent::Exit`).
- Ricaricando la pagina in `tauri dev` le shell aperte si chiudono (`on_page_load(Started)` → `kill_all`).

**Build release** (Task 14, Step 3 e 5)
- Avvio della build release a finestra pronta in meno di 1 s (come nel Task 22 della fase 1): cronometro, a caldo e a freddo, senza debugger. Non misurato: serve `npm run tauri build` e la GUI.
- Il chunk principale (`index-*.js`) non contiene Monaco, pdf.js, xterm né markdown-it: controllato sulla build di `npm run build` (vedi sotto), non su quella release.
- I cinque punti della Review Focus sulla build release:
  1. file di più GB o con una riga enorme (log da 5 GB, JSON da 200 MB su una riga, zeri): F3 si apre subito, Fine e il trascinamento arrivano in fondo, memoria stabile;
  2. salvataggio che perderebbe dati: file cambiato da un altro programma, caratteri non rappresentabili, file binario con F4;
  3. cursore veloce con l'ispettore aperto: UI fluida, anteprima vecchia mai al posto della nuova, calcoli annullati;
  4. file in uso (log aperto in scrittura, file in esclusiva): viewer, ispettore e hash leggono senza bloccare chi scrive; il blocco esclusivo dà `FILE_IN_USE` tradotto;
  5. tasti nel posto sbagliato: lettere nel viewer non alla ricerca rapida; Tab, Ctrl+Tab, Ctrl+W su un viewer agiscono su gruppi e schede; Ctrl+ò e Ctrl+Shift+P anche con il focus nel terminale o in Monaco; Ctrl+C, Tab e frecce nel terminale restano alla shell.
- Chunk principale: circa 507 kB (495 KiB); il commento in `vite.config.ts` ora lo dice.

**Scorciatoie nuove**
- Ctrl+Q (vista rapida) e Ctrl+Shift+F1 (miniature) sono i tasti di TC per le stesse funzioni (confermato sul forum ufficiale ghisler.ch), usati come in TC.
- Ctrl+P in TC è "percorso nella riga di comando"; qui è il salto rapido perché l'app non ha una riga di comando (spec §5.7).
- Ctrl+Shift+P, Ctrl+ò (`Ctrl+Oem3`) e Ctrl+Shift+ò (`Ctrl+Shift+Oem3`) non risultano tra i tasti di TC secondo le fonti secondarie. L'help ufficiale non è raggiungibile (404 su ghisler.com/tcmdhelp): non verificato, come in fase 1.
- Ctrl+Shift+P, Ctrl+ò e Ctrl+Shift+ò su tastiera non italiana: `Oem3` cambia lettera col layout.

## Scelte da confermare

- **Ctrl+P** va al salto rapido come da spec §5.7, anche se in TC è "percorso nella riga di comando". Se si vuole altro: una riga in `tcKeymap.ts`; l'utente può rimappare.
- **`cm_Exchange`** non fa niente se una delle due schede attive è un file. **`mc_OpenInPrevGroup`/`mc_OpenInNextGroup`** aprono una nuova scheda cartella nel gruppo di destinazione se la sua scheda attiva è un file. Lo spec non lo dice; vale per 3 comandi.
- **Destinazione di F5/F6 e del drag** con una scheda file nell'altro gruppo: la cartella contenitore del file.
- **`cm_DirectoryHotlist`** resta `fileList` (un test della fase 1 lo richiede); il piano non lo elenca tra i comandi `group`.
- **Editor, byte non validi**: un file la cui decodifica perde dati (byte non validi per la codifica) si apre in sola lettura, non modificabile con U+FFFD. Estende il piano ("sola lettura" solo per i binari): evita di sovrascrivere in silenzio.
- **Editor, nome del file temporaneo**: `<file>.<8 cifre esadecimali>.mcpart` (`create_new`) invece di `<file>.mcpart` del piano, per non bloccare i salvataggi con il residuo di un crash e per i salvataggi concorrenti. Dopo un crash del processo può restare un `.mcpart` orfano; nomi di circa 255 caratteri falliscono con `INVALID_NAME`.
- **PDF**: `disableAutoFetch: true`, per non leggere mai il file intero. Costo: scorrere in avanti in un PDF molto grosso può essere più lento. `isEvalSupported` non si passa: non esiste in pdfjs-dist 6.4.299, che non usa più `eval`.
- **Salto rapido**: il debounce di 50 ms non è applicato; c'è solo il log dev `[perf] jump`. Dipende dalla misura GUI (vedi sopra).
- **Archivi TAR e TAR.GZ**: nessun tetto ai byte decompressi; parser manuale degli header da 512 byte (L e x entro 64 KiB, g e K ignorati, checksum validato), il crate `tar` solo in `dev-dependencies`. Motivo: l'ispettore elenca da solo al passaggio del cursore e un OOM con `panic = "abort"` chiude l'app. Costo: più codice da mantenere; header esotici danno `CORRUPT_ARCHIVE`.
- **Immagini nel Markdown** dell'ispettore disabilitate (`csp: null`). Il Markdown si rende con `html: false`.
- **`cm_SrcQuickview`** è `group`, non `global`: Ctrl+Q non agisce in campi e dialoghi.
- **`mc_MarkDown`/`mc_MarkUp`/`cm_Reverse`** nella vista miniature si muovono di una riga di riquadri (extra rispetto al piano): confermare l'intento.
- **Chunk principale oltre 500 kB**: `chunkSizeWarningLimit: 3000` in `vite.config.ts` nasconde l'avviso anche per il chunk principale (circa 507 kB); il commento è corretto. Decidere se serve un budget per il solo chunk principale.

## Follow-up noti

**Fuori perimetro (da proporre)**
- Ricerca nel viewer (F7 del Lister di TC) e Shift+F4 (nuovo file).
- Diff con HEAD, stato git e tag nell'ispettore: fase 5 (le sezioni restano nascoste).
- Persistenza delle cartelle recenti di Ctrl+P: arriva con i workspace (fase 5); oggi l'elenco vive per la sessione.
- TOCTOU sulle giunzioni per le primitive elevate (già nel follow-up della fase 1): tocca l'helper UAC, va in un piano a parte.

**Viewer**
- Con l'a capo attivo una riga logica più alta del viewport (pezzi da 4096 byte) resta tagliata in basso: lo scorrimento è per righe logiche. **Risolto con l'a capo spento** (righe intere, `LinesPane`); resta per l'a capo attivo.
- `lineUp` su righe senza LF con base > 0 può dare spezzoni diversi da quelli della discesa (come da piano). Cambiando modalità o codifica `top` si allinea solo all'unità di codice.
- Flash vuoto a ogni miss della finestra (`lines=[]` mentre il layout è `null`).
- Il focus non torna al viewer dopo un click su tab strip o sidebar (effetto `[active]`).
- Esc inattivo finché lo stato è `undefined` (caricamento o errore di `fileInfo`/`readChunk`); errori di lettura senza nuovo tentativo.
- Cambio modalità o seek vicino a EOF può lasciare `top` oltre l'ultima pagina; la maniglia non arriva al 100%.
- L'effetto di `TextPane` non ha `bigEndian` tra le dipendenze; la logica down/up/toEnd/seek non è estratta in `lib` né testata.
- `ScrollRail`: non impedisce la selezione del testo durante il trascinamento; `role=scrollbar` senza focus né `aria-controls`.
- `MediaPane` non rilascia `src` allo smontaggio; il resize ridisegna ogni pagina PDF a ogni tick (niente debounce); l'IntersectionObserver usa `entries[0]`; `root={box.current!}` letto in render; zoom immagine solo da click; salti di altezza con pagine PDF diverse dalla prima.
- PDF: `cMap`, font standard e wasm non serviti: CJK senza font incorporati e immagini JPX/JBIG2 possono non rendersi. Servirebbe copiare `cmaps/`, `standard_fonts/`, `wasm/` come asset. `@napi-rs/canvas` è opzionale nel lock.
- `preview.ts` `KINDS` è `{}`: un'estensione come `constructor` o `toString` dà un risultato sbagliato (stesso schema in `format.ts:18`); usare `Object.create(null)` o `Map`.
- `textPaging.ts:314` ramo morto; test mancanti per `lineUp` su confine di pezzo dopo LF con riga vuota sopra, e per `seekLine` con LF a `MAX_LINE` unità.
- `scripts\make-big-files.ps1`: non rieseguibile due volte nella stessa sessione (`Add-Type`); il commento dice "80 byte" per riga, sono 85.

**Editor**
- Monaco normalizza gli a capo misti a quello prevalente al salvataggio (limite accettato, nel README).
- Conflitto deciso solo da `modified_ms` (FAT32 ha 2 s di risoluzione): confrontare anche la dimensione; richiede di cambiare la firma `expected_modified_ms` del piano.
- `write_part` riporta il percorso del `.mcpart` nell'errore, e su 1176/1177 il `detail` forse non compare nella notice. `ReplaceFileW` su symlink non verificato.
- Un file ASCII puro di un utente con ANSI diverso da UTF-8 è rilevato come UTF-8 (nel README). DBCS con duplicati NEC e IBM non fanno round-trip.
- La coda di salvataggio dipende da `writeDoc` che non rigetta mai (`turn.catch` in `saveQueues`); nessun test TS sui waiter dopo un salvataggio fallito; il test Rust `parallel_writes_never_leave_a_half_written_file` è una rete di sicurezza non deterministica; commento `readOnly` "File binario" obsoleto (`editors.ts:26`).
- `chunkSizeWarningLimit` (vedi sopra); `wide`/`from_win` sono `pub(crate)` in `ops/exec.rs` (riuso voluto).

**Palette e Ctrl+P**
- `fuzzy`: un `toLowerCase` che cambia la lunghezza (`İ`) rende il confronto case-sensitive per quel testo.
- z-index della palette uguale al backdrop dei dialoghi: Alt+F4 con la palette aperta apre `ExitConfirm` sotto di essa. Hover contro frecce; ARIA combobox (`aria-expanded` sempre true, IME `isComposing`); nessun test del componente `Palette`.
- Il segnaposto del campo del titolo resta "Cerca file o comando…" (stringa della fase 1).
- Salto rapido: `cancel_request` può arrivare prima di `register` (pattern del registro, StrictMode in dev); nessun test su lotti vecchi, cancel a metà scansione, sottocartella illeggibile, drive; la selezione può spostarsi mentre arrivano i lotti; stato "scansione" e "nessun risultato" insieme; duplicati recente/preferito e `key` duplicate sui preferiti (id `kind:label:where`); primo lotto solo a 500 voci (rete lenta); cartelle placeholder OneDrive non entrate; il log perf misura solo `fuzzyFilter`. Nessun test TS è stato visto fallire prima dell'implementazione (Task 8).

**Miniature**
- `bgra_to_rgba` non de-premoltiplica: `GetImage` restituisce probabilmente ARGB premoltiplicato, quindi bordi semitrasparenti scuri su PNG con alfa. Il fix va fuori da `bgra_to_rgba`.
- `ShellPreview` non distingue l'icona generica dalla miniatura vera (niente `THUMBNAILONLY`); copia del buffer per l'header (fino a 4 MB); nessun timeout su `get_thumbnail` (uno slot resta occupato da un thumbnailer bloccato); COM inizializzato a ogni richiesta.
- `Tile`: effetto senza `catch` e flash del segnaposto a ogni rimontaggio; i fallimenti non sono in cache, quindi un file senza miniatura è richiesto di nuovo a ogni rimontaggio; `--mc-row-inset` non considerato nelle colonne.
- Test mancanti: `cancelOwner` di A non cancella la stessa chiave di B, refresh LRU.

**Ispettore**
- `colorize` è O(righe²) su 256 KiB con righe corte: misurare e valutare un limite (~2000 righe o 64 KiB). Colorato solo se la riga ≤ 2000 caratteri.
- L'errore di un hash (es. `FILE_IN_USE`) non ha messaggio: il tipo `hashes` del piano non ha uno stato d'errore. Il pulsante hash non si annulla con un clic.
- Il primo `computeDirSize` non è annullabile per un istante (fase 1); `setInspectorWidth` non ri-clampa al ridimensionamento della finestra e il clamp non è testato; `key={e.path}` duplicate nell'elenco archivio.
- La prima anteprima di testo aspetta il caricamento di Monaco (per `languageFor`); se l'import fallisce, testo semplice.
- `@types/markdown-it` non installato: markdown-it 15 ha i propri tipi.
- `TitleBar` chiama `toggleInspector` direttamente, non `executeCommand`.

**Archivi e hash**
- Elenco: `total` ZIP sottostimato con nomi duplicati; `total` e `truncated` mescolano conteggi grezzi e filtrati tra i formati; un nome ZIP da 32–64 KiB rende CORRUPT l'intero elenco; `cancel` ignorato con ZIP vuoti e, su ZIP, solo tra una voce e l'altra (il 7z si ferma anche durante la lettura delle intestazioni).
- **7z nell'ispettore (rischio residuo)**: `sevenz-rust2` decodifica l'intera intestazione codificata in memoria, fino alla `unpack_size` dichiarata, e crea decoder LZMA/LZMA2/PPMd senza limite di memoria (dizionari fino a 4 GiB da un file di poche centinaia di byte); con `panic = "abort"` un'allocazione fallita chiude l'app. Per questo il 7z non si elenca più da solo: serve il clic su "Mostra contenuto", e la lettura è annullabile (`Cancellable`). Il rischio resta per un clic esplicito su un 7z costruito ad arte: il crate non offre un limite di memoria. Mitigazioni possibili: leggere l'intestazione in un processo figlio con limite, o rifiutare i 7z con intestazione codificata oltre una soglia prima di passarli al crate.
- 7z con metodo non supportato (non AES) dà `CORRUPT_ARCHIVE` invece di `UNSUPPORTED`.
- TAR non gz troncato dentro i dati di una voce → `Ok` completo; `L` di size 0 o PAX `path=` vuoto → voce successiva senza nome; voci GNU `V` elencate; `GzDecoder` legge solo il primo membro (usare `MultiGzDecoder`); `GNU.sparse.name` PAX non letto.
- Non provato su archivi creati da 7-Zip o WinRAR né con nomi ZIP in CP437. Crate aggiunti: `zip` 9.0.5, `tar` 0.4.46, `flate2` 1.1.10, `sevenz-rust2` 0.24.1 (licenze permissive).
- Test mancanti: cancel a metà hash, `modified_ms` end-to-end, nomi con `..` o assoluti, ramo di produzione AES256_SHA256 senza la feature.

**Terminale**
- Una shell che esce subito sparisce senza messaggio: tenere la scheda se esce entro 1–2 s o con codice ≠ 0?
- Errori di spawn e di caricamento del chunk solo in `console.error`; il focus viene preso dall'effetto `[activeId, shown]`; `onBinary` non collegato; menu delle shell senza Esc né frecce; palette ANSI non adattata al tema chiaro (`minimumContrastRatio`); titolo della scheda fisso (cartella di partenza, non segue i `cd`); con la lista unità la cartella è vuota.
- Test mancanti su `handleExit` → drop → dispose e sul `catch` di `create`; scritture e resize usano invocazioni IPC separate, l'ordine non è garantito da Tauri.
- Backend: il flusso del registro in `term_spawn` non è testato (spostarlo in `Terminals::spawn`); `term-exit` (evento) e output (Channel) possono arrivare in ordine invertito; `wsl.exe` sempre elencata (stub in System32); cwd UNC con `cmd.exe` → `C:\Windows`; `term_write` sincrono sul thread principale; niente Job object (i processi nipoti restano); `kill_all` non lascia cadere i master; nessun commento sul `symlink_metadata` come alias di `Store`.
- Con più finestre un reload della pagina ucciderebbe anche i terminali delle altre (`kill_all` su `Started`): oggi la finestra è una.

**Build e configurazione**
- `chunkSizeWarningLimit: 3000` (vedi "Scelte da confermare"); chunk principale a 507,25 kB.
- `cargo test` ha 1 test ignorato, della fase 1 (`delete_to_recycle_bin_removes_from_disk`: manda un file vero nel Cestino, si lancia a mano con `-- --ignored`).

**Test**
- Task 1: `read_chunk`/`read_whole` su una cartella danno `ACCESS_DENIED` invece di `INVALID_REQUEST` (i chiamanti controllano `isDir`); nessun test per `len == MAX_CHUNK` né per `FILE_IN_USE` su `read_whole`; `read_chunk` non pre-dimensiona il `Vec`.
- Task 2: `FKeyBar`, `useKeyboard` e `isRunnable` duplicano l'espressione `dirTab`; le icone nella scheda non hanno `flex: none`.
- Nessun test dei componenti: Vitest gira in ambiente node, senza jsdom (Palette, Tile, ThumbList, ShellPreview, Editor, TerminalView sono coperti solo da `tsc` e build).

**Residui della revisione finale** (non corretti)
- Acceleratori della WebView che l'handler comune non raggiunge: `HotlistMenu`, `PathMenu` (`Group.tsx`) e la rinomina in linea (`FileList.tsx`) fanno `stopPropagation` su ogni tasto, quindi con il focus lì F5, Ctrl+R e Ctrl+P ricaricano o stampano. `beforeunload` protegge gli editor modificati, non terminali e layout. Correzione: `preventDefault` per `isBrowserAccelerator` in quei tre handler, o un listener in cattura su `window` che fa solo `preventDefault`.
- Non intercettati: Alt+← / BrowserBack e il tasto hardware BrowserRefresh (`VK168`).
- Doppio avvio del terminale: la corsa resta aperta durante il caricamento di xterm al primo uso (il contatore dei `pending` parte dopo `await`); serve un contatore sincrono all'inizio di `create`.
- Ctrl+S dell'editor (listener su `window` con il focus sul body): non controlla `.mc-page`, quindi con la pagina Impostazioni aperta salva l'editor dietro; dipende dall'ordine dei listener per il rispetto di `defaultPrevented`.
- Chunk principale ora a 509 kB (il commento in `vite.config.ts` dice circa 507 kB).
- Da provare a mano: Alt+F4 → conferma di uscita → Esci con un editor modificato non deve mostrare un secondo avviso di `beforeunload` dopo `destroy()`.

**Dalla fase 1, ancora aperti**
- Vedi `docs/superpowers/fase-1-follow-up.md`: CSP stretta (oggi `csp: null`, rilevante ora che l'asset protocol serve `**`), unità di rete mappate, hotlist malformata, ecc.
