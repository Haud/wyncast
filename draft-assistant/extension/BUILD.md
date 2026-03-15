# Building the Browser Extension

The Wyndham Draft Sync extension supports both **Firefox** (Manifest v2) and **Chrome** (Manifest v3).

## Prerequisites

- **Python 3.6+**
- **Node.js / npm**
- **web-ext** (Firefox only) — install globally:
  ```
  npm install -g web-ext
  ```
- **AMO (addons.mozilla.org) account** with API key access (Firefox only)

## Build Commands

From the `draft-assistant/` directory:

```sh
python extension/build.py firefox    # Build + sign Firefox extension
python extension/build.py chrome     # Build Chrome extension (unpacked)
python extension/build.py all        # Build both
```

Or if your system requires it:

```sh
python3 extension/build.py firefox
```

## Firefox

### Development (temporary add-on)

For local development, you can load the extension directly from the `extension/` root
without any build step:

1. Open Firefox and navigate to `about:debugging`
2. Click **This Firefox** in the left sidebar
3. Click **Load Temporary Add-on...**
4. Select `extension/manifest.json`

The extension will remain loaded until Firefox is restarted. To reload after making
changes, click the **Reload** button next to the extension in `about:debugging`.

### AMO API Credentials

Get your API keys at: https://addons.mozilla.org/developers/addon/api/key/

Provide credentials using **one** of these methods:

#### Option 1: Environment Variables (recommended for CI)

```sh
export AMO_JWT_ISSUER='your-api-key'
export AMO_JWT_SECRET='your-api-secret'
```

On Windows (PowerShell):
```powershell
$env:AMO_JWT_ISSUER = 'your-api-key'
$env:AMO_JWT_SECRET = 'your-api-secret'
```

#### Option 2: `.env` File (recommended for local development)

Create `draft-assistant/.env`:

```
AMO_JWT_ISSUER=your-api-key
AMO_JWT_SECRET=your-api-secret
```

This file is git-ignored.

#### Option 3: Credentials File (legacy)

Create `extension/.amo-credentials` with two lines:

```
your-api-key
your-api-secret
```

This file is git-ignored.

### Syncing from 1Password

If your AMO credentials are stored in 1Password, you can auto-populate the `.env` file:

```sh
python3 extension/sync_env.py
```

This requires the [1Password CLI](https://developer.1password.com/docs/cli/) (`brew install 1password-cli`).

### Build and Sign

```sh
python extension/build.py firefox
```

The script will:
1. Run `web-ext` directly against the extension root (excluding non-extension files)
2. Build the extension into a `.zip`
3. Submit it to AMO for signing (unlisted channel)
4. Download the signed `.xpi` into `dist/firefox/web-ext-artifacts/`

### Installing in Firefox

1. Open Firefox and navigate to `about:addons`
2. Click the gear icon and select **Install Add-on From File...**
3. Select the `.xpi` file printed by the build script

## Chrome

### Build

```sh
python extension/build.py chrome
```

The script will assemble shared + Chrome-specific files into `dist/chrome/`.

### Installing in Chrome

1. Open Chrome and navigate to `chrome://extensions`
2. Enable **Developer mode** (toggle in top-right)
3. Click **Load unpacked**
4. Select the `dist/chrome/` directory
