# UIA dump of the native entrant with a draft in the composer and part of it selected, plus
# direct Text-pattern checks (caret via TextPattern2, selection, value, Invoke on a channel).
. "$PSScriptRoot\lib.ps1"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$Dir = "$PSScriptRoot\..\out"
foreach ($k in "BENCH","BENCH_OUT") { Remove-Item "Env:$k" -ErrorAction SilentlyContinue }
$env:MESSAGES = "3000"
$p = Start-Process -FilePath "$PSScriptRoot\..\native-chat\target\release\native-chat.exe" -PassThru
$h = [IntPtr]::Zero; $t = [Diagnostics.Stopwatch]::StartNew()
while ($h -eq [IntPtr]::Zero -and $t.Elapsed.TotalSeconds -lt 10) { Start-Sleep -m 50; $h = [W]::FindMain($p.Id) }
[W]::Place($h, 80, 80, 0, 0); Start-Sleep -Seconds 2
$c = [W]::Client($h); $s = [W]::GetDpiForWindow($h) / 96.0
[In]::ClickAt($c[0] + [int](600 * $s), $c[1] + $c[3] - [int](40 * $s)); Start-Sleep -m 200
[W]::TypeString("hello native world")
for ($i = 0; $i -lt 5; $i++) { [W]::Key(0x25) }   # caret 5 left
Start-Sleep -m 200
& powershell -NoProfile -ExecutionPolicy Bypass -File $PSScriptRoot\uia-dump.ps1 -ProcessId $p.Id -Out "$Dir\uia-native.txt" | Out-Null
$A = [System.Windows.Automation.AutomationElement]
$win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $p.Id)))
$edit = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)))
$tp = $edit.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
$doc = $tp.DocumentRange
$sel = $tp.GetSelection()[0]
# caret offset = length of text from document start to the (degenerate) selection start
$pre = $doc.Clone(); $pre.MoveEndpointByRange([System.Windows.Automation.Text.TextPatternRangeEndpoint]::End, $sel, [System.Windows.Automation.Text.TextPatternRangeEndpoint]::Start)
$out = @("value=`"$($edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value)`"", "caret offset via selection range = $($pre.GetText(-1).Length) (expected 13)")
# select 'native' via UIA, then read the app's selection back
$r = $doc.FindText("native", $false, $false); $r.Select(); Start-Sleep -m 200
$out += "after Select('native'): selection text=`"$($tp.GetSelection()[0].GetText(-1))`" rects=$($tp.GetSelection()[0].GetBoundingRectangles().Count)"
$w = $doc.Clone(); $w.ExpandToEnclosingUnit([System.Windows.Automation.Text.TextUnit]::Word); $out += "first word=`"$($w.GetText(-1))`""
$ch = $win.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, "random")))
$ch.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); Start-Sleep -m 300
$hist = $win.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::List)))
$out += "after Invoke(random): composer name=`"$($edit.Current.Name)`""
[W]::Screenshot($h, "$PSScriptRoot\..\out\native-uia-select.png")
$out | Set-Content "$Dir\uia-native-checks.txt"; $out
$p.Kill()
