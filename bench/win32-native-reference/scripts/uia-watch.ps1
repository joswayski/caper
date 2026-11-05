# Acts as a UI Automation client for every new bench app window.
# -Mode once: one full tree walk as soon as the window exists (switches the app's a11y tree on), then stays registered
#             for structure/property events (like a screen reader would) without polling.
# -Mode loop: walks the full tree again every 500 ms (a heavy client, like inspect.exe's live tree).
param([string]$Mode = "once", [string]$Dir = "$PSScriptRoot\..\out\watch", [int]$Seconds = 3600)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
New-Item -ItemType Directory -Force $Dir | Out-Null
$A = [System.Windows.Automation.AutomationElement]
$dump = "$PSScriptRoot\uia-dump.ps1"
$seen = @{}
Add-Type -ReferencedAssemblies UIAutomationClient, UIAutomationTypes, WindowsBase -TypeDefinition @"
using System.Windows.Automation;
public static class Ev {
  public static int Count;
  public static void On(object s, StructureChangedEventArgs e) { System.Threading.Interlocked.Increment(ref Count); }
  public static StructureChangedEventHandler H = new StructureChangedEventHandler(On);
}
"@
$handler = [Ev]::H
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
  foreach ($p in Get-Process egui37-chat, gpui-chat, gpui-prof, native-chat -ErrorAction SilentlyContinue) {
    if ($p.MainWindowHandle -eq [IntPtr]::Zero) { continue }
    if (-not $seen.ContainsKey($p.Id)) {
      $seen[$p.Id] = 1
      & $dump -ProcessId $p.Id -Out "$Dir\$($p.ProcessName)-$($p.Id).txt"
      if ($Mode -eq "once") {
        try {
          $cond = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $p.Id)
          $win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
          [System.Windows.Automation.Automation]::AddStructureChangedEventHandler($win, [System.Windows.Automation.TreeScope]::Subtree, $handler)
        } catch {}
      }
    } elseif ($Mode -eq "loop") {
      & $dump -ProcessId $p.Id -Out "$Dir\$($p.ProcessName)-$($p.Id)-loop.txt"
    }
  }
  Start-Sleep -Milliseconds 500
}
