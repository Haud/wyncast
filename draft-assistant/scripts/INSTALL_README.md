# Wyncast Draft Assistant

## Quick Start

**macOS / Linux:**

```
./install.sh
```

**Windows:**

Right-click `install.ps1` and select "Run with PowerShell", or from a terminal:

```
powershell -ExecutionPolicy Bypass -File install.ps1
```

## What Gets Installed

### Binary (`wyncast`)

| Platform      | Location                                       |
|---------------|-------------------------------------------------|
| macOS / Linux | `~/.local/bin/wyncast`                          |
| Windows       | `%LOCALAPPDATA%\Programs\wyncast\wyncast.exe`   |

The installer adds the binary location to your PATH automatically.

### Projection Data

| Platform | Location                                            |
|----------|-----------------------------------------------------|
| macOS    | `~/Library/Application Support/wyncast/data/projections/` |
| Linux    | `~/.local/share/wyncast/data/projections/`          |
| Windows  | `%APPDATA%\wyncast\data\projections\`               |

### Browser Extensions

Installed to the same app data directory under `extensions/`.

## Browser Extension Setup

### Firefox

1. Open `about:debugging` in Firefox
2. Click **This Firefox** → **Load Temporary Add-on**
3. Navigate to the `extensions/firefox/` folder in the app data directory and select `manifest.json`

Note: temporary add-ons must be reloaded each time Firefox restarts.

### Chrome

1. Open `chrome://extensions`
2. Enable **Developer mode** (toggle in top right)
3. Click **Load unpacked**
4. Select the `extensions/chrome/` folder in the app data directory

## Configuration

A config file auto-generates on first run at `{app_data_dir}/config/`. You'll need to configure your Claude API key on first launch — the app will guide you through onboarding.

## Running

Run `wyncast` from your terminal after installation. The app opens a terminal UI dashboard for live draft assistance. Make sure the browser extension is loaded before starting a draft.

## Uninstalling

1. **Delete the binary** from its install location (see table above)
2. **Delete the app data directory:**
   - macOS: `~/Library/Application Support/wyncast/`
   - Linux: `~/.local/share/wyncast/`
   - Windows: `%APPDATA%\wyncast\`
3. **Remove the PATH entry:**
   - macOS / Linux: remove the line added to your shell config (`~/.bashrc`, `~/.zshrc`, etc.)
   - Windows: remove `%LOCALAPPDATA%\Programs\wyncast` from your user PATH environment variable
