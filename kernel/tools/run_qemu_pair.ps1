$KernelDir = Split-Path -Parent $PSScriptRoot
$ImagePath = Join-Path $KernelDir "target\waros-bios.img"
$QemuExe = "C:\Program Files\qemu\qemu-system-x86_64.exe"

if (-not (Test-Path $QemuExe)) {
    Write-Error "QEMU not found at '$QemuExe'."
    exit 1
}

if (-not (Test-Path $ImagePath)) {
    Write-Error "Kernel image not found at '$ImagePath'. Run .\create_image.ps1 first."
    exit 1
}

Write-Host "Start this command in one PowerShell window for node A:"
Write-Host
Write-Host "& `"$QemuExe`" -drive format=raw,file=`"$ImagePath`" -m 128M -serial stdio -serial tcp:127.0.0.1:4444,server,nowait -no-reboot -no-shutdown"
Write-Host
Write-Host "Start this command in a second PowerShell window for node B:"
Write-Host
Write-Host "& `"$QemuExe`" -drive format=raw,file=`"$ImagePath`" -m 128M -serial stdio -serial tcp:127.0.0.1:4444 -no-reboot -no-shutdown"
