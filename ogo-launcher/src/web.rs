//! The web page and its JSON API (docs/CONTRACTS.md, "Web API").
//!
//! Everything under `/api/` except the login needs a session: `POST /api/login` with the PIN shown
//! on the device sets a session cookie. After [`MAX_FAILED_LOGINS`] wrong PINs the device picks a
//! new one, so guessing doesn't get anywhere.
//!
//! Handlers run in the HTTP server's task and report what happened to the launcher through
//! [`WebEvent`]s.

use std::{
    fs::{self, File},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, Mutex},
};

use anyhow::Context;
use esp_idf_svc::{
    http::{
        server::{Configuration, EspHttpConnection, EspHttpServer, Request},
        Method,
    },
    io::{Read, Write},
};
use ogo_board::{
    boot,
    flash::{self, Progress},
    image::{self, ImageKind},
    BatteryLevel,
};
use ogo_format::{
    app::{Folder, Kind},
    slots,
    url::percent_decode,
};
use serde::Serialize;

use crate::{
    library::{self, Entry, ICON_SIZE},
    network::ScannedNetwork,
    package,
    settings::WifiCredentials,
};

const INDEX_HTML: &str = include_str!("web/index.html");
const LOGO_PNG: &[u8] = include_bytes!("../../docs/logo.png");
/// Larger uploads are refused before anything is written.
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const CHUNK_BYTES: usize = 4096;
/// Wrong PINs before a new one is chosen.
const MAX_FAILED_LOGINS: u32 = 5;
const SESSION_COOKIE: &str = "ogo_session";

/// What happened in a request, for the device screen.
#[derive(Debug, Clone)]
pub enum WebEvent {
    Uploading {
        received: usize,
        total: usize,
    },
    Installed {
        name: String,
    },
    Deleted {
        id: String,
    },
    /// A file was uploaded into an app's folder.
    FileSaved {
        name: String,
    },
    FileDeleted {
        name: String,
    },
    Failed(String),
    WifiSaved(WifiCredentials),
    /// Too many wrong PINs; the device shows this new one.
    PinChanged(String),
    /// A launcher update is being written to flash.
    UpdateStarted,
    Updating {
        received: usize,
        total: usize,
    },
    /// A launcher update is in `slot` and checked; the launcher selects it and restarts.
    LauncherWritten {
        slot: &'static str,
    },
    ScanRequested,
    Restart,
}

/// How the device is connected, for the status.
#[derive(Debug, Clone, Serialize)]
pub struct NetworkInfo {
    /// "station" (joined a WiFi) or "access_point" (its own).
    pub mode: &'static str,
    /// The WiFi's name.
    pub name: String,
    pub ip: String,
}

/// What the handlers share. Lives as long as the server.
pub struct WebState {
    pub pin: Mutex<String>,
    /// Session tokens handed out by the login.
    sessions: Mutex<Vec<String>>,
    failed_logins: Mutex<u32>,
    /// From the last WiFi scan.
    pub networks: Mutex<Vec<ScannedNetwork>>,
    /// The SD card root, e.g. `/sd`; `None` without a card.
    pub sd_root: Option<PathBuf>,
    pub network: NetworkInfo,
    pub battery: Mutex<Option<BatteryLevel>>,
    pub events: Mutex<Sender<WebEvent>>,
}

impl WebState {
    pub fn new(
        pin: String,
        sd_root: Option<PathBuf>,
        network: NetworkInfo,
        networks: Vec<ScannedNetwork>,
        battery: Option<BatteryLevel>,
        events: Sender<WebEvent>,
    ) -> Self {
        Self {
            pin: Mutex::new(pin),
            sessions: Mutex::new(Vec::new()),
            failed_logins: Mutex::new(0),
            networks: Mutex::new(networks),
            sd_root,
            network,
            battery: Mutex::new(battery),
            events: Mutex::new(events),
        }
    }

    fn send(&self, event: WebEvent) {
        let _ = self.events.lock().unwrap().send(event);
    }

    fn apps_dir(&self) -> Option<PathBuf> {
        self.sd_root
            .as_ref()
            .map(|root| root.join(library::APPS_DIR))
    }
}

pub struct WebServer {
    _server: EspHttpServer<'static>,
}

type Req<'a, 'b> = Request<&'a mut EspHttpConnection<'b>>;

