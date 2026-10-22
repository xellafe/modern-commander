# Fase 3a: verifiche manuali e follow-up

Data: 2026-10-23 · Branch: `claude/fase-3a-rete-d97c31` (spec: `docs/superpowers/specs/2026-10-22-fase-3a-rete-design.md`)

## Verifiche manuali da fare

Nessuna è stata eseguita: richiedono la GUI o server veri. I test automatici coprono la logica pura, i parser (indici HTML, siti di WinSCP) e i provider con un server finto in memoria.

**Prima di tutto: CSP** (Task 14). La CSP vale solo per le pagine servite da Tauri: in `npm run tauri dev` la WebView carica `http://localhost:1420` direttamente da Vite e **nessuna CSP si applica** (né `csp` né `devCsp`; il proxy del dev server con CSP esiste solo su mobile). Quindi le prove vanno fatte su `npm run tauri build -- --debug` (DevTools disponibili) e poi sulla build release. In DevTools → Console, nessun errore `Refused to …` su:
- lista; anteprima testo colorata e Markdown dell'ispettore (Ctrl+Q);
- Monaco con i worker: F4 su un `.ts` (errori di tipo sottolineati: worker TypeScript) e su un `.json` (worker JSON); in Console nessun "Could not create web worker" né il ripiego sul thread principale;
- PDF con il worker di pdf.js (Network: richieste a `http://asset.localhost/…`);
- immagini (PNG, SVG, WebP), audio e video dall'asset protocol, anche un MP4 grosso con salto a metà;
- terminale xterm (stili, cursore, ridimensionamento);
- palette (Ctrl+Shift+P) e salto rapido (Ctrl+P);
- miniature (Ctrl+Shift+F1) e anteprima della shell di un TIFF;
- immagini, PDF e media di un file **remoto** (scaricati nella cartella temporanea, poi asset protocol).

La CSP arriva come header, non come `<meta>`: si legge in Network → documento → Response Headers → `Content-Security-Policy`.

**SFTP su un server dell'utente**
- Connessione: impronta sconosciuta → dialogo → Accetta; password ricordata ("Ricorda"), poi riconnessione senza richiesta.
- Log da GB: F3, Fine in fondo subito; Esc chiude.
- Download e upload di una cartella con conflitti (sovrascrivi, salta, più recente, "per tutti").
- F4 su un file di configurazione, Ctrl+S, riapertura: contenuto e permessi POSIX invariati.
- Salvataggio in una cartella non scrivibile (file scrivibile): compare il fallback confermato; Annulla non scrive nulla, Salva scrive.
- Server con e senza `posix-rename@openssh.com`: con l'estensione la sovrascrittura avviene senza domande, senza compare la richiesta `UNSAFE_WRITE`.
- VPN staccata durante un trasferimento: errore tradotto entro il timeout, nessun `.mcpart` rimasto quando si riprova; la scheda si riconnette alla prima operazione.
- Nomi con `\` o non UTF-8: si vedono nella lista, ma non si aprono (errore tradotto).
- Login keyboard-interactive (server con `PasswordAuthentication no` e `KbdInteractiveAuthentication yes`).

**FTP e FTPS** su un server di prova
- FTPS esplicito e implicito (990) con certificato autofirmato: dialogo con l'impronta, Accetta, poi niente più domande; certificato cambiato → di nuovo il dialogo.
- vsftpd con `require_ssl_reuse=YES` (riuso della sessione TLS sul canale dati).
- Server con MLSD e collegamenti simbolici; server con solo `LIST` (vsftpd: i file con il punto restano nascosti, vedi i limiti noti).
- Upload di una cartella con molti file su un server senza `MLST` (vsftpd): nel log del server due `LIST` della cartella di destinazione per file (prima erano tre), e nessun `LIST` in più per gli `stat` ravvicinati sulla stessa cartella.
- EPSV su IPv6.
- Disco pieno (552) durante un upload: errore `DISK_FULL` tradotto.
- Un job da molti file fa un solo login (pool di sessioni), non uno per file: controllare il log del server.
- F4 e Ctrl+S su un server senza `MDTM` (date dal `LIST`): la conferma compare sempre.

**Indice HTTP con autenticazione**
- Basic e Digest: navigazione, F3 su un log grosso con `Range` (Fine immediato).
- Server senza `Range`: F3 avvisa e "Scarica e apri" scarica e apre.
- F6, F7, F8, rinomina e F4 disattivati con l'avviso.
- Indice Apache con date in ora locale: lette come UTC, quindi spostate del fuso. IIS: solo date in inglese. IIS solo NTLM: `UNSUPPORTED`.

**SMB**
- `\\vm645srv`: elenco delle condivisioni (le `$` solo con "mostra nascosti"), ingresso in una condivisione.
- `\\wsl.localhost`: ricade sull'elenco normale.
- Ctrl+\ su una condivisione UNC resta sulla radice della condivisione.

**Interfaccia**
- Menu dell'app sulle schede remote: clic destro su una riga non marcata toglie le marcature; i comandi sono quelli dello spec.
- Trascinamento tra gruppi da una scheda remota (copia e spostamento).
- Invio su un file remoto apre il viewer.
- Avvio con due schede remote su connessioni diverse, senza password ricordate: due richieste; annullando la seconda, quella scheda resta in errore e Ctrl+R la ricollega.
- Gestore delle connessioni: "Dimentica impronta" e "Dimentica password". Dopo "Dimentica password" la connessione si chiude: la prossima operazione della scheda già aperta, un trasferimento o una scheda nuova chiedono di nuovo la password.
- Host non valido: con `/`, spazi, `\ ? # @ [ ]` nel campo host il salvataggio resta disattivato e il campo è segnato; `sftp://my host/x` scritto nel percorso dà l'avviso di indirizzo non valido.
- Indirizzo scritto con la password (`sftp://utente:password@host/x`): la connessione nuova ha solo `utente`, il campo password è vuoto e la password non finisce in `connections.json`.
- Importazione da WinSCP con i siti veri dell'utente: anteprima corretta (nome, protocollo, host, utente), SCP diventano SFTP, WebDAV e S3 saltati con il motivo, nessuna password importata; un sito con un host non valido (per esempio `intranet/logs`) saltato con "indirizzo del server non valido", gli altri importati.

