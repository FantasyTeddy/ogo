# Sets up a PowerShell session for building. Dot-source it once per shell:
#   . .\env.ps1

# Rust Xtensa toolchain pinned in rust-toolchain.toml (export file written by `espup install`)
. "$HOME\export-esp-1.97.ps1"

# ESP-IDF 5.5 can't install its Python requirements on Python 3.15, and embuild
# always uses the first `python` on PATH, so put Python 3.12 in front.
$python312 = "$env:LOCALAPPDATA\Python\pythoncore-3.12-64"
if (-not (Test-Path "$python312\python.exe")) {
    throw "Python 3.12 not found at $python312"
}
$env:PATH = "$python312;$python312\Scripts;$env:PATH"
