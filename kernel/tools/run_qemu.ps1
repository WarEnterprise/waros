$KernelDir = Split-Path -Parent $PSScriptRoot
$Image = Join-Path $KernelDir "target\waros.img"
$OvmfPath = $env:WAROS_OVMF_PATH

if ([string]::IsNullOrWhiteSpace($OvmfPath)) {
    $OvmfPath = "C:\Program Files\qemu\share\edk2-x86_64-code.fd"
}

& "$PSScriptRoot\create_image.ps1"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$Qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
if ($null -eq $Qemu) {
    Write-Error "qemu-system-x86_64 was not found in PATH."
    exit 1
}

if (-not (Test-Path $OvmfPath)) {
    Write-Error "OVMF firmware not found at '$OvmfPath'. Set WAROS_OVMF_PATH to a valid OVMF image."
    exit 1
}

& $Qemu.Source `
    -bios $OvmfPath `
    -drive format=raw,file=$Image `
    -m 128M `
    -serial stdio `
    -display sdl `
    -no-reboot `
    -no-shutdown
