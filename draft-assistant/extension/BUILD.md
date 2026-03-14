# Building the Firefox Extension

## Prerequisites

- **Python 3.6+**
- **Node.js / npm**
- **web-ext** — install globally:
  ```
  npm install -g web-ext
  ```
- **AMO (addons.mozilla.org) account** with API key access

## AMO API Credentials

Get your API keys at: https://addons.mozilla.org/developers/addon/api/key/

Provide credentials using **one** of these methods:

### Option 1: Environment Variables (recommended for CI)

```sh
export AMO_JWT_ISSUER='your-api-key'
export AMO_JWT_SECRET='your-api-secret'
```

On Windows (PowerShell):
```powershell
$env:AMO_JWT_ISSUER = 'your-api-key'
$env:AMO_JWT_SECRET = 'your-api-secret'
```

### Option 2: `.env` File (recommended for local development)

Create `draft-assistant/.env`:

```
AMO_JWT_ISSUER=your-api-key
AMO_JWT_SECRET=your-api-secret
```

This file is git-ignored.

### Option 3: Credentials File (legacy)

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

## Build and Sign

From the `draft-assistant/` directory:

```sh
python extension/build.py
```

Or if your system requires it:

```sh
python3 extension/build.py
```

The script will:
1. Build the extension into a `.zip`
2. Submit it to AMO for signing (unlisted channel)
3. Download the signed `.xpi` and print its path

## Installing in Firefox

1. Open Firefox and navigate to `about:addons`
2. Click the gear icon and select **Install Add-on From File...**
3. Select the `.xpi` file printed by the build script
