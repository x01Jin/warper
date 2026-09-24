use std::path::Path;

pub const MAX_LOG_LINES: usize = 5000;
pub const MAX_IP_HISTORY: usize = 10;
const SEAL_PREFIX: &str = "enc:";

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedEntry {
    pub enc: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultData {
    #[serde(default)]
    pub history: Vec<SealedEntry>,
    #[serde(default)]
    pub lines: Vec<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn local_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let digits = hex
        .bytes()
        .map(|b| (b as char).to_digit(16).map(|d| d as u8))
        .collect::<Option<Vec<u8>>>()?;
    digits.chunks(2).map(|c| Some(c[0] << 4 | c[1])).collect()
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        CryptProtectData(
            &input,
            windows::core::PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("CryptProtectData failed: {e:?}"))?;
        if output.pbData.is_null() {
            return Err("CryptProtectData returned null output".to_string());
        }
        if output.cbData > 8 * 1024 * 1024 {
            let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
            return Err("DPAPI output too large".to_string());
        }
        let encrypted = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
        Ok(encrypted)
    }
}

#[cfg(windows)]
fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("CryptUnprotectData failed: {e:?}"))?;
        if output.pbData.is_null() {
            return Err("CryptUnprotectData returned null output".to_string());
        }
        if output.cbData > 8 * 1024 * 1024 {
            let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
            return Err("DPAPI output too large".to_string());
        }
        let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
        Ok(plain)
    }
}

pub fn seal_ip(ip: &str) -> Option<String> {
    #[cfg(windows)]
    {
        protect(ip.as_bytes())
            .ok()
            .map(|blob| format!("{SEAL_PREFIX}{}", hex_encode(&blob)))
    }
    #[cfg(not(windows))]
    {
        let _ = ip;
        None
    }
}

pub fn open_ip(token: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let hex = token.strip_prefix(SEAL_PREFIX)?;
        let blob = hex_decode(hex)?;
        let plain = unprotect(&blob).ok()?;
        String::from_utf8(plain).ok()
    }
    #[cfg(not(windows))]
    {
        let _ = token;
        None
    }
}

fn is_ip_token(word: &str) -> bool {
    word.parse::<std::net::IpAddr>().is_ok()
}

