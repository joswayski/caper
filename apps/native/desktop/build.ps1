$ErrorActionPreference = "Stop"
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
  throw "Caper-windows-x64 packages require an x64 Windows build host."
}
$Native = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Resolve-Path (Join-Path $Native "..\..\..")
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = "2" }
$env:CARGO_TARGET_DIR = Join-Path $Native "target"
$Target = "x86_64-pc-windows-msvc"
# A portable download must not require a separately installed VC++ runtime.
# An explicit target keeps this flag off host-built procedural macros.
$env:RUSTFLAGS = "$env:RUSTFLAGS -C target-feature=+crt-static".Trim()
$env:LK_CUSTOM_WEBRTC = python (Join-Path $Native "voice-spike\fetch_libwebrtc.py") --platform windows
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Verified official ONNX Runtime 1.23.2 x64 binary. Never package an
# unverified download; the native library is loaded from beside Caper.exe.
$OrtArchive = Join-Path $Native "target\onnxruntime-win-x64-1.23.2.zip"
$OrtExtract = Join-Path $Native "target\onnxruntime-win-x64-1.23.2"
New-Item (Join-Path $Native "target") -ItemType Directory -Force | Out-Null
if (-not (Test-Path $OrtArchive)) {
  Invoke-WebRequest 'https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/onnxruntime-win-x64-1.23.2.zip' -OutFile $OrtArchive
}
if ((Get-FileHash $OrtArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '0b38df9af21834e41e73d602d90db5cb06dbd1ca618948b8f1d66d607ac9f3cd') {
  throw 'ONNX Runtime download hash mismatch'
}
Expand-Archive $OrtArchive -DestinationPath (Join-Path $Native "target") -Force
$env:CAPER_ONNXRUNTIME_LIBRARY = Join-Path $OrtExtract 'lib\onnxruntime.dll'
if ((Get-FileHash $env:CAPER_ONNXRUNTIME_LIBRARY -Algorithm SHA256).Hash.ToLowerInvariant() -ne 'dec964ab1ee36cc9b0ae247d13b376627992fc57dec0454354017ab8fd84f1ea') {
  throw 'ONNX Runtime library hash mismatch'
}
if ((Get-FileHash (Join-Path $Root 'apps\web\public\audio\dpdfnet8-v2\dpdfnet8_48khz_hr.onnx') -Algorithm SHA256).Hash.ToLowerInvariant() -ne '7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631') {
  throw 'DPDFNet model hash mismatch'
}

python (Join-Path $Root "scripts\native_fonts.py")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo fmt --manifest-path (Join-Path $Native "Cargo.toml") --package caper-desktop -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --manifest-path (Join-Path $Native "Cargo.toml") --package caper-desktop --target $Target --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --manifest-path (Join-Path $Native "Cargo.toml") --package caper-desktop --target $Target --locked native_inference_is_finite_and_owns_fresh_state -- --ignored
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy --manifest-path (Join-Path $Native "Cargo.toml") --package caper-desktop --target $Target --locked --all-targets --no-deps -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo build --manifest-path (Join-Path $Native "Cargo.toml") --package caper-desktop --target $Target --locked --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Dist = Join-Path $Native "dist"
$Stage = Join-Path $Native "target\package\Caper-windows-x64"
Remove-Item $Dist, $Stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item $Dist, $Stage -ItemType Directory -Force | Out-Null
Copy-Item (Join-Path $Native "target\$Target\release\caper-desktop.exe") (Join-Path $Stage "Caper.exe")
Copy-Item (Join-Path $Root "LICENSE"), (Join-Path $Native "README.md"), (Join-Path $Native "THIRD-PARTY-NOTICES.md") $Stage
Copy-Item (Join-Path $Root "shared\fonts\cache\Satoshi-FFL.txt") $Stage
Copy-Item (Join-Path $Native "voice-spike\licenses\*") $Stage
Copy-Item $env:CAPER_ONNXRUNTIME_LIBRARY $Stage
Copy-Item (Join-Path $OrtExtract 'LICENSE') (Join-Path $Stage 'ONNX-RUNTIME-LICENSE')
Copy-Item (Join-Path $OrtExtract 'ThirdPartyNotices.txt') (Join-Path $Stage 'ONNX-RUNTIME-THIRD-PARTY-NOTICES.txt')
Copy-Item (Join-Path $Root 'apps\web\public\audio\dpdfnet8-v2\LICENSE-APACHE-2.0') (Join-Path $Stage 'DPDFNET-LICENSE')
# ORT's official DLL is /MD even though the Rust executable uses +crt-static.
# Stage Microsoft's signed app-local VC++ runtime from the VS build toolchain;
# do not claim a portable package that depends on an installed redistributable.
if (-not $env:VCToolsRedistDir) { throw 'Run from an x64 Visual Studio developer environment (VCToolsRedistDir is required)' }
$Crt = Get-ChildItem (Join-Path $env:VCToolsRedistDir 'x64\Microsoft.VC*.CRT') -Directory -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $Crt) { throw 'Microsoft VC++ app-local runtime not found in Visual Studio redist directory' }
foreach ($Dll in @('MSVCP140.dll', 'MSVCP140_1.dll', 'VCRUNTIME140.dll', 'VCRUNTIME140_1.dll')) {
  $Source = Join-Path $Crt.FullName $Dll
  if (-not (Test-Path $Source) -or (Get-AuthenticodeSignature $Source).Status -ne 'Valid') {
    throw "Signed Microsoft VC++ runtime DLL missing: $Dll"
  }
  Copy-Item $Source $Stage
}
$Output = Join-Path $Dist "Caper-windows-x64.zip"
Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $Output -CompressionLevel Optimal
Write-Host "Built: $Output (unsigned portable application)"