impl WebServer {
    pub fn start(state: Arc<WebState>) -> anyhow::Result<Self> {
        let mut server = EspHttpServer::new(&Configuration {
            stack_size: 10240,
            max_uri_handlers: 16,
            uri_match_wildcard: true,
            ..Default::default()
        })?;

        server.fn_handler::<anyhow::Error, _>("/", Method::Get, |req| {
            req.into_response(200, None, &[("Content-Type", "text/html; charset=utf-8")])?
                .write_all(INDEX_HTML.as_bytes())?;
            Ok(())
        })?;
        server.fn_handler::<anyhow::Error, _>("/logo.png", Method::Get, |req| {
            req.into_response(
                200,
                None,
                &[
                    ("Content-Type", "image/png"),
                    ("Cache-Control", "max-age=86400"),
                ],
            )?
            .write_all(LOGO_PNG)?;
            Ok(())
        })?;

        let s = state.clone();
        server.fn_handler::<anyhow::Error, _>("/api/login", Method::Post, move |req| {
            login(req, &s)
        })?;

        // Everything else needs a session.
        type Handler = fn(Req, &WebState) -> anyhow::Result<()>;
        let routes: [(&str, Method, Handler); 11] = [
            ("/api/status", Method::Get, status),
            ("/api/apps", Method::Get, list_apps),
            ("/api/apps", Method::Post, upload),
            ("/api/apps/*", Method::Get, app_get),
            ("/api/apps/*", Method::Post, app_post),
            ("/api/apps/*", Method::Delete, app_delete),
            ("/api/wifi", Method::Post, set_wifi),
            ("/api/wifi/networks", Method::Get, networks),
            ("/api/wifi/scan", Method::Post, scan),
            ("/api/restart", Method::Post, restart),
            ("/api/launcher", Method::Post, update_launcher),
        ];
        for (uri, method, handler) in routes {
            let s = state.clone();
            server.fn_handler::<anyhow::Error, _>(uri, method, move |req| {
                if !authorized(&req, &s) {
                    return json(req, 401, &Message::new("Enter the PIN shown on the device"));
                }
                handler(req, &s)
            })?;
        }

        Ok(Self { _server: server })
    }
}

#[derive(Serialize)]
struct Message {
    message: String,
}

