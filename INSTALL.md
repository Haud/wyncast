# Wyncast Draft Assistant — Installation Guide

Welcome! This guide will walk you through installing the Wyncast Draft Assistant on your **Windows** or **Mac** computer. No programming experience required.

---

## What You're Installing

Wyncast is a fantasy baseball draft assistant that runs in your terminal (the black-and-white text window on your computer). It connects to your ESPN draft through a browser extension and gives you real-time player valuations and recommendations.

There are two pieces to install:

1. **The Wyncast app** — runs in your terminal
2. **A browser extension** — connects ESPN's draft page to the app

---

## Step 1: Download Wyncast

Download the correct file for your computer:

| Your Computer | File to Download |
|---|---|
| **Windows** | `draft-assistant-X.X.X-x86_64-pc-windows-msvc.zip` |
| **Mac (Apple Silicon — M1/M2/M3/M4)** | `draft-assistant-X.X.X-aarch64-apple-darwin.tar.gz` |
| **Mac (older Intel Mac)** | `draft-assistant-X.X.X-x86_64-apple-darwin.tar.gz` |

> **Not sure which Mac you have?** Click the Apple menu () in the top-left corner of your screen, then click **About This Mac**. If it says "Apple M1" (or M2, M3, M4), you have Apple Silicon. If it says "Intel", you have an Intel Mac.

---

## Step 2: Extract the Download

### On Mac

1. Open **Finder** and go to your **Downloads** folder
2. Double-click the `.tar.gz` file — it will automatically extract into a folder

### On Windows

1. Open **File Explorer** and go to your **Downloads** folder
2. Right-click the `.zip` file
3. Click **Extract All...**
4. Click **Extract**

You should now see a folder named something like `draft-assistant-0.1.0-...`. Open it.

---

## Step 3: Run the Installer

### On Mac

1. Open the **Terminal** app (search for "Terminal" in Spotlight — press `Cmd + Space` and type `Terminal`)
2. Type `cd ` (with a space after it), then **drag the extracted folder** from Finder into the Terminal window. This fills in the folder path for you. Press **Enter**.
3. Type the following command and press **Enter**:
   ```
   bash install.sh
   ```
4. If macOS shows a security warning saying the app "can't be opened because it is from an unidentified developer":
   - Open **System Settings** → **Privacy & Security**
   - Scroll down and you'll see a message about "wyncast" being blocked
   - Click **Open Anyway**
5. **Close and reopen Terminal** for the changes to take effect

### On Windows

1. Open the extracted folder in **File Explorer**
2. Right-click on **install.ps1**
3. Click **Run with PowerShell**
4. If Windows asks "Do you want to allow this app to make changes?", click **Yes**
5. If you see a red error about "execution policy":
   - Open **PowerShell** by searching for it in the Start menu
   - **Right-click** on PowerShell and choose **Run as administrator**
   - Type the following and press **Enter**:
     ```
     Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned
     ```
   - Type **Y** and press **Enter** to confirm
   - Now drag the `install.ps1` file into the PowerShell window and press **Enter**
6. **Close and reopen PowerShell/Command Prompt** for the changes to take effect

---

## Step 4: Set Up the Browser Extension

The browser extension watches your ESPN draft page and sends live data to the app. You need to load it into your browser.

### For Chrome

1. Open Chrome and type `chrome://extensions` in the address bar, then press **Enter**
2. Turn on **Developer mode** using the toggle in the top-right corner
3. Click the **Load unpacked** button (top-left area)
4. Navigate to the extensions folder that the installer told you about:
   - **Mac:** `~/Library/Application Support/wyncast/extensions/chrome/`
   - **Windows:** `%APPDATA%\wyncast\data\extensions\chrome\`
5. Select the `chrome` folder and click **Open** (Mac) or **Select Folder** (Windows)
6. You should see "Wyndham Draft Sync" appear in your extensions list

### For Firefox

1. Open Firefox and type `about:debugging` in the address bar, then press **Enter**
2. Click **This Firefox** in the left sidebar
3. Click the **Load Temporary Add-on...** button
4. Navigate to the extensions folder:
   - **Mac:** `~/Library/Application Support/wyncast/extensions/firefox/`
   - **Windows:** `%APPDATA%\wyncast\data\extensions\firefox\`
5. Select the **manifest.json** file inside that folder and click **Open**

> **Note:** Firefox temporary extensions are removed when you close Firefox. You'll need to reload the extension each time you restart Firefox.

---

## Step 5: Run Wyncast

1. Open a **Terminal** (Mac) or **Command Prompt / PowerShell** (Windows)
2. Type the following and press **Enter**:
   ```
   wyncast
   ```
3. The first time you run it, the app will automatically create its configuration files. You'll see the dashboard appear in your terminal.

---

## Step 6: Add Your Anthropic API Key (Optional)

If you want AI-powered draft analysis (recommendations from Claude during the draft), you'll need an Anthropic API key.

1. Go to [console.anthropic.com](https://console.anthropic.com/) and create an account
2. Generate an API key
3. When you first launch Wyncast, the onboarding wizard will ask for your key — just paste it in

If you skip this step, the app still works — you just won't get the AI analysis features.

---

## Draft Day Quick Start

1. Open your terminal and run `wyncast`
2. Open your browser and navigate to your ESPN draft page
3. Make sure the browser extension is loaded (see Step 4 above)
4. The extension automatically connects to the app — you'll see live draft data appear in the terminal

### Keyboard Controls

| Key | What It Does |
|---|---|
| `1` through `5` | Switch between tabs (Analysis, Nomination Plan, Players, Draft Log, Teams) |
| `j` or `Down arrow` | Scroll down |
| `k` or `Up arrow` | Scroll up |
| `/` | Search / filter players |
| `p` | Filter by position (cycle through C, 1B, 2B, etc.) |
| `r` | Refresh AI analysis |
| `q` | Quit the app |

---

## Troubleshooting

### "wyncast is not recognized" or "command not found"
- Make sure you **closed and reopened** your terminal after running the installer
- On Mac, try running: `source ~/.zshrc`

### The browser extension won't connect
- Make sure Wyncast is **running in your terminal first**, then load the extension
- Check that nothing else is using port 9001 on your computer

### Mac says the app "can't be opened"
- Go to **System Settings** → **Privacy & Security** and click **Open Anyway** next to the wyncast message

### Windows Defender blocks the app
- Click **More info** → **Run anyway** on the SmartScreen popup

### AI analysis isn't showing up
- Make sure you entered your Anthropic API key (see Step 6)
- Check that you have credit/balance on your Anthropic account

---

## Uninstalling

### On Mac
```
rm ~/.local/bin/wyncast
rm -rf ~/Library/Application\ Support/wyncast
```

### On Windows
1. Delete the folder: `%LOCALAPPDATA%\Programs\wyncast`
2. Delete the folder: `%APPDATA%\wyncast`
3. Open **Settings** → **System** → **About** → **Advanced system settings** → **Environment Variables**, find `Path` under User variables, and remove the wyncast entry
