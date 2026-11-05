# Shared helpers; dot-source me.
Add-Type -Path "$PSScriptRoot\Win.cs" -ReferencedAssemblies System.Drawing
Add-Type -Path "$PSScriptRoot\Input.cs"
[W]::SetProcessDpiAwarenessContext([IntPtr]::new(-4)) | Out-Null
$TscHz = [W]::CalibrateTsc(); $env:TSC_HZ = [string][int64]$TscHz
$Root = (Resolve-Path "$PSScriptRoot\..").Path
$CaperExe = "$Root\caper\apps\native\desktop\target\x86_64-pc-windows-msvc\release\caper-desktop.exe"
if (-not $global:ClientLabel) { $global:ClientLabel = "caper-real-windows" }
$Frames = "$Root\results\caper-frames.txt"
$Control = "http://127.0.0.1:3901/__fixture/control"

function UnixMicros { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() * 1000 }

function Get-Caper {
  $p = Get-Process caper-desktop -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $p) { throw "caper-desktop not running" }
  $h = [W]::FindMain($p.Id)
  [pscustomobject]@{ Process = $p; Hwnd = $h; UiThread = [W]::UiThread($h) }
}

function Get-Pct([double[]]$sorted, [double]$p) {
  if ($sorted.Count -eq 0) { return 0 }
  $sorted[[int][Math]::Round(($sorted.Count - 1) * $p)]
}

# Runs $Action for $Seconds and writes one JSON line for the window.
function Measure-Window($c, [string]$Name, [double]$Seconds, [scriptblock]$Action, [string]$Out, [int]$Run) {
  $p = $c.Process; $p.Refresh()
  $ui0 = [W]::ThreadCpuSeconds($c.UiThread); $cpu0 = [W]::ProcessCpuSeconds($p.Id); $tick0 = $p.TotalProcessorTime.TotalSeconds
  $gpu = [GpuSampler]::new($p.Id); $gpu.Start()
  $t0 = UnixMicros; $sw = [Diagnostics.Stopwatch]::StartNew()
  & $Action
  while ($sw.Elapsed.TotalSeconds -lt $Seconds) { Start-Sleep -Milliseconds 10 }
  $t1 = UnixMicros; $wall = $sw.Elapsed.TotalSeconds
  $gpu.Stop()
  $p.Refresh()
  $cpu1 = [W]::ProcessCpuSeconds($p.Id); $ui1 = [W]::ThreadCpuSeconds($c.UiThread); $tick1 = $p.TotalProcessorTime.TotalSeconds
  $lines = Get-Content $Frames | ForEach-Object { $a = $_ -split ' '; if ($a.Count -eq 2) { [pscustomobject]@{ t = [int64]$a[0]; u = [double]$a[1] / 1000 } } } | Where-Object { $_.t -ge $t0 -and $_.t -le $t1 }
  $upd = @($lines | ForEach-Object { $_.u } | Sort-Object)
  $ts = @($lines | ForEach-Object { $_.t })
  $iv = @(); for ($i = 1; $i -lt $ts.Count; $i++) { $iv += ($ts[$i] - $ts[$i - 1]) / 1000.0 }
  $iv = @($iv | Sort-Object)
  $inv = [Globalization.CultureInfo]::InvariantCulture
  $rec = [ordered]@{
    client = $global:ClientLabel; scenario = $Name; run = $Run; wall_s = [Math]::Round($wall, 2)
    process_cpu_pct = [Math]::Round(($cpu1 - $cpu0) / $wall * 100, 1)
    ui_thread_cpu_pct = [Math]::Round(($ui1 - $ui0) / $wall * 100, 1)
    process_cpu_pct_ticks = [Math]::Round(($tick1 - $tick0) / $wall * 100, 1)
    ui_passes = $upd.Count; passes_per_s = [Math]::Round($upd.Count / $wall, 1)
    update_ms_median = [Math]::Round((Get-Pct $upd 0.5), 2); update_ms_p95 = [Math]::Round((Get-Pct $upd 0.95), 2)
    update_ms_max = $(if ($upd.Count) { [Math]::Round($upd[-1], 2) } else { 0 })
    update_share_pct = [Math]::Round((($upd | Measure-Object -Sum).Sum / 1000) / $wall * 100, 1)
    pass_interval_p50_ms = [Math]::Round((Get-Pct $iv 0.5), 2); pass_interval_p95_ms = [Math]::Round((Get-Pct $iv 0.95), 2)
    pass_interval_p99_ms = [Math]::Round((Get-Pct $iv 0.99), 2); intervals_over_33ms = @($iv | Where-Object { $_ -gt 33.4 }).Count
    private_mib = [Math]::Round($p.PrivateMemorySize64 / 1MB, 1); working_set_mib = [Math]::Round($p.WorkingSet64 / 1MB, 1)
  }
  $json = ($rec | ConvertTo-Json -Compress)
  $json = $json.TrimEnd('}') + ',"gpu":' + $gpu.Json() + '}'
  Add-Content -Path $Out -Value $json
  $json
}


