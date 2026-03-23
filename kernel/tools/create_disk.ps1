$KernelDir = Split-Path -Parent $PSScriptRoot
$DiskImage = Join-Path $KernelDir "target\waros-disk.img"

$QemuImg = Get-Command qemu-img -ErrorAction SilentlyContinue
if ($null -eq $QemuImg) {
    $DefaultQemuImg = "C:\Program Files\qemu\qemu-img.exe"
    if (Test-Path $DefaultQemuImg) {
        $QemuImg = [pscustomobject]@{ Source = $DefaultQemuImg }
    } else {
        Write-Error "qemu-img was not found in PATH or at '$DefaultQemuImg'."
        exit 1
    }
}

if (Test-Path $DiskImage) {
    Write-Host "Disk image already exists at $DiskImage"
    exit 0
}

& $QemuImg.Source create -f raw $DiskImage 64M
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Host "Created 64 MB disk image at $DiskImage"
