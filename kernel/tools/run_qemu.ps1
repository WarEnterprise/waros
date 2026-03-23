$KernelDir = Split-Path -Parent $PSScriptRoot
$Image = Join-Path $KernelDir "target\waros-bios.img"

& "$PSScriptRoot\create_image.ps1"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$Qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
if ($null -eq $Qemu) {
    $DefaultQemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
    if (Test-Path $DefaultQemu) {
        $Qemu = [pscustomobject]@{ Source = $DefaultQemu }
    } else {
        Write-Error "qemu-system-x86_64 was not found in PATH or at '$DefaultQemu'."
        exit 1
    }
}

& $Qemu.Source `
    -drive format=raw,file=$Image `
    -m 256M `
    -serial stdio `
    -netdev user,id=net0 `
    -device virtio-net-pci,netdev=net0 `
    -k pt-br `
    -display sdl `
    -no-reboot `
    -no-shutdown
