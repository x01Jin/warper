import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Store } from "@tauri-apps/plugin-store";

interface WarpStatus {
  connected: boolean;
  status: string;
  reason: string;
}

interface WarpIdentity {
  accountType: string;
  deviceId: string;
}

interface WarpInfo {
  installed: boolean;
  path?: string;
  version?: string;
}

interface Progress {
  step: string;
}

interface IpEntry {
  ip: string;
  at: number;
}

interface SettingsLocation {
  path: string;
  portable: boolean;
  imported: boolean;
}

interface Settings {
  autoReset: boolean;
  intervalMinutes: number;
  startMinimized: boolean;
  notifications: boolean;
  ipHistory: IpEntry[];
  connectMode: string;
  standbyMode: string;
}

const IP_TIMEOUT_MS = 8_000;
const IP_BACKSTOP_MS = 60_000;
const STATUS_POLL_MS = 5_000;
const SETTLE_RETRIES = 12;
const SETTLE_PAUSE_MS = 2_000;
const MAX_LOG_LINES = 200;
const MAX_IP_HISTORY = 10;
const VALID_INTERVALS = [5, 15, 30, 60];
const VALID_CONNECT_MODES = ["warp", "doh", "warp+doh", "dot", "warp+dot", "proxy", "tunnel_only"];
const VALID_STANDBY_MODES = [...VALID_CONNECT_MODES, "off"];
const MODE_LABELS: Record<string, string> = {
  warp: "Traffic and DNS (UDP)",
  doh: "DNS only (HTTPS)",
  dot: "DNS only (TLS)",
  "warp+doh": "Traffic and DNS (HTTPS)",
  "warp+dot": "Traffic and DNS (TLS)",
  proxy: "Local proxy",
  tunnel_only: "Traffic only",
};
const WARP_DOWNLOAD_PAGE = "https://developers.cloudflare.com/warp-client/get-started/windows/";

const DEFAULTS: Settings = {
  autoReset: false,
  intervalMinutes: 15,
  startMinimized: true,
  notifications: true,
  ipHistory: [],
  connectMode: "warp",
  standbyMode: "doh",
};

const el = {
  dot: document.querySelector("#status-dot") as HTMLElement,
  statusText: document.querySelector("#status-text") as HTMLElement,
  device: document.querySelector("#device-id") as HTMLElement,
  exitIp: document.querySelector("#exit-ip") as HTMLElement,
  warpVersion: document.querySelector("#warp-version") as HTMLElement,
  log: document.querySelector("#log") as HTMLElement,
  buttons: [...document.querySelectorAll<HTMLButtonElement>(".actions button")],
  ops: [
    ...document.querySelectorAll<HTMLButtonElement>(
      "#btn-quick, #btn-full, #btn-connect, #btn-standby",
    ),
  ],
  autoToggle: document.querySelector("#auto-toggle") as HTMLInputElement,
  segment: [...document.querySelectorAll<HTMLButtonElement>(".segment button[data-minutes]")],
  countdown: document.querySelector("#auto-countdown") as HTMLElement,
  banner: document.querySelector("#warp-banner") as HTMLElement,
  bannerDetail: document.querySelector("#banner-detail") as HTMLElement,
  autostartToggle: document.querySelector("#autostart-toggle") as HTMLInputElement,
  notifToggle: document.querySelector("#notif-toggle") as HTMLInputElement,
  startSegment: [...document.querySelectorAll<HTMLButtonElement>(".segment button[data-start]")],
  ipHistory: document.querySelector("#ip-history") as HTMLUListElement,
  modeValue: document.querySelector("#mode-value") as HTMLElement,
  connectMode: document.querySelector("#connect-mode") as HTMLSelectElement,
  standbyMode: document.querySelector("#standby-mode") as HTMLSelectElement,
};

let busy = false;
let warpMissing = false;
let lastAutoRun = Date.now();
let settings: Settings = { ...DEFAULTS };
let store: Store | null = null;
let storePath: string | null = null;
let lastHealth: string | null = null;
let modeAdopted = false;
let deviceFetching = false;
let lastDeviceError: string | null = null;

