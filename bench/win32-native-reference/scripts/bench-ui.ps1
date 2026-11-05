# Runs the recreation scenarios on Windows. Each run appends the app's own JSON
# line to $Out, then a gpu line (GPU Engine counters for the process) to $Out-gpu.
# Usage: bench-ui.ps1 -Out results.jsonl -Repeat 3 [-Scenarios ...] [-Variants ...]
param(
  [string]$Out = "$PSScriptRoot\..\out\ui.jsonl",
  [int]$Repeat = 3,
  [string]$Scenarios = "startup,idle,scroll,burst,resize,typing,hover",
  [string]$Variants = "native"
)
$ErrorActionPreference = "Stop"
$ScenarioList = $Scenarios -split "[, ]+"; $VariantList = $Variants -split "[, ]+"
. "$PSScriptRoot\lib.ps1"
[W]::SetProcessDpiAwarenessContext([IntPtr]::new(-4)) | Out-Null
# Add more apps here (e.g. "egui37" = "path\to\egui37-chat.exe") to compare in the same run.
$Bin = @{
  "native" = "$PSScriptRoot\..\native-chat\target\release\native-chat.exe"
}
$Length = @{ startup = 0; idle = 10; scroll = 8; burst = 10; resize = 6; typing = 8; hover = 8 }
New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
$GpuOut = $Out -replace '\.jsonl$', '-gpu.jsonl'

for ($r = 1; $r -le $Repeat; $r++) {
  foreach ($v in $VariantList) {
    foreach ($s in $ScenarioList) {
      [W]::SetCursorPos(5, 5) | Out-Null
      $env:BENCH = $s; $env:BENCH_OUT = $Out
      if ($v -match "styled") { $env:GPUI_TEXT = "styled" } else { Remove-Item Env:GPUI_TEXT -ErrorAction SilentlyContinue }
      $tag = Join-Path (Split-Path $Out) "$v-$s-$r"
      if ($env:PHASES -and $v -match "^prof") { $env:GPUI_PHASES = "$tag.csv" } else { Remove-Item Env:GPUI_PHASES -ErrorAction SilentlyContinue }
      if ($env:PHASES -and $v -match "^(egui|native)") { $env:CAPER_BENCH_FRAMES = "$tag.probe" } else { Remove-Item Env:CAPER_BENCH_FRAMES -ErrorAction SilentlyContinue }
      if ($v -match "^cached") { $env:GPUI_CACHED = "1" } else { Remove-Item Env:GPUI_CACHED -ErrorAction SilentlyContinue }
      if ($v -match "gc-") { $env:GPUI_GLYPHCACHE = "1" } else { Remove-Item Env:GPUI_GLYPHCACHE -ErrorAction SilentlyContinue }
      if ($v -match "a11y") { $env:GPUI_A11Y = "1" } else { Remove-Item Env:GPUI_A11Y -ErrorAction SilentlyContinue }
      if ($v -match "cull") { $env:GPUI_CULL = "1" } else { Remove-Item Env:GPUI_CULL -ErrorAction SilentlyContinue }
      if ($env:SAMPLE) { $env:SAMPLE_OUT = "$tag.samples" } else { Remove-Item Env:SAMPLE_OUT -ErrorAction SilentlyContinue }
      $p = Start-Process -FilePath $Bin[$v] -PassThru -NoNewWindow
      $t0 = [Diagnostics.Stopwatch]::StartNew()
      $h = [IntPtr]::Zero
      while ($h -eq [IntPtr]::Zero -and $t0.Elapsed.TotalSeconds -lt 10 -and -not $p.HasExited) { Start-Sleep -Milliseconds 50; $h = [W]::FindMain($p.Id) }
      if ($h -ne [IntPtr]::Zero) { [W]::Place($h, 80, 80, 0, 0) }
      $gpu = $null
      if ($s -ne "startup") {
        while ($t0.Elapsed.TotalSeconds -lt 2.0) { Start-Sleep -Milliseconds 20 }
        $gpu = [GpuSampler]::new($p.Id); $gpu.Start()
        if ($s -eq "hover") {
          while ($t0.Elapsed.TotalSeconds -lt 2.5) { Start-Sleep -Milliseconds 20 }
          [W]::Hover($h, 9.0)
        }
        # Stop sampling when the app's own measuring window ends.
        while ($t0.Elapsed.TotalSeconds -lt 2.0 + $Length[$s] -and -not $p.HasExited) { Start-Sleep -Milliseconds 20 }
        $gpu.Stop()
      }
      if (-not $p.WaitForExit(60000)) { $p.Kill(); Write-Output "TIMEOUT $v $s run $r" }
      if ($gpu) {
        $g = $gpu.Json()
        Add-Content -Path $GpuOut -Value ("{""framework"":""$v"",""scenario"":""$s"",""run"":$r,""gpu"":$g}")
      }
      Write-Output "$v $s run $r exit $($p.ExitCode)"
      Start-Sleep -Seconds 1
    }
  }
}