impl Message {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

fn json(req: Req, status: u16, body: &impl Serialize) -> anyhow::Result<()> {
    let body = serde_json::to_vec(body)?;
    req.into_response(status, None, &[("Content-Type", "application/json")])?
        .write_all(&body)?;
    Ok(())
}

/// The body length from `Content-Length`; 0 if missing.
fn content_len(req: &Req) -> usize {
    req.header("Content-Length")
        .and_then(|len| len.trim().parse().ok())
        .unwrap_or(0)
}

/// Whether the request carries a session cookie from [`login`].
fn authorized(req: &Req, state: &WebState) -> bool {
    let Some(cookies) = req.header("Cookie") else {
        return false;
    };
    let sessions = state.sessions.lock().unwrap();
    cookies
        .split(';')
        .filter_map(|cookie| {
            cookie
                .trim()
                .strip_prefix(SESSION_COOKIE)?
                .strip_prefix('=')
        })
        .any(|token| sessions.iter().any(|session| session == token))
}

/// `POST /api/login` with `{"pin": "1234"}`: sets the session cookie.
fn login(mut req: Req, state: &WebState) -> anyhow::Result<()> {
    #[derive(serde::Deserialize)]
    struct Login {
        pin: String,
    }
    let len = content_len(&req);
    if len == 0 || len > 64 {
        return json(req, 400, &Message::new("Bad request"));
    }
    let mut body = vec![0u8; len];
    req.read_exact(&mut body)
        .map_err(|e| anyhow::anyhow!("connection error: {e:?}"))?;
    let pin = serde_json::from_slice::<Login>(&body)
        .map(|login| login.pin)
        .unwrap_or_default();

    if pin.trim() != *state.pin.lock().unwrap() {
        let mut failed = state.failed_logins.lock().unwrap();
        *failed += 1;
        if *failed >= MAX_FAILED_LOGINS {
            *failed = 0;
            let new_pin = random_pin();
            *state.pin.lock().unwrap() = new_pin.clone();
            state.send(WebEvent::PinChanged(new_pin));
            return json(
                req,
                403,
                &Message::new("Too many wrong PINs. The device shows a new one."),
            );
        }
        return json(req, 403, &Message::new("Wrong PIN"));
    }

    *state.failed_logins.lock().unwrap() = 0;
    let token = random_token();
    state.sessions.lock().unwrap().push(token.clone());
    let cookie = format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict");
    req.into_response(
        200,
        None,
        &[
            ("Content-Type", "application/json"),
            ("Set-Cookie", &cookie),
        ],
    )?
    .write_all(b"{\"message\":\"Welcome\"}")?;
    Ok(())
}

/// A 4-digit PIN from the hardware random number generator.
pub fn random_pin() -> String {
    format!("{:04}", unsafe { esp_idf_svc::sys::esp_random() } % 10_000)
}

fn random_token() -> String {
    (0..4)
        .map(|_| format!("{:08x}", unsafe { esp_idf_svc::sys::esp_random() }))
        .collect()
}

fn networks(req: Req, state: &WebState) -> anyhow::Result<()> {
    let networks = state.networks.lock().unwrap().clone();
    json(req, 200, &networks)
}

fn scan(req: Req, state: &WebState) -> anyhow::Result<()> {
    state.send(WebEvent::ScanRequested);
    json(req, 202, &Message::new("Scanning"))
}

/// `POST /api/launcher`: a launcher image (`launcher.bin`) as the raw body. Written straight into
/// the launcher slot that isn't running; the launcher then selects it and restarts.
fn update_launcher(mut req: Req, state: &WebState) -> anyhow::Result<()> {
    let total = content_len(&req);
    if total == 0 {
        return json(req, 400, &Message::new("Empty upload"));
    }
    let slot = boot::inactive_launcher_slot().label;
    state.send(WebEvent::UpdateStarted);

    let mut reported = 0;
    let result = flash::write_stream(
        |buf| {
            req.read(buf)
                .map_err(|e| anyhow::anyhow!("connection error: {e:?}"))
        },
        total as u64,
        slot,
        None,
        |progress| {
            if let Progress::Writing { written, total } = progress {
                let tenth = (written * 10 / total.max(1)) as usize;
                if tenth > reported {
                    reported = tenth;
                    state.send(WebEvent::Updating {
                        received: written as usize,
                        total: total as usize,
                    });
                }
            }
        },
    )
    .and_then(|()| {
        // A valid image, but is it a launcher? An app in a launcher slot would never return.
        match image::in_slot(slot) {
            Some(desc) if desc.kind() == Some(ImageKind::Launcher) => Ok(()),
            Some(desc) => anyhow::bail!("that's an {} image, not a launcher", desc.kind_name()),
            None => anyhow::bail!("not an ogo launcher image"),
        }
    });

    match result {
        Ok(()) => {
            json(req, 200, &Message::new("Installed. The device restarts into the new launcher; open its WiFi screen again to come back here."))?;
            state.send(WebEvent::LauncherWritten { slot });
            Ok(())
        }
        Err(e) => {
            state.send(WebEvent::Failed(format!("Update failed: {e:#}")));
            json(req, 400, &Message::new(format!("Update failed: {e:#}")))
        }
    }
}

fn restart(req: Req, state: &WebState) -> anyhow::Result<()> {
    json(req, 200, &Message::new("Restarting"))?;
    state.send(WebEvent::Restart);
    Ok(())
}

fn status(req: Req, state: &WebState) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct Slot {
        /// "A" or "B"
        name: &'static str,
        running: bool,
        /// What the slot holds, e.g. "ogo-launcher"; `None` if empty.
        kind: Option<String>,
        version: Option<String>,
        /// Build date and time as ESP-IDF records it, e.g. "Oct 11 2026 19:20:31".
        built: Option<String>,
        /// Tells builds of the same version apart, e.g. "3f9a2c1d".
        build_id: Option<String>,
        /// Git commit, e.g. "220357ce"; only in builds made with `cargo xtask`.
        commit: Option<String>,
    }
    #[derive(Serialize)]
    struct Status {
        version: &'static str,
        battery_percent: Option<u8>,
        battery_mv: Option<u32>,
        slots: Vec<Slot>,
        sd_total: Option<u64>,
        sd_free: Option<u64>,
        network: NetworkInfo,
    }
    let battery = *state.battery.lock().unwrap();
    let sd_space = state.sd_root.as_ref().and_then(|_| ogo_board::sd::space());
    let running = boot::running_partition_label();
    let slots = slots::LAUNCHER_SLOTS
        .iter()
        .map(|&slots::LauncherSlot { label, name }| {
            let desc = image::in_slot(label);
            let info = flash::app_info(label);
            Slot {
                name,
                running: running == label,
                kind: desc.map(|d| d.kind_name().to_owned()),
                version: desc.map(|d| d.version().to_owned()),
                commit: desc.and_then(|d| d.commit().map(str::to_owned)),
                built: info.as_ref().map(|info| info.built.clone()),
                build_id: info.map(|info| info.build_id),
            }
        })
        .collect();
    json(
        req,
        200,
        &Status {
            version: env!("CARGO_PKG_VERSION"),
            battery_percent: battery.map(|b| b.percent),
            battery_mv: battery.map(|b| b.millivolts),
            slots,
            sd_total: sd_space.map(|(total, _)| total),
            sd_free: sd_space.map(|(_, free)| free),
            network: state.network.clone(),
        },
    )
}

