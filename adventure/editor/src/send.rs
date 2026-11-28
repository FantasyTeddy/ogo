//! Sending a world to the device, through the launcher's web API: the same upload its web page
//! uses for the Adventure app's "Worlds" folder. The API answers only while the device's WiFi
//! screen is open, and only after a login with the PIN that screen shows.

use std::{net::Ipv4Addr, time::Duration};

/// How long connecting may take; the device is on the same network.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// The launcher's session cookie, set by its login.
const COOKIE: &str = "ogo_session";

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

/// The device's IP address as typed, e.g. "192.168.1.23".
pub fn host(address: &str) -> Result<Ipv4Addr, String> {
    address
        .trim()
        .parse()
        .map_err(|_| "Type the address as the device shows it, e.g. 192.168.1.23".into())
}

/// The PIN's length: the launcher picks 4 digits.
pub const PIN_DIGITS: usize = 4;

/// The PIN as typed, without the spaces the device shows between its digits.
pub fn pin(typed: &str) -> Result<String, String> {
    let pin: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
    if pin.len() == PIN_DIGITS && pin.bytes().all(|b| b.is_ascii_digit()) {
        Ok(pin)
    } else {
        Err(format!(
            "The PIN is the {PIN_DIGITS} digits the device shows"
        ))
    }
}

/// The `ogo_session=…` part of a `Set-Cookie` header.
fn session_cookie(header: &str) -> Option<&str> {
    let cookie = header.split(';').next()?.trim();
    cookie
        .strip_prefix(COOKIE)?
        .strip_prefix('=')
        .filter(|token| !token.is_empty())
        .map(|_| cookie)
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build()
        .into()
}

/// The text of the launcher's answer: it answers `{"message": "..."}`.
fn message(response: &mut ureq::http::Response<ureq::Body>) -> String {
    let body = response.body_mut().read_to_string().unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|json| json["message"].as_str().map(str::to_string))
        .unwrap_or(body)
}

fn unreachable(host: &str) -> String {
    format!("Can't reach {host}. Is the device's WiFi screen open?")
}

fn not_ogo(host: &str) -> String {
    format!("{host} isn't an ogo device")
}

/// Logged in to the launcher at an address.
#[derive(Clone)]
pub struct Session {
    /// As typed, to notice when it's changed
    pub address: String,
    host: String,
    /// `ogo_session=…`
    cookie: String,
    /// What was found, e.g. "ogo 0.3.0, Adventure installed"
    pub found: String,
}

/// Logs in to the launcher at `address` with the PIN its WiFi screen shows, and checks that the
/// Adventure app is installed.
pub fn connect(address: &str, typed_pin: &str) -> Result<Session, String> {
    let host = host(address)?.to_string();
    let pin = pin(typed_pin)?;
    let agent = agent(CONNECT_TIMEOUT);
    let login = serde_json::json!({ "pin": pin }).to_string();
    let mut response = agent
        .post(&format!("http://{host}/api/login"))
        .content_type("application/json")
        .send(login.as_bytes())
        .map_err(|_| unreachable(&host))?;
    match response.status().as_u16() {
        200 => {}
        // "Wrong PIN", or that the device picked a new one
        403 => return Err(message(&mut response)),
        _ => return Err(not_ogo(&host)),
    }
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| session_cookie(value.to_str().ok()?))
        .next()
        .ok_or_else(|| not_ogo(&host))?
        .to_string();

    let get = |path: &str| -> Result<serde_json::Value, String> {
        let mut response = agent
            .get(&format!("http://{host}{path}"))
            .header("Cookie", &cookie)
            .call()
            .map_err(|_| unreachable(&host))?;
        if response.status().as_u16() != 200 {
            return Err(not_ogo(&host));
        }
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|_| unreachable(&host))?;
        serde_json::from_str(&body).map_err(|_| not_ogo(&host))
    };
    let status = get("/api/status")?;
    let version = status["version"].as_str().unwrap_or("?").to_string();
    let apps = get("/api/apps")?;
    let installed = apps
        .as_array()
        .is_some_and(|apps| apps.iter().any(|app| app["id"] == "adventure"));
    if !installed {
        return Err(format!(
            "Found ogo {version}, but the Adventure app isn't installed"
        ));
    }
    Ok(Session {
        address: address.trim().to_string(),
        host,
        cookie,
        found: format!("ogo {version}, Adventure installed"),
    })
}

/// Why a world wasn't sent.
pub enum Failure {
    /// The device forgot the session: its WiFi screen was closed, or it restarted. Connect
    /// again with the PIN it shows now.
    LoggedOut,
    Other(String),
}

/// Uploads `bytes` as `name` into the Adventure app's worlds; the launcher's answer on success.
pub fn send(session: &Session, name: &str, bytes: &[u8]) -> Result<String, Failure> {
    let host = &session.host;
    let url = format!(
        "http://{host}/api/apps/adventure/folders/0/{}",
        percent_encode(name)
    );
    let mut response = agent(SEND_TIMEOUT)
        .post(&url)
        .header("Cookie", &session.cookie)
        .content_type("application/octet-stream")
        .send(bytes)
        .map_err(|_| Failure::Other(unreachable(host)))?;
    let status = response.status().as_u16();
    let message = message(&mut response);
    match status {
        200 => Ok(message),
        401 => Err(Failure::LoggedOut),
        404 => Err(Failure::Other(format!(
            "{message} (is the Adventure app installed?)"
        ))),
        _ => Err(Failure::Other(message)),
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

    #[test]
    fn addresses_pins_and_cookies() {
        let ip = Ipv4Addr::new(192, 168, 1, 23);
        assert_eq!(host(" 192.168.1.23 "), Ok(ip));
        for wrong in [
            "",
            "192.168.1",
            "192.168.1.256",
            "anything",
            "http://192.168.1.23/",
            "1.2.3.4:80",
        ] {
            assert!(host(wrong).is_err(), "{wrong:?}");
        }
        assert_eq!(pin(" 1 2 3 4 "), Ok("1234".into()));
        for wrong in ["", "123", "12345", "12a4", "١٢٣٤"] {
            assert!(pin(wrong).is_err(), "{wrong:?}");
        }
        assert_eq!(
            session_cookie("ogo_session=abc; Path=/; HttpOnly; SameSite=Strict"),
            Some("ogo_session=abc")
        );
        assert_eq!(session_cookie("other=abc; Path=/"), None);
        assert_eq!(session_cookie("ogo_session=; Path=/"), None);
    }
}
