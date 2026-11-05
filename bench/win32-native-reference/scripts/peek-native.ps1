# Launches an interactive (no BENCH) recreation, screenshots its default state, then hovers a
# row, clicks the composer and types, drags a text selection, and screenshots each state.
param([string]$Exe = "$PSScriptRoot\..\native-chat\target\release\native-chat.exe", [string]$Name = "native")
. "$PSScriptRoot\lib.ps1"
[W]::SetProcessDpiAwarenessContext([IntPtr]::new(-4)) | Out-Null
$dir = "$PSScriptRoot\..\out"; New-Item -ItemType Directory -Force $dir | Out-Null
Remove-Item Env:BENCH -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $Exe -PassThru
$h = [IntPtr]::Zero; $t = [Diagnostics.Stopwatch]::StartNew()
while ($h -eq [IntPtr]::Zero -and $t.Elapsed.TotalSeconds -lt 10) { Start-Sleep -m 50; $h = [W]::FindMain($p.Id) }
[W]::Place($h, 80, 80, 0, 0)
Start-Sleep -Seconds 2
[W]::Screenshot($h, "$dir\$Name-default.png")
$c = [W]::Client($h); $s = [W]::GetDpiForWindow($h) / 96.0
# hover a row in the middle of the history
[W]::SetCursorPos($c[0] + [int](600 * $s), $c[1] + [int](420 * $s)); Start-Sleep -m 400
[W]::Screenshot($h, "$dir\$Name-hover.png")
# drag-select message text
[W]::SetCursorPos($c[0] + [int](380 * $s), $c[1] + [int](300 * $s)); Start-Sleep -m 100
[In]::Down(0); for ($i = 1; $i -le 20; $i++) { [W]::SetCursorPos($c[0] + [int]((380 + 20 * $i) * $s), $c[1] + [int]((300 + 6 * $i) * $s)); Start-Sleep -m 20 }; [In]::Up(0)
Start-Sleep -m 300
[W]::Screenshot($h, "$dir\$Name-select.png")
# click the composer and type
[W]::SetCursorPos($c[0] + [int](600 * $s), $c[3] + $c[1] - [int](40 * $s)); Start-Sleep -m 100; [In]::Down(0); [In]::Up(0); Start-Sleep -m 200
[W]::TypeString("Hello from the native client, wrapping a fairly long draft message to check it grows ")
Start-Sleep -m 400
[W]::Screenshot($h, "$dir\$Name-typing.png")
"exited=$($p.HasExited)"
$p.Kill()