function logHealth(reason: string): void {
  if (reason === lastHealth) return;
  lastHealth = reason;
  logLine(`Health: ${reason || "—"}`);
}

function logLine(message: string): void {
  const time = new Date().toLocaleTimeString();
  const lines = el.log.textContent ? el.log.textContent.split("\n") : [];
  lines.push(`[${time}] ${message}`);
  el.log.textContent = lines.slice(-MAX_LOG_LINES).join("\n");
  el.log.scrollTop = el.log.scrollHeight;
}

async function notify(title: string, body: string): Promise<void> {
  if (!settings.notifications) return;
  try {
    if (!(await isPermissionGranted())) {
      if ((await requestPermission()) !== "granted") return;
    }
    sendNotification({ title, body });
  } catch {
    // Best-effort; failures already surface in the log.
  }
}

function updateButtons(): void {
  for (const button of el.ops) {
    button.disabled = busy || warpMissing;
  }
  for (const button of el.buttons) {
    if (!el.ops.includes(button)) {
      button.disabled = busy;
    }
  }
  el.autoToggle.disabled = busy;
}

function setBusy(value: boolean): void {
  busy = value;
  updateButtons();
}

function paintStatus(status: WarpStatus): void {
  el.dot.className = `dot pulse ${status.connected ? "on" : "off"}`;
  el.statusText.textContent = status.connected ? "Connected" : status.status;
}

function paintMissing(): void {
  el.dot.className = "dot off";
  el.statusText.textContent = "WARP missing";
  el.device.textContent = "—";
  el.warpVersion.textContent = "missing";
  el.bannerDetail.textContent = "Install Cloudflare WARP to use Warper.";
  el.banner.classList.remove("hidden");
  updateButtons();
}

async function persist(): Promise<void> {
  if (!store) return;
  try {
    await ensureStore();
    if (!store) return;
    await store.set("autoReset", settings.autoReset);
    await store.set("intervalMinutes", settings.intervalMinutes);
    await store.set("startMinimized", settings.startMinimized);
    await store.set("notifications", settings.notifications);
    await store.set("ipHistory", settings.ipHistory);
    await store.set("connectMode", settings.connectMode);
    await store.set("standbyMode", settings.standbyMode);
    await store.save();
  } catch (error) {
    logLine(`settings save failed: ${error}`);
  }
}

/// Reload the store when the exe folder moved since boot (portable rename).
/// The plugin binds a store handle to its absolute load path, so saving
/// through a stale handle would recreate the old folder tree.
async function ensureStore(): Promise<void> {
  if (!store || !storePath) return;
  const loc = await invoke<SettingsLocation>("get_settings_path");
  if (loc.path === storePath) return;
  store = await Store.load(loc.path);
  storePath = loc.path;
  logLine(`settings location moved — now saving to ${loc.path}`);
}

async function loadSettings(): Promise<void> {
  const loc = await invoke<SettingsLocation>("get_settings_path");
  store = await Store.load(loc.path);
  storePath = loc.path;
  if (loc.portable) {
    logLine(`settings: ${loc.path}`);
  } else {
    logLine(`settings (exe folder is read-only): ${loc.path}`);
  }
  if (loc.imported) {
    logLine("imported previous settings from app data");
  }
  const auto = await store.get<boolean>("autoReset");
  const mins = await store.get<number>("intervalMinutes");
  const minimized = await store.get<boolean>("startMinimized");
  const notifications = await store.get<boolean>("notifications");
  const history = await store.get<IpEntry[]>("ipHistory");
  const connectMode = await store.get<string>("connectMode");
  const standbyMode = await store.get<string>("standbyMode");
  const hasKeys = await store.has("autoReset");
  if (typeof auto === "boolean") settings.autoReset = auto;
  if (typeof mins === "number" && VALID_INTERVALS.includes(mins)) {
    settings.intervalMinutes = mins;
  }
  if (typeof minimized === "boolean") settings.startMinimized = minimized;
  if (typeof notifications === "boolean") settings.notifications = notifications;
  if (Array.isArray(history)) settings.ipHistory = history.slice(0, MAX_IP_HISTORY);
  if (typeof connectMode === "string" && VALID_CONNECT_MODES.includes(connectMode)) {
    settings.connectMode = connectMode;
  }
  if (typeof standbyMode === "string" && VALID_STANDBY_MODES.includes(standbyMode)) {
    settings.standbyMode = standbyMode;
  }
  // Migrate legacy localStorage settings once.
  if (!hasKeys && localStorage.getItem("warper.migrated") !== "1") {
    settings.autoReset = localStorage.getItem("warper.auto") === "1";
    const legacy = Number(localStorage.getItem("warper.interval") ?? "15");
    if (VALID_INTERVALS.includes(legacy)) settings.intervalMinutes = legacy;
    await persist();
    localStorage.setItem("warper.migrated", "1");
  }
}

