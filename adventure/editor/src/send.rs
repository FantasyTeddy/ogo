//! Sending a world to the device, through the launcher's web API: the same upload its web page
//! uses for the Adventure app's "Worlds" folder.

use std::time::Duration;

/// A file name the launcher accepts, made from a world's title: without `/ \ : * ? " < > |` and
/// control characters, not starting with "." or ending in "." or a space.
pub fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let stem = if trimmed.is_empty() { "world" } else { trimmed };
    format!("{stem}.adv")
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The host part of an address as typed, e.g. "http://192.168.1.23/" → "192.168.1.23".
fn host(address: &str) -> Result<&str, String> {
    let host = address
        .trim()
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if host.is_empty() {
        Err("Enter the device's address: its WiFi screen shows it".into())
    } else {
        Ok(host)
    }
}

/// Whether the launcher answers at `address`, and whether the Adventure app is installed: e.g.
/// "ogo 0.3.0, Adventure installed".
pub fn check(address: &str) -> Result<String, String> {
    let host = host(address)?;
    let get = |path: &str| -> Result<serde_json::Value, String> {
        let mut response = ureq::get(&format!("http://{host}{path}"))
            .config()
            .timeout_global(Some(Duration::from_secs(4)))
            .build()
            .call()
            .map_err(|_| format!("Not reachable at {host}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        serde_json::from_str(&body).map_err(|_| format!("{host} isn't an ogo device"))
    };
    let status = get("/api/status")?;
    let version = status["version"].as_str().unwrap_or("?").to_string();
    let apps = get("/api/apps")?;
    let installed = apps
        .as_array()
        .is_some_and(|apps| apps.iter().any(|app| app["id"] == "adventure"));
    if installed {
        Ok(format!("Found: ogo {version}, Adventure installed"))
    } else {
        Err(format!(
            "Found ogo {version}, but the Adventure app isn't installed"
        ))
    }
}

/// Uploads `bytes` as `name` to the device at `address` (e.g. "192.168.1.23"); the launcher's
/// answer on success, or why it failed.
pub fn send(address: &str, name: &str, bytes: &[u8]) -> Result<String, String> {
    let host = host(address)?;
    let url = format!(
        "http://{host}/api/apps/adventure/folders/0/{}",
        percent_encode(name)
    );
    let mut response = ureq::post(&url)
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .content_type("application/octet-stream")
        .send(bytes)
        .map_err(|e| format!("can't reach {host}: {e}"))?;
    let status = response.status();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    // The launcher answers {"message": "..."}
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|json| json["message"].as_str().map(str::to_string))
        .unwrap_or(body);
    match status.as_u16() {
        200 => Ok(message),
        404 => Err(format!("{message} (is the Adventure app installed?)")),
        _ => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_launcher_accepts() {
        assert_eq!(file_name("Grandma's Village"), "Grandma's Village.adv");
        assert_eq!(file_name("a/b: c?"), "a_b_ c_.adv");
        assert_eq!(file_name(" .hidden "), "hidden.adv");
        assert_eq!(file_name("  "), "world.adv");
        assert_eq!(
            percent_encode("Grandma's Village.adv"),
            "Grandma%27s%20Village.adv"
        );
        assert_eq!(percent_encode("Bär"), "B%C3%A4r");
    }
}
