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
- Archivi: solo l'elenco del contenuto (ZIP, TAR, TAR.GZ, 7z); l'estrazione arriva con la fase 3. 7z con intestazioni cifrate non supportato; RAR escluso. Nell'ispettore ZIP, TAR e TAR.GZ si elencano da soli, il 7z solo su richiesta (pulsante "Mostra contenuto"): il crate non limita la memoria e un 7z costruito ad arte può allocarne molta.

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
