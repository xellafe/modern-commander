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

## Licenza

MIT, vedi [LICENSE](LICENSE).