async function fetchExitIp(): Promise<string> {
  const sources = ["https://api.cloudflare.com/cdn-cgi/trace", "https://ifconfig.me/ip"];
  for (const url of sources) {
    try {
      const response = await fetch(url, {
        signal: AbortSignal.timeout(IP_TIMEOUT_MS),
      });
      if (!response.ok) continue;
      const text = (await response.text()).trim();
      const ipMatch = text.match(/ip=([^\s]+)/);
      const ip = ipMatch ? ipMatch[1] : text.split("\n")[0].trim();
      if (ip) return ip;
    } catch {
      continue;
    }
  }
  return "unreachable";
}

function buildModeOptions(): void {
  el.connectMode.innerHTML = "";
  for (const value of VALID_CONNECT_MODES) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = MODE_LABELS[value] ?? value;
    el.connectMode.appendChild(option);
  }
  el.standbyMode.innerHTML = "";
  for (const value of VALID_CONNECT_MODES) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = MODE_LABELS[value] ?? value;
    el.standbyMode.appendChild(option);
  }
  const off = document.createElement("option");
  off.value = "off";
  off.textContent = "Disconnected";
  el.standbyMode.appendChild(off);
}

function paintIpHistory(): void {
  el.ipHistory.innerHTML = "";
  for (const entry of settings.ipHistory) {
    const li = document.createElement("li");
    li.textContent = `${entry.ip} — ${new Date(entry.at).toLocaleTimeString()}`;
    el.ipHistory.appendChild(li);
  }
}

async function recordIp(ip: string): Promise<void> {
  if (!ip || ip === "unreachable") return;
  const old = settings.ipHistory[0]?.ip;
  if (old === ip) return;
  settings.ipHistory = [{ ip, at: Date.now() }, ...settings.ipHistory].slice(0, MAX_IP_HISTORY);
  paintIpHistory();
  await persist();
  if (old) {
    await notify("Warper", `Exit IP changed: ${old} → ${ip}`);
  }
}

async function refreshIp(): Promise<void> {
  if (busy || warpMissing) return;
  const ip = await fetchExitIp();
  el.exitIp.textContent = ip;
  await recordIp(ip);
}

async function checkWarp(): Promise<boolean> {
  try {
    const info = await invoke<WarpInfo>("get_warp_info");
    warpMissing = !info.installed;
    el.banner.classList.toggle("hidden", info.installed);
    el.warpVersion.textContent = info.installed ? (info.version ?? "installed") : "missing";
    if (!info.installed) {
      paintMissing();
    } else {
      updateButtons();
    }
    return info.installed;
  } catch (error) {
    logLine(`warp check failed: ${error}`);
    return !warpMissing;
  }
}

function paintDevice(identity: WarpIdentity | null): void {
  el.device.textContent =
    identity && identity.deviceId
      ? `${identity.deviceId.slice(0, 8)}… (${identity.accountType || "?"})`
      : "—";
}

/// Re-fetch `registration show` only while Device is still blank.
async function refreshDeviceIfBlank(): Promise<void> {
  if (busy || warpMissing || deviceFetching) return;
  if (el.device.textContent !== "—") return;
  deviceFetching = true;
  try {
    const identity = await invoke<WarpIdentity>("get_identity");
    paintDevice(identity);
    if (el.device.textContent !== "—") {
      lastDeviceError = null;
    } else if (lastDeviceError !== "empty") {
      lastDeviceError = "empty";
      logLine("device lookup returned empty — will retry while blank");
    }
  } catch (error) {
    const message = String(error);
    if (message !== lastDeviceError) {
      lastDeviceError = message;
      logLine(`device lookup failed: ${message}`);
    }
  } finally {
    deviceFetching = false;
  }
}