**Viewer ed editor su file remoti**
- Immagine, PDF e media remoti: avanzamento del download, Esc annulla.
- "Scarica e apri" su un server HTTP senza `Range`.
- F4 e Ctrl+S con il dialogo `UNSAFE_WRITE` e con le date FTP poco affidabili.
- La cartella temporanea si svuota alla chiusura dell'app e all'avvio successivo (anche dopo una chiusura forzata).

**Regressioni locali** (i cinque punti della Review Focus della fase 2, sulla build release)
1. file di più GB o con una riga enorme (log da 5 GB, JSON da 200 MB su una riga, zeri): F3 si apre subito, Fine e il trascinamento arrivano in fondo, memoria stabile;
2. salvataggio che perderebbe dati: file cambiato da un altro programma, caratteri non rappresentabili, file binario con F4;
3. cursore veloce con l'ispettore aperto: UI fluida, anteprima vecchia mai al posto della nuova, calcoli annullati;
4. file in uso (log aperto in scrittura, file in esclusiva): viewer, ispettore e hash leggono senza bloccare chi scrive; il blocco esclusivo dà `FILE_IN_USE` tradotto;
5. tasti nel posto sbagliato: lettere nel viewer non alla ricerca rapida; Tab, Ctrl+Tab, Ctrl+W su un viewer agiscono su gruppi e schede; Ctrl+ò e Ctrl+Shift+P anche con il focus nel terminale o in Monaco; Ctrl+C, Tab e frecce nel terminale restano alla shell.

## Test ignorati

Non eseguiti (richiedono server veri o il Credential Manager). Le impronte finiscono in una cartella temporanea, mai nel `known_hosts` vero; le password non compaiono nell'output.

| Test | Variabile | Formato |
|---|---|---|
| `sftp_live` | `MC_TEST_SFTP` | `host:porta:utente:password:cartella_scrivibile` |
| `ftp_live` | `MC_TEST_FTP` e/o `MC_TEST_FTPS` | `host:porta:utente:password:cartella_scrivibile` (FTPS implicito se la porta è 990) |
| `http_live` | `MC_TEST_HTTP` | `url:utente:password` (vuoti per un indice pubblico) |
| `credential_manager_roundtrip` | nessuna | scrive, legge ed elimina una chiave di prova |

La password non può contenere `:`. Per lanciarne uno:

```powershell
$env:MC_TEST_SFTP = "host:22:utente:password:/tmp/mc"
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored sftp_live
```

