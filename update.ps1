# Rebuild cdev from this checkout and (re)install it into ~/.cargo/bin.
#
# Run it after changing anything under src/, sdk/ or web/. The SDKs and the web
# panel are compiled into the binary, so edits to them need a reinstall too.
#
#   .\update.ps1          rebuild and reinstall
#   .\update.ps1 -Force   first stop a running cdev (and the app it wraps)

param([switch]$Force)

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "cargo not found on PATH. Install Rust from https://rustup.rs first." -ForegroundColor Red
    exit 1
}

$root = if ($env:CARGO_INSTALL_ROOT) { $env:CARGO_INSTALL_ROOT } elseif ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$installed = Join-Path $root 'bin\cdev.exe'

# Windows can't overwrite an .exe that is running, so the build or the copy
# fails while either of these is open.
$locked = @($installed, (Join-Path $PSScriptRoot 'target\release\cdev.exe'))
$running = @(Get-Process cdev -ErrorAction SilentlyContinue | Where-Object { $locked -contains $_.Path })
if ($running.Count -gt 0) {
    $ids = ($running | ForEach-Object { $_.Id }) -join ', '
    if (-not $Force) {
        Write-Host "cdev is running (PID $ids), so its .exe can't be replaced." -ForegroundColor Yellow
        Write-Host "Quit it (q in the TUI) or rerun with -Force to stop it." -ForegroundColor Yellow
        exit 1
    }
    Write-Host "Stopping cdev (PID $ids)"
    # /T takes the wrapped app down with it, as quitting cdev normally would
    foreach ($p in $running) { taskkill /T /F /PID $p.Id | Out-Null }
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}

# --locked: build with the committed Cargo.lock, the same versions `cargo build` uses
cargo install --path $PSScriptRoot --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host ""
Write-Host "Installed $(& $installed --version) -> $installed" -ForegroundColor Green

$onPath = Get-Command cdev -ErrorAction SilentlyContinue
if (-not $onPath) {
    Write-Host "$(Split-Path $installed) is not on PATH, so `cdev` won't resolve in a new shell." -ForegroundColor Yellow
} elseif ($onPath.Source -ne $installed) {
    Write-Host "Note: `cdev` on PATH resolves to $($onPath.Source), not the copy just installed." -ForegroundColor Yellow
}
