# wyncast installer for Windows
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

# --- Paths ---
$BinDir = Join-Path $env:LOCALAPPDATA "Programs\wyncast"
$DataDir = Join-Path $env:APPDATA "wyncast"

Write-Host "Installing wyncast (Windows)..." -ForegroundColor Blue
Write-Host ""

# --- Install binary ---
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Copy-Item "$ScriptDir\bin\wyncast.exe" "$BinDir\wyncast.exe" -Force
Write-Host "[OK] Installed binary to $BinDir\wyncast.exe" -ForegroundColor Green

# --- Add to PATH ---
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -and $UserPath.Split(';') -contains $BinDir) {
    Write-Host "[OK] PATH already contains $BinDir" -ForegroundColor Green
} else {
    if ($UserPath) {
        $NewPath = "$UserPath;$BinDir"
    } else {
        $NewPath = $BinDir
    }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    Write-Host "[OK] Added $BinDir to user PATH" -ForegroundColor Green
    Write-Host "     Restart your terminal for PATH changes to take effect" -ForegroundColor Yellow
}

# --- Copy projection data ---
$ProjectionsDir = Join-Path $DataDir "projections"
New-Item -ItemType Directory -Force -Path $ProjectionsDir | Out-Null
Copy-Item "$ScriptDir\projections\hitters.csv" "$ProjectionsDir\hitters.csv" -Force
Copy-Item "$ScriptDir\projections\pitchers.csv" "$ProjectionsDir\pitchers.csv" -Force
Write-Host "[OK] Installed projection data to $ProjectionsDir" -ForegroundColor Green

# --- Copy extensions ---
$ExtensionsDir = Join-Path $DataDir "extensions"
$SourceExtensions = Join-Path $ScriptDir "extensions"

if (Test-Path $SourceExtensions) {
    $FirefoxSrc = Join-Path $SourceExtensions "firefox"
    if (Test-Path $FirefoxSrc) {
        $FirefoxDst = Join-Path $ExtensionsDir "firefox"
        if (Test-Path $FirefoxDst) { Remove-Item -Recurse -Force $FirefoxDst }
        New-Item -ItemType Directory -Force -Path $ExtensionsDir | Out-Null
        Copy-Item -Recurse $FirefoxSrc $FirefoxDst
        Write-Host "[OK] Installed Firefox extension to $FirefoxDst" -ForegroundColor Green
    }

    $ChromeSrc = Join-Path $SourceExtensions "chrome"
    if (Test-Path $ChromeSrc) {
        $ChromeDst = Join-Path $ExtensionsDir "chrome"
        if (Test-Path $ChromeDst) { Remove-Item -Recurse -Force $ChromeDst }
        New-Item -ItemType Directory -Force -Path $ExtensionsDir | Out-Null
        Copy-Item -Recurse $ChromeSrc $ChromeDst
        Write-Host "[OK] Installed Chrome extension to $ChromeDst" -ForegroundColor Green
    }
} else {
    Write-Host "[WARN] No extensions found in archive (built with --skip-extensions?)" -ForegroundColor Yellow
}

# --- Post-install instructions ---
Write-Host ""
Write-Host "========================================" -ForegroundColor Blue
Write-Host "Installation complete!" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Blue
Write-Host ""
Write-Host "Browser Extension Setup:" -ForegroundColor Blue
Write-Host ""
Write-Host "  Firefox:" -ForegroundColor Yellow
Write-Host "    1. Open about:debugging in Firefox"
Write-Host "    2. Click 'This Firefox' -> 'Load Temporary Add-on'"
Write-Host "    3. Select manifest.json from:"
Write-Host "       $ExtensionsDir\firefox\"
Write-Host ""
Write-Host "  Chrome:" -ForegroundColor Yellow
Write-Host "    1. Open chrome://extensions"
Write-Host "    2. Enable 'Developer mode' (top right)"
Write-Host "    3. Click 'Load unpacked' and select:"
Write-Host "       $ExtensionsDir\chrome\"
Write-Host ""
Write-Host "Notes:" -ForegroundColor Blue
Write-Host "  - Config file auto-generates on first run"
Write-Host "  - Run 'wyncast' to start"