fn list_apps(req: Req, state: &WebState) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct AppJson {
        id: String,
        name: String,
        /// `None` for broken entries.
        kind: Option<Kind>,
        version: String,
        author: String,
        has_icon: bool,
        folders: Vec<Folder>,
        /// Why the entry can't be used, for broken ones.
        error: Option<String>,
    }
    let entries = state
        .apps_dir()
        .and_then(|dir| library::scan_dir(&dir))
        .unwrap_or_default();
    let apps: Vec<AppJson> = entries
        .into_iter()
        .map(|entry| match entry {
            Entry::App(app) => AppJson {
                id: app.id,
                name: app.name,
                kind: Some(app.kind),
                version: app.version,
                author: app.author,
                has_icon: app.has_icon,
                folders: app.folders,
                error: None,
            },
            Entry::Broken(broken) => AppJson {
                id: broken.folder.clone(),
                name: broken.folder,
                kind: None,
                version: String::new(),
                author: String::new(),
                has_icon: false,
                folders: Vec::new(),
                error: Some(broken.reason),
            },
        })
        .collect();
    json(req, 200, &apps)
}

/// `/api/apps/<id>/<rest>...`: the app id, if valid, and the rest of the path, percent-decoded.
fn app_path(uri: &str) -> Option<(String, Vec<String>)> {
    let path = uri.split('?').next()?.strip_prefix("/api/apps/")?;
    let mut parts = path.split('/');
    let id = parts.next().filter(|id| ogo_format::app::is_valid_id(id))?;
    let rest = parts.map(percent_decode).collect::<Option<Vec<_>>>()?;
    Some((id.to_owned(), rest))
}

fn app_get(req: Req, state: &WebState) -> anyhow::Result<()> {
    let Some((id, rest)) = app_path(req.uri()) else {
        return json(req, 404, &Message::new("Not found"));
    };
    match rest.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["icon.bmp"] => icon(req, state, &id),
        ["folders", folder] => list_files(req, state, &id, folder),
        _ => json(req, 404, &Message::new("Not found")),
    }
}

fn app_post(req: Req, state: &WebState) -> anyhow::Result<()> {
    let Some((id, rest)) = app_path(req.uri()) else {
        return json(req, 404, &Message::new("Not found"));
    };
    match rest.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["folders", folder, name] => upload_file(req, state, &id, folder, name),
        _ => json(req, 404, &Message::new("Not found")),
    }
}

fn app_delete(req: Req, state: &WebState) -> anyhow::Result<()> {
    let Some((id, rest)) = app_path(req.uri()) else {
        return json(req, 404, &Message::new("No such app"));
    };
    match rest.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => delete(req, state, &id),
        ["folders", folder, name] => delete_file(req, state, &id, folder, name),
        _ => json(req, 404, &Message::new("Not found")),
    }
}

