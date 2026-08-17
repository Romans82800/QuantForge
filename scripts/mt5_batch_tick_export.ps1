# Batch-export IC Markets tick CSVs via MT5 Strategy Tester + SQ_TickDataExportEA.
# Matches the proven workflow (see Tester profiles for EURJPY 2016.01.01).
# Usage:
#   powershell -File scripts/mt5_batch_tick_export.ps1
# Optional:
#   -Symbols EURUSD,GBPUSD
#   -FromDate 2016.01.01
#   -ToDate 2026.08.17

param(
  [string[]]$Symbols = @(
    "AUDUSD", "CHFJPY", "EURAUD", "EURCAD", "EURGBP", "EURUSD",
    "GBPJPY", "GBPNZD", "GBPUSD", "NZDUSD", "US500", "USDCHF",
    "USDJPY", "XAUUSD", "BTCUSD", "USTEC", "XTIUSD", "EURJPY", "EURNZD"
  ),
  [string]$DefaultFromDate = "2016.01.01",
  [string]$ToDate = (Get-Date -Format "yyyy.MM.dd"),
  [string]$OutDir = "$env:USERPROFILE\Documents\QuantForge\tick_import_2016",
  [string]$TerminalExe = "C:\Program Files\MetaTrader 5\terminal64.exe",
  [string]$TerminalData = "$env:APPDATA\MetaQuotes\Terminal\D0E8209F77C8CF37AD8BF550E51FF075",
  [int]$Model = 1, # 1 = 1-minute OHLC (same as successful EURJPY/EURNZD exports)
  [switch]$SkipExisting
)

$ErrorActionPreference = "Stop"

# Allow -Symbols AUDUSD,GBPUSD (single comma-string) as well as a true array.
if ($Symbols.Count -eq 1 -and $Symbols[0] -match ",") {
  $Symbols = @($Symbols[0] -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}

# Markets that don't have clean 2016 history on IC Markets.
$FromOverride = @{
  "BTCUSD" = "2017.07.01"
  "USTEC"  = "2018.01.01"
  "XTIUSD" = "2018.01.01"
  "US100"  = "2018.01.01"
}

if (-not (Test-Path $TerminalExe)) {
  throw "MT5 terminal not found at $TerminalExe"
}
$ea = Join-Path $TerminalData "MQL5\Experts\SQ_TickDataExportEA.ex5"
if (-not (Test-Path $ea)) {
  throw "SQ_TickDataExportEA.ex5 missing at $ea - compile it in MetaEditor first"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$work = Join-Path $OutDir "_tester_configs"
New-Item -ItemType Directory -Force -Path $work | Out-Null
$logPath = Join-Path $OutDir "export_log.txt"

function Write-Log([string]$msg) {
  $line = "{0} {1}" -f (Get-Date -Format "yyyy-MM-dd HH:mm:ss"), $msg
  Add-Content -Path $logPath -Value $line
  Write-Host $line
}

function Find-TickExport([string]$symbol) {
  $roots = @(
    (Join-Path $env:APPDATA "MetaQuotes\Tester"),
    (Join-Path $TerminalData "Tester")
  )
  $hits = @()
  foreach ($root in $roots) {
    if (-not (Test-Path $root)) { continue }
    $hits += Get-ChildItem $root -Recurse -Filter "${symbol}_TickData.csv" -ErrorAction SilentlyContinue
  }
  $hits | Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

function Export-Symbol([string]$symbol) {
  $from = if ($FromOverride.ContainsKey($symbol)) { $FromOverride[$symbol] } else { $DefaultFromDate }
  $dest = Join-Path $OutDir "${symbol}_TickData.csv"

  if ($SkipExisting -and (Test-Path $dest)) {
    $len = (Get-Item $dest).Length
    if ($len -gt 10MB) {
      Write-Log "SKIP $symbol (already have $([math]::Round($len/1MB,1)) MB)"
      return
    }
  }

  Write-Log "START $symbol from $from to $ToDate (model=$Model)"

  # Kill any running terminal so /config is exclusive.
  Get-Process terminal64 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2

  $ini = Join-Path $work "${symbol}.ini"
  @"
[Common]
Login=7973647
Server=ICMarketsSC-MT5-2
KeepPrivate=1

[Tester]
Expert=SQ_TickDataExportEA.ex5
Symbol=$symbol
Period=H1
Optimization=0
Model=$Model
FromDate=$from
ToDate=$ToDate
ForwardMode=0
Deposit=10000
Currency=USD
ProfitInPips=0
Leverage=100
ExecutionMode=0
OptimizationCriterion=0
Visual=0
ShutdownTerminal=1
ReplaceReport=1
[TesterInputs]
"@ | Set-Content -Path $ini -Encoding ASCII

  $arg = '/config:' + $ini
  $proc = Start-Process -FilePath $TerminalExe -ArgumentList $arg -PassThru
  $timeoutSec = 6 * 60 * 60 # 6h hard cap per symbol
  if (-not $proc.WaitForExit($timeoutSec * 1000)) {
    Write-Log "TIMEOUT $symbol - killing terminal"
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    throw "Timed out exporting $symbol"
  }
  Write-Log "terminal exit code $($proc.ExitCode) for $symbol"

  Start-Sleep -Seconds 2
  $found = Find-TickExport $symbol
  if (-not $found) {
    # Live-terminal fallback (rare for Strategy Tester exports).
    $alt = Join-Path $TerminalData "MQL5\Files\${symbol}_TickData.csv"
    if (Test-Path $alt) { $found = Get-Item $alt }
  }
  if (-not $found) {
    throw "Export finished but ${symbol}_TickData.csv was not found under MetaQuotes/Tester"
  }
  if ($found.Length -lt 10MB) {
    throw "Export for $symbol is only $([math]::Round($found.Length/1MB,2)) MB - expected a multi-year dump"
  }

  Copy-Item -Force $found.FullName $dest
  Write-Log ("DONE {0} -> {1} ({2:N1} MB)" -f $symbol, $dest, ($found.Length / 1MB))
}

Write-Log "=== batch begin ($($Symbols.Count) symbols) -> $OutDir ==="
$failed = @()
foreach ($sym in $Symbols) {
  try {
    Export-Symbol $sym
  } catch {
    Write-Log "FAIL $sym : $_"
    $failed += $sym
  }
}
Write-Log "=== batch end; failed=$($failed -join ',') ==="
if ($failed.Count -gt 0) { exit 1 }

