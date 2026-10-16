# Modern Commander

File manager per Windows in stile Total Commander, con un'interfaccia moderna
(ispirata a VS Code e Claude Code Desktop). Open source, licenza MIT.

Stack: Tauri 2, backend Rust, frontend React + TypeScript (Vite). Solo Windows 10/11 x64.

## Prerequisiti

- Node.js 22 e npm
- Rust stable (toolchain `x86_64-pc-windows-msvc`)
- Visual Studio Build Tools (carico di lavoro "Sviluppo di applicazioni desktop con C++")
- WebView2 (incluso in Windows 11)

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

## Build

Prerequisiti: Rust stable (`x86_64-pc-windows-msvc`), Visual Studio Build Tools con il carico di
lavoro C++, Node.js e npm. WebView2 è dato per presente (incluso in Windows 11): né l'installer né
il pacchetto portable lo installano.

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