fn upload(mut req: Req, state: &WebState) -> anyhow::Result<()> {
    let (Some(apps_dir), Some(root)) = (state.apps_dir(), state.sd_root.clone()) else {
        return json(req, 409, &Message::new("No SD card in the device"));
    };
    let total = content_len(&req);
    if total == 0 {
        return json(req, 400, &Message::new("Empty upload"));
    }
    if total > MAX_UPLOAD_BYTES {
        return json(
            req,
            413,
            &Message::new(format!("Too big: at most {} MB", MAX_UPLOAD_BYTES >> 20)),
        );
    }

    let path = package::upload_path(&root);
    if let Err(e) = receive(&mut req, &path, total, state) {
        let _ = fs::remove_file(&path);
        state.send(WebEvent::Failed("Upload failed".into()));
        return json(req, 400, &Message::new(format!("Upload failed: {e:#}")));
    }

    let result = package::install(&path, &apps_dir);
    let _ = fs::remove_file(&path);
    match result {
        Ok(app) => {
            state.send(WebEvent::Installed {
                name: app.name.clone(),
            });
            json(
                req,
                200,
                &Message::new(format!("Installed {} {}", app.name, app.version)),
            )
        }
        Err(e) => {
            state.send(WebEvent::Failed(format!("{e:#}")));
            json(req, 400, &Message::new(format!("{e:#}")))
        }
    }
}

/// Streams the request body into `path`, reporting progress about every 10 %.
fn receive(req: &mut Req, path: &Path, total: usize, state: &WebState) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file =
        File::create(path).with_context(|| format!("can't create {}", path.display()))?;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut received = 0;
    let mut reported = 0;
    while received < total {
        let count = req
            .read(&mut buffer)
            .map_err(|e| anyhow::anyhow!("connection error: {e:?}"))?;
        if count == 0 {
            anyhow::bail!("the connection closed early");
        }
        file.write_all(&buffer[..count])
            .context("writing to the SD card failed")?;
        received += count;
        if received * 10 / total > reported {
            reported = received * 10 / total;
            state.send(WebEvent::Uploading { received, total });
        }
    }
    Ok(())
}

fn delete(req: Req, state: &WebState, id: &str) -> anyhow::Result<()> {
    let Some(apps_dir) = state.apps_dir() else {
        return json(req, 404, &Message::new("No such app"));
    };
    let dir = apps_dir.join(id);
    if !dir.is_dir() {
        return json(req, 404, &Message::new("No such app"));
    }
    fs::remove_dir_all(&dir).with_context(|| format!("can't delete {id}"))?;
    state.send(WebEvent::Deleted { id: id.to_owned() });
    json(req, 200, &Message::new(format!("Deleted {id}")))
}

/// App `id`'s folder number `index` (from `app.toml`) and where it is on the SD card.
fn app_folder(state: &WebState, id: &str, index: &str) -> Result<(Folder, PathBuf), Message> {
    let (Some(apps_dir), Some(root)) = (state.apps_dir(), state.sd_root.as_ref()) else {
        return Err(Message::new("No SD card in the device"));
    };
    let app =
        library::load_app(&apps_dir.join(id), Some(id)).map_err(|_| Message::new("No such app"))?;
    let folder = index
        .parse::<usize>()
        .ok()
        .and_then(|index| app.folders.into_iter().nth(index))
        .ok_or_else(|| Message::new("No such folder"))?;
    let dir = root.join(&folder.path);
    Ok((folder, dir))
}

/// `GET /api/apps/<id>/folders/<n>`: the files in the folder the app reads, by name.
fn list_files(req: Req, state: &WebState, id: &str, index: &str) -> anyhow::Result<()> {
    #[derive(Serialize)]
    struct FileJson {
        name: String,
        size: u64,
    }
    let (folder, dir) = match app_folder(state, id, index) {
        Ok(found) => found,
        Err(message) => return json(req, 404, &message),
    };
    // A folder that doesn't exist yet is just empty
    let mut files: Vec<FileJson> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let meta = entry.metadata().ok()?;
            (meta.is_file() && folder.accepts(&name)).then_some(FileJson {
                name,
                size: meta.len(),
            })
        })
        .collect();
    files.sort_by_key(|file| file.name.to_lowercase());
    json(req, 200, &files)
}

