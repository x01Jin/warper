<div align="center">

# Warper

<img src="assets/warper-icon.png" alt="Warper icon" width="140" />

A simple GUI tool for cycling Cloudflare WARP exit IPs on Windows.

</div>

## Features

- Connect and disconnect with one press.
- Quick reset reconnects you fast when a page or call stalls.
- Full reset adds a clear-out step first, for stubborn cases.
- Auto reset via timers for automatic IP cycling.
- Exit IP readout plus a list of your last 10 exit IPs.
- Setting up connect and standby modes of connection and disconnection.
- Start with Windows, open in the tray or in a window, desktop notifications for each result.
- WARP check with a banner and download link when WARP is missing.
- a script to cleanup everything warper related residue from the whole system (like genuinely everything, it's a nuke... well except for the WARP client itself, that one is left untouched).

## Use cases

- A site flags your IP and you simply just want a fresh one without touching a vpn setup or terminal.
- Calls or streams stall and a quick reset gets you moving again.
- Keep light browsing on DNS mode and switch to full traffic mode for the rest of the day.
- Leave auto reset on during travel or on shared networks so your exit IP rotates on a schedule.

## Use

Press Connect to go online, Disconnect to step back to your standby mode. You pick Quick reset for the short cycle or Full reset for the deeper one. Turn on Auto reset and pick reset cycle duration.

There's also a standby mode option to completely disconnect and turn off WARP.

## Cleanup

`cleanup-warper.bat` deletes Warper traces from the system. It must run as administrator. Cloudflare WARP itself is never touched.

What it does, in order:

1. Kills the `warper.exe` process tree.
2. Kills orphaned `warp-cli.exe --listen` listeners Warper started.
3. Sweeps any remaining `warp-cli.exe` processes, then stops if anything survived so no locked files are deleted half-way.
4. Deletes files and folders:
   - `warper-config.json` beside the script (portable settings).
   - `warper.log` beside the script (portable encrypted log).
   - `warper.exe.WebView2` beside the script (portable WebView2 data).
   - `%LOCALAPPDATA%\com.x01jin.warper` (WebView2 profile).
   - `%APPDATA%\com.x01jin.warper` (settings and logs).
   - `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Warper.lnk` (toast shortcut).
   - `%APPDATA%\Microsoft\Windows\Recent\warper*` (Explorer recent items).
   - `.warper-write-test` beside the script (write-probe residue).
   - `%LOCALAPPDATA%\CrashDumps\warper.exe.*.dmp` (crash dumps).
   - `%SystemRoot%\Prefetch\WARPER*.pf` (prefetch traces).
5. Deletes registry entries:
   - `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run\Warper` (autostart entry).
   - `HKCU\SOFTWARE\Classes\AppUserModelId\com.x01jin.warper` (toast registration).
   - `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run\Warper` (Task Manager approval entry).
   - UserAssist values matching Warper (Explorer run history, matched as ROT13 `jnicre`).
   - MuiCache values whose path contains Warper (friendly-name cache).
6. Prints `[ok]`, `[..]` (already missing), or `[!!]` (locked) per item.

Left alone: Amcache/Shimcache, the notification platform DB, Jump-List hash blobs, Chromium `%TEMP%` scoped dirs, and icon/thumbnail caches. Those are OS-owned or unsafe to delete per-app, so Windows keeps managing them.

Virus Total scan: https://www.virustotal.com/gui/file/7b40aee8f25cac91dcdb5a6484d1dac1252af7ebe95555754229b673efa58949/detection

## Installation and setup

- Download the latest release from the [releases page](https://github.com/x01Jin/warper/releases).
- Install the Cloudflare WARP Windows client from [Cloudflare](https://one.one.one.one/) if you haven't already.
- Run `warper.exe` and it will check for WARP and prompt you to install it if missing.
- If you notice when you're doing a reset the cloudflare one client window apears, it happens when doing a reset.

For the auto resetting problem where the cloudflare one client appears completely disrupting you, you can follow this guide:

- Exit cloudflare one client in tray and it will notify you that all of it's processes will be killed, click okay
- Now you will notice in warper that you are disconnected and connecting doesnt work, that's okay just do a full reset
- After that the tool will now function normally and the cloudflare one client will not appear again anymore.

From this point on is upto your preference if you want to keep the cloudflare one client running in the background or not, You can just leave it as is right now and it will not appear again until you restart your pc, but if you want to completely disable it from starting up again you can:

- Disable cloudflare one client startup in task manager to prevent it from starting up again.
- I recommend to sometimes open cloudflare one client to check for updates though...

In warper you can customize the connection modes like in cloudflare one client alongside with it's own features.

## License

This project is licensed under the **GNU General Public License v3.0 (GPLv3)**. 

Copyright (C) 2024 [x01Jin]
