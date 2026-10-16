# Crea dist-portable\ModernCommander-<versione>-portable.zip: exe release + file vuoto `portable`.
# Prerequisito: `npm run tauri build`. Richiede WebView2 (presente in Windows 11).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = (Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
$exe = Join-Path $root 'src-tauri\target\release\modern-commander.exe'
if (-not (Test-Path $exe)) {
    throw "Exe di release non trovato: $exe. Esegui prima 'npm run tauri build'."
}

$out = Join-Path $root 'dist-portable'
$staging = Join-Path $out 'staging'
$zip = Join-Path $out "ModernCommander-$version-portable.zip"
Remove-Item $staging, $zip -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $staging | Out-Null
Copy-Item $exe (Join-Path $staging 'ModernCommander.exe')
New-Item -ItemType File (Join-Path $staging 'portable') | Out-Null
Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip
Remove-Item $staging -Recurse -Force
Write-Host "Creato: $zip"