/// `POST /api/apps/<id>/folders/<n>/<name>` with the file as the raw body. It's received under a
/// temporary name, so a broken upload never replaces a file; one with the same name is replaced.
fn upload_file(
    mut req: Req,
    state: &WebState,
    id: &str,
    index: &str,
    name: &str,
) -> anyhow::Result<()> {
    let (folder, dir) = match app_folder(state, id, index) {
        Ok(found) => found,
        Err(message) => return json(req, 404, &message),
    };
    if !folder.accepts(name) {
        let extensions: Vec<String> = folder.extensions.iter().map(|e| format!(".{e}")).collect();
        let reason = if ogo_format::app::is_valid_file_name(name) {
            format!("only {} files", extensions.join(", "))
        } else {
            "names can't start with \".\", end in \".\" or a space, or contain / \\ : * ? \" < > |"
                .to_owned()
        };
        return json(req, 400, &Message::new(format!("{name}: {reason}")));
    }
    let total = content_len(&req);
    if total == 0 {
        return json(req, 400, &Message::new("Empty upload"));
    }
    if total > MAX_UPLOAD_BYTES {
        return json(
            req,
            413,
            &Message::new(format!("Too big: at most {} MB", MAX_UPLOAD_BYTES >> 20)),
        );
    }

    let partial = dir.join(".upload.part");
    let target = dir.join(name);
    let result = receive(&mut req, &partial, total, state).and_then(|()| {
        // FAT can't rename onto an existing file
        if target.exists() {
            fs::remove_file(&target).with_context(|| format!("can't replace {name}"))?;
        }
        fs::rename(&partial, &target).with_context(|| format!("can't save {name}"))
    });
    if let Err(e) = result {
        let _ = fs::remove_file(&partial);
        state.send(WebEvent::Failed("Upload failed".into()));
        return json(req, 400, &Message::new(format!("Upload failed: {e:#}")));
    }
    state.send(WebEvent::FileSaved {
        name: name.to_owned(),
    });
    json(req, 200, &Message::new(format!("Saved {name}")))
}

/// `DELETE /api/apps/<id>/folders/<n>/<name>`.
fn delete_file(
    req: Req,
    state: &WebState,
    id: &str,
    index: &str,
    name: &str,
) -> anyhow::Result<()> {
    let (folder, dir) = match app_folder(state, id, index) {
        Ok(found) => found,
        Err(message) => return json(req, 404, &message),
    };
    let path = dir.join(name);
    if !folder.accepts(name) || !path.is_file() {
        return json(req, 404, &Message::new("No such file"));
    }
    fs::remove_file(&path).with_context(|| format!("can't delete {name}"))?;
    state.send(WebEvent::FileDeleted {
        name: name.to_owned(),
    });
    json(req, 200, &Message::new(format!("Deleted {name}")))
}

/// `GET /api/apps/<id>/icon.bmp`: the icon as a 16-bit BMP, which browsers show directly.
fn icon(req: Req, state: &WebState, id: &str) -> anyhow::Result<()> {
    let path = state.apps_dir().map(|dir| dir.join(id).join("icon.rgb565"));
    let Some(pixels) = path.and_then(|path| fs::read(path).ok()) else {
        return json(req, 404, &Message::new("No icon"));
    };
    if pixels.len() != (ICON_SIZE * ICON_SIZE * 2) as usize {
        return json(req, 404, &Message::new("No icon"));
    }
    let bmp = ogo_format::app::icon_bmp(&pixels, ICON_SIZE);
    req.into_response(
        200,
        None,
        &[("Content-Type", "image/bmp"), ("Cache-Control", "no-cache")],
    )?
    .write_all(&bmp)?;
    Ok(())
}

fn set_wifi(mut req: Req, state: &WebState) -> anyhow::Result<()> {
    let len = content_len(&req);
    if len == 0 || len > 512 {
        return json(req, 400, &Message::new("Bad request"));
    }
    let mut body = vec![0u8; len];
    req.read_exact(&mut body)
        .map_err(|e| anyhow::anyhow!("connection error: {e:?}"))?;
    let credentials: WifiCredentials = match serde_json::from_slice(&body) {
        Ok(credentials) => credentials,
        Err(_) => {
            return json(
                req,
                400,
                &Message::new("Expected {\"ssid\": ..., \"password\": ...}"),
            )
        }
    };
    if credentials.ssid.is_empty() || credentials.ssid.len() > 32 || credentials.password.len() > 64
    {
        return json(
            req,
            400,
            &Message::new("The SSID needs 1-32 characters, the password at most 64"),
        );
    }
    json(
        req,
        200,
        &Message::new(format!("Saved. The device now joins {}.", credentials.ssid)),
    )?;
    state.send(WebEvent::WifiSaved(credentials));
    Ok(())
}
