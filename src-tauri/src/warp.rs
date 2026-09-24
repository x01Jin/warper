use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

const WARP_TIMEOUT: Duration = Duration::from_secs(15);
const HIGH_INTEGRITY_SID: &str = "S-1-16-12288";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WarpStatus {
    pub connected: bool,
    pub status: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WarpIdentity {
    pub account_type: String,
    pub device_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WarpInfo {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

pub fn warp_cli_path() -> PathBuf {
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(program_files)
        .join("Cloudflare")
        .join("Cloudflare WARP")
        .join("warp-cli.exe");
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from("warp-cli")
    }
}

pub async fn run_warp(args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        WARP_TIMEOUT,
        Command::new(warp_cli_path())
            .creation_flags(CREATE_NO_WINDOW)
            .args(args)
            .output(),
    )
    .await
    .map_err(|_| "warp-cli timed out after 15s".to_string())?
    .map_err(|e| format!("failed to launch warp-cli: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("warp-cli {} failed: {}", args.join(" "), output.status)
        } else {
            stderr
        })
    }
}

/// Mutations require elevation.
pub async fn is_elevated() -> bool {
    let output = Command::new("whoami")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["/groups"])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    output.contains(HIGH_INTEGRITY_SID)
}

pub fn require_admin(elevated: bool) -> Result<(), String> {
    if elevated {
        Ok(())
    } else {
        Err("administrator rights required — restart Warper as administrator".to_string())
    }
}

pub fn parse_status(text: &str) -> Result<WarpStatus, String> {
    let value: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|e| format!("unparseable warp-cli status: {e}"))?;
    let status = value
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let reason = value
        .get("reason")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(WarpStatus {
        connected: status.eq_ignore_ascii_case("connected"),
        status,
        reason,
    })
}

pub async fn get_status() -> Result<WarpStatus, String> {
    let out = run_warp(&["--json", "status"]).await?;
    parse_status(&out)
}

fn field<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(key))
        .map(str::trim)
        .unwrap_or_default()
}

pub async fn get_identity() -> Result<WarpIdentity, String> {
    let out = run_warp(&["registration", "show"]).await?;
    Ok(WarpIdentity {
        account_type: field(&out, "Account type:").to_string(),
        device_id: field(&out, "Device ID:").to_string(),
    })
}

pub const ALLOW_MODES: [&str; 7] = [
    "warp",
    "doh",
    "warp+doh",
    "dot",
    "warp+dot",
    "proxy",
    "tunnel_only",
];

/// Map the `Mode:` value from `warp-cli settings` to a `warp-cli mode` arg.
pub fn parse_mode(text: &str) -> Result<String, String> {
    let raw = text
        .lines()
        .find_map(|line| {
            line.split("Mode:")
                .nth(1)
                .filter(|_| line.contains("Mode:"))
                .map(str::trim)
        })
        .unwrap_or_default()
        .to_lowercase();
    match raw.as_str() {
        "warp" => Ok("warp".to_string()),
        "dnsoverhttps" => Ok("doh".to_string()),
        "warpdnsoverhttps" => Ok("warp+doh".to_string()),
        "dnsovertls" => Ok("dot".to_string()),
        "warpdnsovertls" => Ok("warp+dot".to_string()),
        "proxy" => Ok("proxy".to_string()),
        "tunnelonly" => Ok("tunnel_only".to_string()),
        other => Err(format!("unrecognized WARP mode: {other}")),
    }
}

pub fn check_mode(mode: &str) -> Result<(), String> {
    if ALLOW_MODES.contains(&mode) {
        Ok(())
    } else {
        Err(format!("invalid WARP mode: {mode}"))
    }
}

pub async fn get_mode() -> Result<String, String> {
    let out = run_warp(&["settings"]).await?;
    parse_mode(&out)
}

pub async fn set_mode(mode: &str) -> Result<(), String> {
    check_mode(mode)?;
    run_warp(&["mode", mode]).await?;
    Ok(())
}

/// Never errors; missing binaries report `installed: false`.
pub async fn warp_info() -> WarpInfo {
    let path = warp_cli_path();
    match run_warp(&["--version"]).await {
        Ok(raw) => {
            let version = raw.lines().next().unwrap_or_default().trim().to_string();
            WarpInfo {
                installed: true,
                path: Some(path.to_string_lossy().into_owned()),
                version: if version.is_empty() {
                    None
                } else {
                    Some(version)
                },
            }
        }
        Err(_) => WarpInfo {
            installed: false,
            path: None,
            version: None,
        },
    }
}

