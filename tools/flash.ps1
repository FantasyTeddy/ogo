<#
.SYNOPSIS
Builds and flashes the launcher (and optionally the recovery app) over USB.

.DESCRIPTION
Uses the bootloader built with this project's sdkconfig (MENU at power-on starts recovery), the
partition table from partitions.csv, and puts the launcher into launcher_a.

.EXAMPLE
.\tools\flash.ps1                  # launcher only
.\tools\flash.ps1 -All             # recovery + launcher, clears the boot selection (first time)
.\tools\flash.ps1 -Monitor         # launcher, then open the serial monitor
.\tools\flash.ps1 -SaveImage dist\launcher.bin   # no flashing: a launcher image for the web page
#>
param(
    [string]$Port = "COM7",
    # Also flash the recovery app and clear the boot selection; needed after a partition table change.
    [switch]$All,
    [switch]$Monitor,
    # Instead of flashing, write a launcher image to this file, for "Update launcher" on the web page.
    [string]$SaveImage,
    # Extra cargo features for the launcher, e.g. "rollback-test".
    [string]$Features
)

# No $ErrorActionPreference = "Stop": Windows PowerShell treats cargo's progress output on stderr
# as errors then. Exit codes are checked instead.
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    . "$root\env.ps1"

    $featureArgs = @()
    if ($Features) { $featureArgs = @("--features", $Features) }

    if ($SaveImage) {
        New-Item -ItemType Directory -Force (Split-Path -Parent $SaveImage) | Out-Null
        cargo espflash save-image --chip esp32 --flash-size 16mb --release --package ogo-launcher @featureArgs $SaveImage
        if ($LASTEXITCODE -ne 0) { throw "Building the launcher image failed" }
        Write-Host "Launcher image: $SaveImage"
        return
    }

    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "Build failed" }

    # esp-idf-sys builds the bootloader next to ESP-IDF; take the newest one.
    $targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
    $bootloader = Get-ChildItem "$targetDir\xtensa-esp32-espidf\release\build\esp-idf-sys-*\out\build\bootloader\bootloader.bin" |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $bootloader) { throw "No bootloader.bin found under $targetDir" }
    Write-Host "Bootloader: $($bootloader.FullName)"

    $common = @("--release", "--bootloader", $bootloader.FullName, "--port", $Port, "--baud", "921600")
    if ($All) {
        cargo espflash flash @common --package ogo-recovery --target-app-partition recovery
        if ($LASTEXITCODE -ne 0) { throw "Flashing the recovery app failed" }
    }
    # Clearing otadata makes the freshly flashed launcher_a run even if a web update had selected
    # launcher_b: the bootloader starts recovery, which picks the first valid launcher slot.
    $launcher = @("--package", "ogo-launcher", "--target-app-partition", "launcher_a", "--erase-parts", "otadata") + $featureArgs
    if ($Monitor) { $launcher += "--monitor" }
    cargo espflash flash @common @launcher
    if ($LASTEXITCODE -ne 0) { throw "Flashing the launcher failed" }
}
finally {
    Pop-Location
}