pub fn seal_line(line: &str) -> String {
    line.split(' ')
        .map(|word| {
            if is_ip_token(word) {
                seal_ip(word).unwrap_or_else(|| word.to_string())
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

pub fn open_line(line: &str) -> String {
    line.split(' ')
        .map(|word| {
            if word.starts_with(SEAL_PREFIX) {
                open_ip(word).unwrap_or_else(|| word.to_string())
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn split_counter(body: &str) -> (&str, usize) {
    if let Some((base, rest)) = body.rsplit_once(" (x") {
        if let Some(n) = rest.strip_suffix(')') {
            if let Ok(n) = n.parse::<usize>() {
                return (base, n);
            }
        }
    }
    (body, 1)
}

pub fn push_line_data(data: &mut VaultData, message: &str) {
    let sealed = seal_line(&format!("[{}] {message}", local_stamp()));
    let dup_count: Option<usize> = data.lines.last().and_then(|l| {
        let (_, m) = l.split_once("] ")?;
        let opened = open_line(m);
        let (base, n) = split_counter(&opened);
        if base == message {
            Some(n)
        } else {
            None
        }
    });
    match dup_count {
        Some(n) => {
            data.lines.pop();
            let line = seal_line(&format!("[{}] {message} (x{})", local_stamp(), n + 1));
            data.lines.push(line);
        }
        None => data.lines.push(sealed),
    }
    if data.lines.len() > MAX_LOG_LINES {
        let excess = data.lines.len() - MAX_LOG_LINES;
        data.lines.drain(..excess);
    }
}

pub fn next_history(current: &[SealedEntry], ip: &str, at: u64) -> Option<Vec<SealedEntry>> {
    if ip.is_empty() || ip == "unreachable" {
        return None;
    }
    if current
        .first()
        .and_then(|e| open_ip(&e.enc))
        .is_some_and(|first| first == ip)
    {
        return None;
    }
    let enc = seal_ip(ip)?;
    let mut next = Vec::with_capacity(current.len() + 1);
    next.push(SealedEntry {
        enc,
        at: if at == 0 { now_ms() } else { at },
    });
    next.extend(current.iter().cloned());
    next.truncate(MAX_IP_HISTORY);
    Some(next)
}

pub fn open_lines(data: &VaultData) -> Vec<String> {
    data.lines.iter().map(|l| open_line(l)).collect()
}

fn convert_v1(bytes: &[u8]) -> Option<VaultData> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct V1Entry {
        #[serde(default)]
        ip: String,
        #[serde(default)]
        at: u64,
    }
    #[derive(serde::Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    struct V1 {
        #[serde(default)]
        history: Vec<V1Entry>,
        #[serde(default)]
        lines: Vec<String>,
    }
    #[cfg(windows)]
    let plain = unprotect(bytes).ok()?;
    #[cfg(not(windows))]
    let plain = bytes.to_vec();
    let v1: V1 = serde_json::from_slice(&plain).ok()?;
    let mut data = VaultData::default();
    for entry in v1.history.iter().take(MAX_IP_HISTORY) {
        if entry.ip.is_empty() {
            continue;
        }
        data.history.push(SealedEntry {
            enc: seal_ip(&entry.ip).unwrap_or_else(|| entry.ip.clone()),
            at: entry.at,
        });
    }
    for line in v1.lines.iter().take(MAX_LOG_LINES) {
        data.lines.push(seal_line(line));
    }
    Some(data)
}

pub fn load(path: &Path) -> VaultData {
    let bytes = std::fs::read(path).unwrap_or_default();
    if bytes.is_empty() {
        return VaultData::default();
    }
    if let Ok(data) = serde_json::from_slice::<VaultData>(&bytes) {
        return data;
    }
    convert_v1(&bytes).unwrap_or_default()
}

pub fn save(path: &Path, data: &VaultData) -> Result<(), String> {
    let bytes = serde_json::to_vec(data).map_err(|e| format!("vault encode failed: {e}"))?;
    std::fs::write(path, bytes).map_err(|e| format!("vault write failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_only_ip_tokens() {
        let sealed = seal_line("warp-cli 2026.7.1376.0 ok");
        assert_eq!(sealed, "warp-cli 2026.7.1376.0 ok");
        let ip_sealed = seal_line("exit 203.0.113.7 done");
        assert!(ip_sealed.starts_with("exit enc:"));
        assert!(ip_sealed.ends_with(" done"));
        assert_eq!(open_line(&ip_sealed), "exit 203.0.113.7 done");
    }

    #[test]
    fn unsealable_tokens_survive() {
        assert_eq!(open_line("oops enc:zz top"), "oops enc:zz top");
    }

    #[test]
    fn trims_lines_to_cap() {
        let mut data = VaultData::default();
        for i in 0..MAX_LOG_LINES + 50 {
            data.lines.push(format!("line {i}"));
        }
        if data.lines.len() > MAX_LOG_LINES {
            let excess = data.lines.len() - MAX_LOG_LINES;
            data.lines.drain(..excess);
        }
        assert_eq!(data.lines.len(), MAX_LOG_LINES);
        assert_eq!(data.lines[0], "line 50");
    }

    #[test]
    fn rejects_unreachable_ip() {
        let current: Vec<SealedEntry> = vec![];
        assert_eq!(next_history(&current, "unreachable", 0), None);
        assert_eq!(next_history(&current, "", 0), None);
    }

    #[test]
    fn records_and_dedupes_ip() {
        let empty: Vec<SealedEntry> = vec![];
        let first = next_history(&empty, "203.0.113.7", 0).expect("record");
        assert_eq!(next_history(&first, "203.0.113.7", 0), None);
        let second = next_history(&first, "198.51.100.9", 0).expect("change");
        assert_eq!(second.len(), 2);
        assert_eq!(open_ip(&second[0].enc).as_deref(), Some("198.51.100.9"));
        assert_eq!(open_ip(&second[1].enc).as_deref(), Some("203.0.113.7"));
        let raw = serde_json::to_string(&second).expect("encode");
        assert!(!raw.contains("203.0.113.7"));
        assert!(!raw.contains("198.51.100.9"));
    }

    #[test]
    fn file_round_trip_collapses_dupes() {
        let path = std::env::temp_dir().join("warper-vault-roundtrip.log");
        let _ = std::fs::remove_file(&path);
        let mut data = VaultData::default();
        push_line_data(&mut data, "hello");
        push_line_data(&mut data, "hello");
        save(&path, &data).expect("save");
        let back = load(&path);
        assert_eq!(back.lines.len(), 1);
        assert!(back.lines[0].ends_with("hello (x2)"));
        let _ = std::fs::remove_file(&path);
    }
}