async function refresh(): Promise<void> {
  if (busy) return;
  if (!(await checkWarp())) return;
  try {
    const [status, identity, ip, mode] = await Promise.all([
      invoke<WarpStatus>("get_status"),
      invoke<WarpIdentity>("get_identity").catch((error: unknown) => {
        logLine(`device lookup failed: ${error}`);
        return null;
      }),
      fetchExitIp(),
      invoke<string>("get_mode").catch(() => null),
    ]);
    paintStatus(status);
    logHealth(status.reason);
    paintDevice(identity);
    el.exitIp.textContent = ip;
    el.modeValue.textContent = mode ? (MODE_LABELS[mode] ?? mode) : "—";
    if (!modeAdopted) {
      modeAdopted = true;
      if (mode === "doh" || mode === "dot") {
        settings.standbyMode = mode;
        el.standbyMode.value = mode;
        logLine(`standby mode adopted from WARP: ${mode}`);
        await persist();
      }
    }
    await recordIp(ip);
  } catch (error) {
    el.dot.className = "dot off";
    el.statusText.textContent = "Error";
    logLine(`refresh failed: ${error}`);
  }
}

async function refreshStatus(): Promise<boolean> {
  if (busy || warpMissing) return false;
  try {
    const status = await invoke<WarpStatus>("get_status");
    paintStatus(status);
    logHealth(status.reason);
    if (status.connected) {
      await refreshDeviceIfBlank();
    }
    return status.connected;
  } catch (error) {
    logLine(`status poll failed: ${error}`);
    return false;
  }
}

async function settleStatus(): Promise<void> {
  for (let attempt = 0; attempt < SETTLE_RETRIES; attempt++) {
    if (await refreshStatus()) return;
    await new Promise((resolve) => setTimeout(resolve, SETTLE_PAUSE_MS));
  }
  await refreshStatus();
}

async function runOp(
  label: string,
  command: string,
  args?: Record<string, unknown>,
): Promise<void> {
  if (busy) {
    logLine(`${label}: already running, ignored`);
    return;
  }
  setBusy(true);
  logLine(`${label} started`);
  try {
    const result = await invoke<string>(command, args);
    logLine(result);
  } catch (error) {
    logLine(`${label} failed: ${error}`);
  } finally {
    setBusy(false);
    lastAutoRun = Date.now();
    await settleStatus();
    await refresh();
  }
}

function paintSegment(): void {
  const current = String(settings.intervalMinutes);
  for (const button of el.segment) {
    button.classList.toggle("active", button.dataset.minutes === current);
  }
}

function paintStartSegment(): void {
  const current = settings.startMinimized ? "tray" : "window";
  for (const button of el.startSegment) {
    button.classList.toggle("active", button.dataset.start === current);
  }
}

function tickAuto(): void {
  if (!settings.autoReset || busy || warpMissing) return;
  const remaining = settings.intervalMinutes * 60_000 - (Date.now() - lastAutoRun);
  if (remaining <= 0) {
    el.countdown.textContent = "";
    void runOp("auto quick-reset", "quick_reset", {
      connectMode: settings.connectMode,
    });
  } else {
    const minutes = Math.floor(remaining / 60_000);
    const seconds = Math.floor((remaining % 60_000) / 1000);
    el.countdown.textContent = `${minutes}:${String(seconds).padStart(2, "0")}`;
  }
}

