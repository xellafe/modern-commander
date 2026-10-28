# Modern Commander

File manager per Windows in stile Total Commander, con un'interfaccia moderna
(ispirata a VS Code e Claude Code Desktop). Open source, licenza MIT.

Stack: Tauri 2, backend Rust, frontend React + TypeScript (Vite). Solo Windows 10/11 x64.

## Prerequisiti

- Node.js 22 e npm
- Rust stable (toolchain `x86_64-pc-windows-msvc`)
- Visual Studio Build Tools (carico di lavoro "Sviluppo di applicazioni desktop con C++")
- WebView2 (incluso in Windows 11; su Windows 10 va installato se manca)

## Sviluppo

```powershell
npm install
npm run tauri dev   # avvia l'app in modalita sviluppo
npm test            # test frontend (vitest)
cargo test --manifest-path src-tauri/Cargo.toml   # test backend
```

I test del backend sono due serie:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml                          # unitari
cargo test --manifest-path src-tauri/Cargo.toml --features test-hooks    # + integrazione dell'helper elevato
```

## Vedere e modificare i file (fase 2)

| Tasti | Funzione |
|---|---|
| F3 | Viewer interno in una scheda: testo (con codifica), esadecimale, immagini, PDF, audio e video; i file si leggono a blocchi, quindi anche quelli di più GB (da verificare) |
| F4 | Editor Monaco in una scheda, con salvataggio sicuro (file temporaneo e sostituzione, mai sul posto) |
| Ctrl+Q | Ispettore: anteprima, metadati, MD5/SHA-1/SHA-256 e elenco del contenuto degli archivi |
| Ctrl+Shift+P | Command palette |
| Ctrl+P | Salto rapido tra cartelle recenti, preferiti e file |
| Ctrl+ò / Ctrl+Shift+ò | Terminale a schede nel pannello in basso / nuovo terminale |
| Ctrl+Shift+F1 | Vista a miniature (anteprime della Shell) |

- **Ctrl+ò**: "ò" è il nome sul layout italiano del tasto `Oem3` (il suo carattere cambia con il layout: sul layout US è il backtick, a sinistra dell'1).
- **Viewer**: nel testo `A`, `7` e `6` scelgono ANSI, UTF-8 e UTF-16LE; `W` va a capo; `1` testo, `3` esadecimale. Con l'a capo spento ← → scorrono di una colonna, Ctrl+← e Ctrl+→ di una pagina, Home e Fine vanno a inizio riga e alla fine della riga più lunga visibile (Ctrl+Home e Ctrl+Fine a inizio e fine file). Esc annulla una scansione in corso, altrimenti chiude la scheda.
- **Editor esterno**: l'opzione `useExternalEditor` nelle impostazioni fa aprire a F4 `externalEditor` invece di Monaco.
- Monaco, pdf.js, xterm.js e markdown-it si caricano solo al primo uso, in chunk separati dal principale; l'effetto sul tempo di avvio è **da verificare** sulla build release (vedi il follow-up).

### Limiti accettati

- L'editor (Monaco) normalizza gli a capo misti a quello prevalente al salvataggio.
- Il viewer non conosce le codifiche DOS (437, 850).
- L'editor apre file fino a 32 MiB (`EDITOR_MAX_BYTES`): quelli più grandi si leggono con F3. Un file la cui decodifica perde dati (byte non validi) si apre in sola lettura.
- Un file solo ASCII di un utente con ANSI diverso da UTF-8 è rilevato come UTF-8.
- Il salvataggio scrive su `<file>.<8 cifre>.mcpart`; dopo un crash del processo può restare un residuo `.mcpart`.
- Immagini, PDF e media passano dall'asset protocol di Tauri: la verifica di Range 206, percorsi UNC, nomi con `#`, `%` e spazi, percorsi oltre 260 caratteri è **da verificare**; per i file che falliscono restano testo ed esadecimale. Vedi [docs/superpowers/fase-2-follow-up.md](docs/superpowers/fase-2-follow-up.md).
- Archivi: nell'ispettore solo l'elenco del contenuto (ZIP, TAR, TAR.GZ, 7z); l'apertura come cartella è nella sezione "Archivi (fase 3b)". Nell'ispettore ZIP, TAR e TAR.GZ si elencano da soli, il 7z solo su richiesta (pulsante "Mostra contenuto"): il crate non limita la memoria e un 7z costruito ad arte può allocarne molta.

## Sorgenti di rete (fase 3a)

| Tasti | Funzione |
|---|---|
| Ctrl+F | Gestore delle connessioni: nuova, modifica, duplica, elimina, connetti, importa da WinSCP |
| `\\server` nella barra del percorso | Elenco delle condivisioni SMB del server |

