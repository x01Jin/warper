use std::time::Duration;

const RELEASES_API: &str = "https://api.github.com/repos/x01Jin/warper/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/x01Jin/warper/releases";
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub url: String,
    pub available: bool,
}

fn version_parts(version: &str) -> [u64; 3] {
    let mut parts = [0u64; 3];
    for (slot, piece) in parts.iter_mut().zip(version.split(['.', '-', '+']).take(3)) {
        let digits: String = piece.chars().take_while(char::is_ascii_digit).collect();
        *slot = digits.parse().unwrap_or(0);
    }
    parts
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    version_parts(latest) > version_parts(current)
}

pub async fn check(current: &str) -> Result<UpdateInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| format!("http client failed: {error}"))?;
    let response = client
        .get(RELEASES_API)
        .header("User-Agent", "warper-update-check")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("release lookup failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("github returned {}", response.status()));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("release read failed: {error}"))?;
    let release: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| format!("release parse failed: {error}"))?;
    let tag = release
        .get("tag_name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "release has no tag".to_string())?;
    let latest = tag.trim_start_matches(['v', 'V']).to_string();
    let url = release
        .get("html_url")
        .and_then(|value| value.as_str())
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    Ok(UpdateInfo {
        current: current.to_string(),
        available: is_newer(&latest, current),
        latest,
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(!is_newer("", "0.1.2"));
        assert!(is_newer("0.1.3-beta", "0.1.2"));
    }
}