window.addEventListener("DOMContentLoaded", () => {
  void (async () => {
    await loadSettings();
    buildModeOptions();
    el.autoToggle.checked = settings.autoReset;
    el.notifToggle.checked = settings.notifications;
    el.connectMode.value = settings.connectMode;
    el.standbyMode.value = settings.standbyMode;
    paintSegment();
    paintStartSegment();
    paintIpHistory();
    try {
      el.autostartToggle.checked = await invoke<boolean>("get_autostart");
    } catch (error) {
      logLine(`autostart check failed: ${error}`);
    }
    document.querySelector("#btn-connect")?.addEventListener(
      "click",
      () =>
        void runOp("connect", "warp_connect", {
          connectMode: settings.connectMode,
        }),
    );
    document.querySelector("#btn-standby")?.addEventListener(
      "click",
      () =>
        void runOp("standby", "warp_standby", {
          standby: settings.standbyMode,
        }),
    );
    document.querySelector("#btn-quick")?.addEventListener(
      "click",
      () =>
        void runOp("quick reset", "quick_reset", {
          connectMode: settings.connectMode,
        }),
    );
    document.querySelector("#btn-full")?.addEventListener(
      "click",
      () =>
        void runOp("full reset", "full_reset", {
          connectMode: settings.connectMode,
        }),
    );
    document.querySelector("#btn-clear")?.addEventListener("click", () => {
      el.log.textContent = "";
    });
    document.querySelector("#btn-download-page")?.addEventListener("click", () => {
      openUrl(WARP_DOWNLOAD_PAGE).catch((error: unknown) => {
        logLine(`cannot open download page: ${error}`);
      });
    });

    el.autoToggle.addEventListener("change", () => {
      settings.autoReset = el.autoToggle.checked;
      lastAutoRun = Date.now();
      el.countdown.textContent = "";
      logLine(`auto reset ${settings.autoReset ? "enabled" : "disabled"}`);
      void persist();
    });
    for (const button of el.segment) {
      button.addEventListener("click", () => {
        const mins = Number(button.dataset.minutes ?? "15");
        if (VALID_INTERVALS.includes(mins)) {
          settings.intervalMinutes = mins;
          lastAutoRun = Date.now();
          paintSegment();
          void persist();
        }
      });
    }
    el.autostartToggle.addEventListener("change", () => {
      void (async () => {
        try {
          const result = await invoke<string>("set_autostart", {
            enabled: el.autostartToggle.checked,
          });
          logLine(result);
        } catch (error) {
          el.autostartToggle.checked = !el.autostartToggle.checked;
          logLine(`autostart failed: ${error}`);
        }
      })();
    });
    for (const button of el.startSegment) {
      button.addEventListener("click", () => {
        settings.startMinimized = button.dataset.start !== "window";
        paintStartSegment();
        logLine(
          `startup behavior: ${settings.startMinimized ? "minimize to tray" : "open window"}`,
        );
        void persist();
      });
    }
    el.notifToggle.addEventListener("change", () => {
      settings.notifications = el.notifToggle.checked;
      logLine(`notifications ${settings.notifications ? "enabled" : "disabled"}`);
      void persist();
      if (settings.notifications) {
        void (async () => {
          try {
            if (!(await isPermissionGranted())) {
              const permission = await requestPermission();
              if (permission !== "granted") {
                logLine("notifications blocked by the OS — allow Warper in Windows Settings");
              }
            }
          } catch {
            // Best-effort; the next toast attempt retries.
          }
        })();
      }
    });
    el.connectMode.addEventListener("change", () => {
      settings.connectMode = el.connectMode.value;
      logLine(`connect mode: ${settings.connectMode}`);
      void persist();
    });
    el.standbyMode.addEventListener("change", () => {
      settings.standbyMode = el.standbyMode.value;
      logLine(`standby mode: ${settings.standbyMode}`);
      void persist();
    });
    await listen<Progress>("warp-progress", (event) => {
      logLine(event.payload.step);
    });
    await listen<WarpStatus>("warp-status", (event) => {
      if (!busy) {
        paintStatus(event.payload);
        logHealth(event.payload.reason);
        if (event.payload.connected) {
          void refreshDeviceIfBlank();
        }
      }
    });
    await listen<string>("warp-ip", (event) => {
      void recordIp(event.payload);
    });
    await listen("warp-missing", () => {
      warpMissing = true;
      logLine("warp-cli not found — install WARP to continue");
      paintMissing();
    });

    await refresh();
    setInterval(() => void refreshIp(), IP_BACKSTOP_MS);
    setInterval(() => void refreshStatus(), STATUS_POLL_MS);
    setInterval(tickAuto, 1000);
  })();
});
