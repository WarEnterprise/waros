$KernelDir = Split-Path -Parent $PSScriptRoot
$TargetDir = Join-Path $KernelDir "target"
$KernelBin = Join-Path $TargetDir "x86_64-unknown-none\release\waros-kernel"
$RepoRoot = Split-Path -Parent $KernelDir
$ImageBuilderManifest = Join-Path $RepoRoot "tools\kernel-image-builder\Cargo.toml"

Set-Location $KernelDir

cargo +nightly build --release --target x86_64-unknown-none
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$null = Set-Location $RepoRoot
cargo +nightly run --manifest-path $ImageBuilderManifest --target x86_64-pc-windows-msvc --release -- $KernelBin $TargetDir
exit $LASTEXITCODE
