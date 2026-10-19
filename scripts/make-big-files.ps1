<#
.SYNOPSIS
  Crea i file grandi per la verifica manuale del viewer F3 (fase 2): big.log, one-line.json, zeros.bin.
.EXAMPLE
  .\scripts\make-big-files.ps1 -Path D:\mc-test
  .\scripts\make-big-files.ps1 -Path $env:TEMP\mc-test -LogBytes 5MB -JsonBytes 1MB -ZerosBytes 1MB   # prova veloce
#>
param(
  [Parameter(Mandatory)][string]$Path,
  # big.log: righe numerate, 5 GB
  [long]$LogBytes = 5GB,
  # one-line.json: una sola riga senza a capo, 200 MB
  [long]$JsonBytes = 200MB,
  # zeros.bin: solo byte 0, 5 GB
  [long]$ZerosBytes = 5GB
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;

public static class McBigFiles {
  // Righe numerate di 80 byte ("000000000001 ..."): il numero di riga dice dove si è nel file. Scritte a blocchi da 8 MiB.
  public static void WriteLog(string path, long bytes) {
    using (var fs = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 20)) {
      var block = new StringBuilder();
      long written = 0, n = 1;
      while (written < bytes) {
        block.Clear();
        for (int i = 0; i < 100000 && written + block.Length < bytes; i++, n++) {
          string line = n.ToString("D12") + " lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod\n";
          if (written + block.Length + line.Length > bytes) line = line.Substring(0, (int)(bytes - written - block.Length) - 1) + "\n";
          block.Append(line);
          if (written + block.Length >= bytes) break;
        }
        var data = Encoding.ASCII.GetBytes(block.ToString());
        fs.Write(data, 0, data.Length);
        written += data.Length;
      }
    }
  }

  // Un JSON minificato su una sola riga: nessun a capo in tutto il file.
  public static void WriteOneLine(string path, long bytes) {
    using (var fs = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 20)) {
      var head = Encoding.ASCII.GetBytes("{\"items\":[");
      var item = Encoding.ASCII.GetBytes("{\"id\":0,\"name\":\"voce\",\"tags\":[\"a\",\"b\"]},");
      var tail = Encoding.ASCII.GetBytes("null]}");
      long written = 0;
      if (bytes <= head.Length + tail.Length) { var small = new byte[bytes]; for (int i = 0; i < small.Length; i++) small[i] = (byte)'x'; fs.Write(small, 0, small.Length); return; }
      fs.Write(head, 0, head.Length); written += head.Length;
      while (written + item.Length + tail.Length <= bytes) { fs.Write(item, 0, item.Length); written += item.Length; }
      // Riempie fino alla dimensione richiesta con spazi dentro l'array, poi chiude.
      var pad = new byte[bytes - written - tail.Length];
      for (int i = 0; i < pad.Length; i++) pad[i] = (byte)' ';
      fs.Write(pad, 0, pad.Length);
      fs.Write(tail, 0, tail.Length);
    }
  }
}
'@

New-Item -ItemType Directory -Force -Path $Path | Out-Null
$Path = (Resolve-Path $Path).Path

Write-Host "big.log ($LogBytes byte)..."
[McBigFiles]::WriteLog((Join-Path $Path 'big.log'), $LogBytes)

Write-Host "one-line.json ($JsonBytes byte)..."
[McBigFiles]::WriteOneLine((Join-Path $Path 'one-line.json'), $JsonBytes)

Write-Host "zeros.bin ($ZerosBytes byte)..."
$zeros = Join-Path $Path 'zeros.bin'
if (Test-Path $zeros) { Remove-Item $zeros -Force }
fsutil file createnew $zeros $ZerosBytes | Out-Null
if ($LASTEXITCODE -ne 0) { throw "fsutil non è riuscito a creare $zeros (servono i permessi di scrittura su $Path)" }

'big.log', 'one-line.json', 'zeros.bin' | ForEach-Object { Get-Item (Join-Path $Path $_) } | Select-Object Name, Length