const IP_TIMEOUT: Duration = Duration::from_secs(8);

fn parse_trace_ip(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("ip=")
            .map(str::trim)
            .filter(|ip| !ip.is_empty())
            .map(str::to_string)
    })
}

fn parse_plain_ip(text: &str) -> Option<String> {
    let ip = text.lines().next().unwrap_or_default().trim();
    if ip.is_empty() {
        None
    } else {
        Some(ip.to_string())
    }
}

/// Current public exit IP, mirroring the frontend's sources.
/// Never errors; total failure reports `"unreachable"`.
pub async fn fetch_exit_ip() -> String {
    let Ok(client) = reqwest::Client::builder().timeout(IP_TIMEOUT).build() else {
        return "unreachable".to_string();
    };
    if let Ok(resp) = client
        .get("https://api.cloudflare.com/cdn-cgi/trace")
        .send()
        .await
    {
        if let Ok(resp) = resp.error_for_status() {
            if let Ok(text) = resp.text().await {
                if let Some(ip) = parse_trace_ip(&text) {
                    return ip;
                }
            }
        }
    }
    if let Ok(resp) = client.get("https://ifconfig.me/ip").send().await {
        if let Ok(resp) = resp.error_for_status() {
            if let Ok(text) = resp.text().await {
                if let Some(ip) = parse_plain_ip(&text) {
                    return ip;
                }
            }
        }
    }
    "unreachable".to_string()
}

pub async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exit_ip_bodies() {
        let trace = "fl=1\nh=api.cloudflare.com\nip=203.0.113.7\nts=1\n";
        assert_eq!(parse_trace_ip(trace).as_deref(), Some("203.0.113.7"));
        assert_eq!(parse_trace_ip("fl=1\nno ip here\n"), None);
        assert_eq!(
            parse_plain_ip("198.51.100.9\n").as_deref(),
            Some("198.51.100.9")
        );
        assert_eq!(parse_plain_ip("  \n"), None);
    }

    #[test]
    fn parses_mode_values() {
        let doh: Result<String, String> = Ok("doh".to_string());
        let warp: Result<String, String> = Ok("warp".to_string());
        let warp_dot: Result<String, String> = Ok("warp+dot".to_string());
        assert_eq!(parse_mode("(user set)\tMode: DnsOverHttps"), doh);
        assert_eq!(parse_mode("Mode: Warp"), warp);
        assert_eq!(parse_mode("Mode: WarpDnsOverTls"), warp_dot);
        assert!(parse_mode("Exclude mode, with hosts/ips:").is_err());
        assert!(parse_mode("Mode: SomethingNew").is_err());
    }

    #[test]
    fn parses_status_values() {
        let connected = parse_status(r#"{"status":"Connected","reason":"NetworkHealthy"}"#)
            .expect("connected must parse");
        assert!(connected.connected);
        assert_eq!(connected.status, "Connected");

        let connecting = parse_status(r#"{"status":"Connecting","reason":"CheckingNetwork"}"#)
            .expect("connecting must parse");
        assert!(!connecting.connected);
        assert_eq!(connecting.status, "Connecting");
        assert_eq!(connecting.reason, "CheckingNetwork");

        let disconnected = parse_status(r#"{"status":"Disconnected","reason":"Manual"}"#)
            .expect("disconnected must parse");
        assert!(!disconnected.connected);

        assert!(parse_status("not json").is_err());
    }

    #[test]
    fn parses_registration_fields() {
        let sample = "Account type: Free\nID: abc\nDevice ID: def-123\nPublic key: xyz\n";
        assert_eq!(field(sample, "Account type:"), "Free");
        assert_eq!(field(sample, "Device ID:"), "def-123");
        assert_eq!(field(sample, "Missing:"), "");
    }

    #[tokio::test]
    async fn live_status_parses() {
        let status = get_status().await.expect("warp-cli status must parse");
        assert!(!status.status.is_empty());
    }

    #[tokio::test]
    async fn live_identity_parses() {
        let identity = get_identity().await.expect("registration show must parse");
        assert!(!identity.device_id.is_empty());
    }
}
