# Building & Signing the Firefox Extension

## Prerequisites

- **Node.js** and **npm**
- **web-ext** CLI tool: `npm install -g web-ext`
- **AMO (addons.mozilla.org) account** with API keys for signing

## Getting AMO API Keys

1. Log in to your Mozilla account
2. Visit <https://addons.mozilla.org/developers/addon/api/key/>
3. Generate credentials — you'll get a **JWT issuer** and **JWT secret**

## Setting Credentials

**Option A — Environment variables:**

```sh
export AMO_JWT_ISSUER='your-jwt-issuer'
export AMO_JWT_SECRET='your-jwt-secret'
```

**Option B — Credentials file:**

Create `extension/.amo-credentials` with two lines:

```
your-jwt-issuer
your-jwt-secret
```

This file is gitignored and will not be committed.

## Build & Sign

```sh
cd extension
./build.sh
```

The script will:
1. Build a `.zip` artifact via `web-ext build`
2. Sign it with `web-ext sign --channel=unlisted`
3. Print the path to the resulting `.xpi` file

## Installing the Signed Extension

1. Open Firefox and navigate to `about:addons`
2. Click the gear icon and select **Install Add-on From File...**
3. Select the `.xpi` file from `extension/web-ext-artifacts/`
