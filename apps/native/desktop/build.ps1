$ErrorActionPreference = "Stop"
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
  throw "Caper-windows-x64 packages require an x64 Windows build host."
}
$Native = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Resolve-Path (Join-Path $Native "..\..\..")
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = "2" }
$env:CARGO_TARGET_DIR = Join-Path $Native "target"

cargo fmt --manifest-path (Join-Path $Native "Cargo.toml") -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --manifest-path (Join-Path $Native "Cargo.toml") --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy --manifest-path (Join-Path $Native "Cargo.toml") --locked --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo build --manifest-path (Join-Path $Native "Cargo.toml") --locked --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Dist = Join-Path $Native "dist"
$Stage = Join-Path $Native "target\package\Caper-windows-x64"
Remove-Item $Dist, $Stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item $Dist, $Stage -ItemType Directory -Force | Out-Null
Copy-Item (Join-Path $Native "target\release\caper-desktop.exe") (Join-Path $Stage "Caper.exe")
Copy-Item (Join-Path $Root "LICENSE"), (Join-Path $Native "README.md"), (Join-Path $Native "THIRD-PARTY-NOTICES.md") $Stage
$Output = Join-Path $Dist "Caper-windows-x64.zip"
Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $Output -CompressionLevel Optimal
Write-Host "Built: $Output (unsigned portable application)"
