# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build for Windows and stage a folder, optionally zipped.
#
#   powershell -ExecutionPolicy Bypass -File packaging\windows.ps1
#   powershell -ExecutionPolicy Bypass -File packaging\windows.ps1 -Zip
#
# Needs the MSVC build tools, which the Rust installer offers to fetch.
# Nothing else: the keyboard speaks HID, which Windows drives itself.

param([switch]$Zip)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root "dist\windows"

Write-Host "==> building"
cargo build --release --manifest-path (Join-Path $root "Cargo.toml")

Write-Host "==> icons"
python3 (Join-Path $root "packaging\make-icon.py") | Out-Null

New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item (Join-Path $root "target\release\spawn-universal.exe") $dist -Force
Copy-Item (Join-Path $root "packaging\icons\icon.ico") $dist -Force
Copy-Item (Join-Path $root "README.md") $dist -Force
Copy-Item (Join-Path $root "LICENSE") $dist -Force

# Close the vendor software before running: Windows hands out HID devices
# exclusively, and whichever program opens the keyboard first keeps it.
@"
Spawn Universal

Run spawn-universal.exe. If the keyboard is not found, close the
manufacturer's software first: only one program can hold it at a time.

Licensed GPL-3.0-or-later. Source: https://github.com/oops404/spawn-universal
"@ | Set-Content (Join-Path $dist "READ ME FIRST.txt")

Write-Host "==> $dist"

if ($Zip) {
    $version = (Select-String -Path (Join-Path $root "Cargo.toml") `
        -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    $zip = Join-Path $root "dist\spawn-universal-$version-windows.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path "$dist\*" -DestinationPath $zip
    Write-Host "==> $zip"
}
