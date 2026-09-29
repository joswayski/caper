param([Parameter(Mandatory)] [string] $Setup, [Parameter(Mandatory)] [string] $Stage)
$ErrorActionPreference = 'Stop'
# Exercise the real installer on disposable CI hosts, without launching media.
$Install = Join-Path $env:LOCALAPPDATA 'Programs\Caper'
$Registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Caper'
if ((Test-Path $Install) -or (Test-Path $Registry)) { throw 'Installer test requires a clean host without Caper installed' }
$Shell = New-Object -ComObject WScript.Shell
$Links = @((Join-Path $Shell.SpecialFolders.Item('Programs') 'Caper.lnk'), (Join-Path $Shell.SpecialFolders.Item('Desktop') 'Caper.lnk'))
foreach ($Link in $Links) { if (Test-Path $Link) { throw "Existing shortcut: $Link" } }
# Inspect the actual PE header, not just the Rust source attribute.
$Bytes = [IO.File]::ReadAllBytes((Join-Path $Stage 'Caper.exe'))
$Pe = [BitConverter]::ToInt32($Bytes, 0x3c)
if ([BitConverter]::ToUInt16($Bytes, $Pe + 24 + 68) -ne 2) { throw 'Caper.exe must use the Windows GUI subsystem (no console)' }
foreach ($Attempt in 1..2) {
  $Process = Start-Process -FilePath $Setup -ArgumentList '/S' -Wait -PassThru
  if ($Process.ExitCode -ne 0) { throw "Installer failed: $($Process.ExitCode)" }
  foreach ($File in Get-ChildItem $Stage -File) {
    $Installed = Join-Path $Install "app\$($File.Name)"
    if ((Get-FileHash $Installed).Hash -ne (Get-FileHash $File.FullName).Hash) { throw "Installed file mismatch: $Installed" }
  }
  foreach ($Link in $Links) {
    if ($Shell.CreateShortcut($Link).TargetPath -ne (Join-Path $Install 'app\Caper.exe')) { throw "Wrong shortcut: $Link" }
  }
  if ((Get-ItemProperty $Registry).DisplayName -ne 'Caper') { throw 'Missing installed-app registration' }
}
# Run in place without NSIS's asynchronous temporary-uninstaller handoff.
$Uninstaller = Join-Path $env:TEMP "caper-uninstall-test-$PID.exe"
Copy-Item (Join-Path $Install 'Uninstall.exe') $Uninstaller
try {
  $Process = Start-Process -FilePath $Uninstaller -ArgumentList "/S _?=$Install" -Wait -PassThru
  if ($Process.ExitCode -ne 0) { throw "Uninstaller failed: $($Process.ExitCode)" }
  foreach ($Path in @($Registry, $Install) + $Links) {
    if (Test-Path $Path) { throw "Uninstaller left behind: $Path" }
  }
} finally { Remove-Item $Uninstaller -Force }
Write-Host 'PASS: GUI subsystem, install, reinstall, payload hashes, shortcuts, registration, uninstall'