`lists_shares_of_localhost` (`\\localhost`) è stato eseguito ed è verde. Restano anche gli ignorati della fase 1 (`delete_to_recycle_bin_removes_from_disk`).

## CSP

In `src-tauri/tauri.conf.json` (`app.security.csp`). Il JSON non ammette commenti e `SecurityConfig` rifiuta chiavi sconosciute: le motivazioni stanno qui. Verificate sul sorgente di Tauri 2.12.4 / tauri-codegen 2.7.3 e sulla build di `npm run build`.

| Direttiva | Valore | Perché |
|---|---|---|
| `default-src` | `'self'` | Tutto il resto (oggetti, frame, manifest) solo dall'origine dell'app (`http://tauri.localhost`). |
| `script-src` | `'self'` | Solo i chunk di Vite. Nessun `eval` né `new Function` nei bundle (l'unico `eval(` è una dichiarazione dentro `lib.d.ts` del worker TypeScript, testo). Gli script di inizializzazione di Tauri passano da WebView2 (`AddScriptToExecuteOnDocumentCreated`) e non sono soggetti alla CSP. Niente `'wasm-unsafe-eval'`: nessun WebAssembly sul thread principale (il wasm di pdf.js è nel worker e comunque non servito). |
| `style-src` | `'self' 'unsafe-inline'` | Monaco e xterm aggiungono `<style>` a runtime e attributi `style`. |
| `img-src` | `'self' asset: http://asset.localhost data: blob:` | Viewer immagini (asset protocol), sfondi `data:` nel CSS di Monaco. `blob:` è ammesso dal piano ma oggi nessuna vista lo usa (le miniature vanno su canvas). |
| `media-src` | `'self' asset: http://asset.localhost` | Audio e video del viewer. |
| `font-src` | `'self' data:` | `codicon.ttf` di Monaco (file del bundle). pdf.js carica i font incorporati nei PDF con `FontFace` da byte (fuori dalla CSP); `data:` copre il suo ripiego con una regola `@font-face`. |
| `connect-src` | `'self' ipc: http://ipc.localhost asset: http://asset.localhost` | IPC di Tauri (`fetch` verso `http://ipc.localhost` su Windows; senza, ripiega su `postMessage` con un avviso); pdf.js legge il PDF con `fetch` e `Range` dall'asset protocol. |
| `worker-src` | `'self' blob:` | Worker di Monaco (`?worker` di Vite: `new Worker('/assets/…')`) e di pdf.js (`workerSrc` sulla stessa origine). `blob:` copre i ripieghi delle due librerie (wrapper di pdf.js per un worker su un'altra origine, `_createWorker` di Monaco senza `getWorker`), che oggi non scattano. |

Note:
- Nessuna origine remota: anche le immagini nel Markdown dell'ispettore restano disattivate (`lib/markdown.ts`).
- Tauri aggiunge nonce e hash a `script-src`/`style-src` solo per `<script>` e `<style>` scritti in `index.html`. Oggi non ce ne sono. Se se ne aggiunge uno in linea, il nonce in `style-src` fa ignorare `'unsafe-inline'` al browser e Monaco e xterm perdono gli stili: in quel caso serve `dangerousDisableAssetCspModification: ["style-src"]`.
- I worker ricevono la CSP dalla propria risposta, e Tauri la manda solo per l'HTML: dentro i worker non ci sono restrizioni.
- `devCsp` non serve: in `tauri dev` su desktop non si applica (vedi sopra), e l'HMR di Vite (`ws://localhost:1420`) non ne risente.

## Limiti noti

- Un server SFTP che rifiuta un secondo canale non si collega (la navigazione e i trasferimenti usano canali separati).
- SFTP: nomi con `\` o non UTF-8 visibili ma non apribili.
- HTTP: date di Apache in ora locale lette come UTC; IIS solo date in inglese; IIS solo NTLM → `UNSUPPORTED`. Sola lettura.
- Niente chiavi SSH né WebDAV (issue #1, #2), niente SCP.
- FTP con solo `LIST` (vsftpd): i file e le cartelle che iniziano con il punto non si vedono. È voluto: `LIST` parte senza `-a` perché alcuni server leggono `-a` come un percorso.
- Un elenco remoto con più di 200.000 voci (o un elenco FTP oltre 64 MB, un indice HTTP oltre 16 MB) dà `TOO_LARGE`.
- Host delle connessioni: nomi, IPv4 e IPv6 senza parentesi quadre; niente host con `/`, spazi o `\ ? # @`.