- **Protocolli**: SFTP (password o keyboard-interactive), FTP e FTPS (esplicito e implicito, modalità passiva), indici HTTP/HTTPS di Apache, nginx e IIS (Basic e Digest). Le schede remote si usano come quelle locali: F3, F4, F5, F6, F7, F8 (HTTP: solo lettura), ispettore, conflitti e coda.
- **Password**: solo nel Windows Credential Manager (`ModernCommander:<id connessione>`), mai nei file di configurazione. Le connessioni stanno in `connections.json` nella cartella di configurazione.
- **Impronte**: le chiavi SSH degli host e i certificati TLS non riconosciuti dall'archivio di Windows si accettano con un dialogo e si ricordano in `known_hosts.json` (stessa cartella). Una chiave cambiata blocca la connessione finché non si usa "Dimentica impronta".
- **Import da WinSCP**: siti dal registro o da `WinSCP.ini`, senza password; SCP diventa SFTP, WebDAV e S3 si saltano.
- Il viewer apre immagini, PDF e media remoti scaricandoli in una cartella temporanea, che si svuota alla chiusura e all'avvio.

### Limiti accettati

- Niente chiavi SSH né WebDAV (issue #1, #2), niente SCP. HTTP è in sola lettura.
- Dove la scrittura sicura (temporaneo e rinomina) non è possibile, il salvataggio diretto chiede conferma.
- Verifiche manuali e limiti noti: [docs/superpowers/fase-3a-follow-up.md](docs/superpowers/fase-3a-follow-up.md).

## Archivi (fase 3b)

| Tasti | Funzione |
|---|---|
| Invio / Ctrl+PgDn | Apre l'archivio sotto il cursore come cartella (ZIP, 7z, TAR, TAR.GZ), anche su un server |
| Backspace dalla radice | Torna alla cartella che contiene l'archivio |
| Alt+F5 | Crea uno ZIP dalla selezione nella cartella dell'altro gruppo (anche su un server) |
| Alt+F9 | Estrae l'archivio sotto il cursore, col dialogo della copia, in una sottocartella con il nome dell'archivio |
| F5, F6, F7, F8, Shift+F6, F4 | Dentro uno ZIP: aggiungere, spostare, creare cartelle, eliminare (F8 è sempre definitivo), rinominare, modificare un file (il salvataggio sostituisce la voce) |

- **Sola lettura**: 7z, TAR e TAR.GZ si sfogliano ed estraggono, ma non si modificano; F4 su di essi apre in sola lettura.
- **F3** su una voce la estrae prima nella cartella temporanea dell'app.
- **Scrittura**: ogni modifica a uno ZIP lo riscrive una sola volta per operazione in un file temporaneo e sostituisce l'originale solo a lavoro finito; errore o annullamento lasciano l'originale intatto.

### Limiti accettati

- Niente RAR, niente archivi annidati, niente archivi cifrati.
- Dizionario LZMA/LZMA2 (o memoria PPMd) oltre 256 MiB: archivio rifiutato (`ARCHIVE_TOO_DEMANDING`).
- Un TAR.GZ remoto si scarica per intero; ZIP e 7z remoti si leggono a intervalli.
- La modifica di un archivio non passa dall'helper elevato: dove serve l'elevazione fallisce.
- Uno ZIP locale aperto in un altro programma non si sostituisce (`ACCESS_DENIED`, originale intatto).
- Su FTP/SFTP senza rinomina atomica la sostituzione di uno ZIP elimina l'originale prima della rinomina finale.
- Verifiche manuali e limiti noti: [docs/superpowers/fase-3b-follow-up.md](docs/superpowers/fase-3b-follow-up.md).

## Build

Prerequisiti: Rust stable (`x86_64-pc-windows-msvc`), Visual Studio Build Tools con il carico di
lavoro C++, Node.js e npm. Requisito: Windows 10 o 11 x64 con WebView2. L'installer NSIS usa la
modalità di default di Tauri (`webviewInstallMode` = `downloadBootstrapper`, silenziosa): se WebView2
manca scarica e installa il bootstrapper di Microsoft, quindi serve la connessione. Il pacchetto
portable non lo installa: su un Windows 10 senza WebView2 va installato a parte.

```powershell
npm install
npm run tauri build            # installer NSIS in src-tauri\target\release\bundle\nsis\
scripts\package-portable.ps1   # dist-portable\ModernCommander-<versione>-portable.zip
```

- **Installer NSIS**: installazione per-macchina (`Program Files`), quindi richiede i privilegi di
  amministratore. È voluto: l'helper elevato (modello di sicurezza in `src-tauri/src/elevation`)
  presuppone che l'exe non sia scrivibile da un utente normale. La configurazione va in
  `%APPDATA%\ModernCommander`.
- **Portable**: lo zip contiene l'exe e un file vuoto `portable`; la configurazione va in
  `<cartella dell'exe>\config`. In una cartella scrivibile dall'utente l'elevazione non ha la
  garanzia dell'installazione per-macchina.
- La feature `test-hooks` serve solo ai test: non va mai abilitata nelle build distribuite
  (`npm run tauri build` non la abilita).

## Licenza

MIT, vedi [LICENSE](LICENSE).
