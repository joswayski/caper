# Dumps the UI Automation tree of a process's main window (raw view).
# Usage: uia-dump.ps1 -ProcessId N -Out file.txt [-MaxNodes 4000]
param([int]$ProcessId, [string]$Out, [int]$MaxNodes = 4000)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$cond = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $ProcessId)
$sw = [Diagnostics.Stopwatch]::StartNew()
$win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if (-not $win) { "no window for $ProcessId" | Set-Content $Out; exit 1 }
$walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
$pats = @{
  Value = [System.Windows.Automation.ValuePattern]::Pattern
  Text = [System.Windows.Automation.TextPattern]::Pattern
  Invoke = [System.Windows.Automation.InvokePattern]::Pattern
  SelectionItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
  Scroll = [System.Windows.Automation.ScrollPattern]::Pattern
  Toggle = [System.Windows.Automation.TogglePattern]::Pattern
}
$lines = New-Object System.Collections.Generic.List[string]
$stats = @{}
$n = 0
function Walk($el, $depth) {
  if ($script:n -ge $MaxNodes) { return }
  $script:n++
  try {
    $c = $el.Current
    $ct = $c.ControlType.ProgrammaticName -replace 'ControlType\.', ''
    $p = @(); foreach ($k in $pats.Keys) { $o = $null; if ($el.TryGetCurrentPattern($pats[$k], [ref]$o)) { $p += $k } }
    $extra = ''
    if ($p -contains 'Value') { $o = $el.GetCurrentPattern($pats.Value); $extra += " value=`"$($o.Current.Value)`"" }
    if ($p -contains 'Text') {
      $o = $el.GetCurrentPattern($pats.Text)
      try { $sel = $o.GetSelection(); $extra += " textSel=$($sel.Count)"; if ($sel.Count) { $extra += " selText=`"$($sel[0].GetText(60))`"" } } catch { $extra += " textSelErr" }
      try { $extra += " docText=`"$($o.DocumentRange.GetText(60))`"" } catch {}
    }
    $r = $c.BoundingRectangle
    $name = $c.Name; if ($name.Length -gt 80) { $name = $name.Substring(0, 80) + '…' }
    $lines.Add(("{0}{1} name=`"{2}`" id=`"{3}`" pats=[{4}] focusable={5} rect={6},{7},{8}x{9}{10}" -f ('  ' * $depth), $ct, $name, $c.AutomationId, ($p -join ','), $c.IsKeyboardFocusable, $(if ([double]::IsInfinity($r.X) -or $r.IsEmpty) { "inf" } else { [Math]::Round($r.X) }), $(if ($r.IsEmpty) { "" } else { [Math]::Round($r.Y) }), $(if ($r.IsEmpty) { "" } else { [Math]::Round($r.Width) }), $(if ($r.IsEmpty) { "" } else { [Math]::Round($r.Height) }), $extra))
    $key = $ct + $(if ($name) { '+named' } else { '' }); $stats[$key] = 1 + [int]$stats[$key]
  } catch { $lines.Add(('  ' * $depth) + "ERR $($_.Exception.Message)") }
  $ch = $walker.GetFirstChild($el)
  while ($ch -ne $null -and $script:n -lt $MaxNodes) { Walk $ch ($depth + 1); $ch = $walker.GetNextSibling($ch) }
}
Walk $win 0
$lines.Insert(0, "nodes=$n walk_ms=$([int]$sw.Elapsed.TotalMilliseconds)")
$lines.Insert(1, "counts: " + (($stats.GetEnumerator() | Sort-Object Name | % { "$($_.Name)=$($_.Value)" }) -join ' '))
$lines | Set-Content -Encoding utf8 $Out
$lines[0]; $lines[1]

