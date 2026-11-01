<div align="center">

# Warper

<img src="assets/warper-icon.png" alt="Warper icon" width="140" />

A simple GUI tool for cycling Cloudflare WARP exit IPs on Windows.

</div>

## Features

- Drives `warp-cli` only; the WARP window never needs to run.
- Connect and disconnect with one press.
- Quick reset reconnects you fast when a page or call stalls.
- Full reset adds a clear-out step first, for stubborn cases.
- Auto reset via timers for automatic IP cycling.
- Exit IP readout plus a list of your last 10 exit IPs.
- Connect and standby modes for connection and disconnection.
- Start with Windows, open in the tray or in a window, desktop notifications for each result.
- Missing-warp-cli banner with a Recheck button and a download link.
- Checks the latest GitHub release at launch and offers the release page when a newer version exists.
- Repairs its own logon autostart entry, including the case where Windows is told to run Warper as administrator.
- `cleanup-warper.bat` removes Warper's files, registry entries, and startup traces. Cloudflare WARP itself is left untouched.

## Use cases

- A site flags your IP and you want a fresh one without setting up a VPN or touching a terminal.
- Calls or streams stall and a quick reset gets you moving again.
- Keep light browsing on DNS mode and switch to full traffic mode for the rest of the day.
- Leave auto reset on during travel or on shared networks so your exit IP rotates on a schedule.

## Use

Press Connect to go online. Standby switches to the configured standby mode, or disconnects when that mode is set to Disconnected. Quick reset runs the short cycle, Full reset clears the old registration first. Auto reset can cycle either way on a timer.

## Installation and setup

- Download the latest release from the [releases page](https://github.com/x01Jin/warper/releases).
- Warper needs `warp-cli`, which Cloudflare installs with the Cloudflare WARP package. Windows has no separate warp-cli installer. Get the package from [Cloudflare](https://one.one.one.one/) if `warp-cli` is missing.
- `warp-cli` talks to the `warp-svc` service, so the WARP window does not need to be open or signed in. Warper never starts it.
- Run `warper.exe`. If `warp-cli` is missing, a banner offers a download link and Recheck.
- Connect and standby modes mirror the WARP client: traffic and DNS over UDP, TLS, or HTTPS, DNS only over HTTPS or TLS, local proxy, traffic only, and disconnected.
- If the WARP window still opens on login or during a reset, that is the WARP client's own startup entry. Disable it in Task Manager > Startup.

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

VirusTotal scan: https://www.virustotal.com/gui/file/7b40aee8f25cac91dcdb5a6484d1dac1252af7ebe95555754229b673efa58949/detection

## License

This project is licensed under the **GNU General Public License v3.0 (GPLv3)**.

Copyright (C) 2024 [x01Jin]
