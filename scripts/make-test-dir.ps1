# Crea una cartella di prova per le misure di prestazioni della lista file:
# -Count file vuoti in -Path, più la sottocartella "10k" con 10.000 file.
# Da usare FUORI dal repository, per esempio: .\scripts\make-test-dir.ps1 -Path $env:TEMP\mc-test -Count 100000
param(
    [Parameter(Mandatory)] [string] $Path,
    [int] $Count = 100000
)
$ErrorActionPreference = 'Stop'

function New-EmptyFiles([string] $dir, [int] $n) {
    [System.IO.Directory]::CreateDirectory($dir) | Out-Null
    $exts = 'txt', 'md', 'json', 'png', 'ts', 'pdf', 'zip', 'log'
    for ($i = 0; $i -lt $n; $i++) {
        $name = 'file{0:D6}.{1}' -f $i, $exts[$i % $exts.Length]
        [System.IO.File]::Create((Join-Path $dir $name)).Dispose()
    }
}

New-EmptyFiles $Path $Count
New-EmptyFiles (Join-Path $Path '10k') 10000
Write-Host "Creati $Count file in $Path e 10000 in $(Join-Path $Path '10k')"
