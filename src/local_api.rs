use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, WrapErr, bail};
use serde::Serialize;

use crate::desktop::events::{EventLog, VoiceEvent, now_ms};

mod dictation;
mod discovery;
mod http;

#[cfg(debug_assertions)]
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
pub use discovery::call_developer;
use discovery::*;
pub use discovery::{call_control, running_pid};
use http::*;
const API_VERSION: &str = "2";
const LIVE_PROBE_TIMEOUT: Duration = Duration::from_millis(150);
const ACCEPT_INTERVAL: Duration = Duration::from_millis(10);
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(500);
const HEADER_DEADLINE: Duration = Duration::from_secs(2);
const AUTH_OBSERVATION_INTERVAL: Duration = Duration::from_secs(5);
const HTTP_WORKERS: usize = 4;
const CONNECTION_QUEUE_CAPACITY: usize = 16;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_PATH_BYTES: usize = 2 * 1024;
const MAX_DEVELOPER_CONTROL_BYTES: usize = 16 * 1024;
const DEVELOPER_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTROL_BYTES: usize = 64 * 1024;
const MAX_DICTATION_SOURCE_BYTES: usize = 128;

type HealthProvider = Arc<dyn Fn() -> HealthResponse + Send + Sync>;

pub struct LocalApi {
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    discovery: DiscoveryLease,
    events: EventLog,
    stop_observed: Arc<AtomicBool>,
    _instance: Option<crate::desktop::instance::InstanceLock>,
}

#[derive(Default)]
struct AuthObservationLimiter {
    last: Mutex<Option<Instant>>,
}

struct HttpContext {
    token: Arc<str>,
    health: HealthProvider,
    events: EventLog,
    auth_limiter: AuthObservationLimiter,
    shutdown: Arc<AtomicBool>,
    developer_control: Option<SyncSender<crate::control::developer_control::DeveloperCall>>,
    control: Option<SyncSender<crate::control::ControlCall>>,
    dictations: dictation::Dictations,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    version: &'static str,
    api_version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesResponse {
    service_capture: bool,
    developer_control: bool,
    control: bool,
}

/// Optional control channels the running app exposes through its local API.
#[derive(Default)]
pub struct LocalApiOptions {
    pub developer_control: Option<SyncSender<crate::control::developer_control::DeveloperCall>>,
    /// The `hex` control commands (`POST /control`).
    pub control: Option<SyncSender<crate::control::ControlCall>>,
    /// Running-app dictation (`/dictations`): the macOS recognition loop or
    /// the Linux listener.
    pub dictation_control: Option<dictation::Control>,
}

impl LocalApi {
    /// Start the one app-owned local API and publish its discovery document.
    pub fn start(events: EventLog, options: LocalApiOptions) -> Result<Self> {
        let instance = crate::desktop::instance::acquire("local-api")?;
        Self::start_bound(
            crate::app_paths::local_api_discovery_file()?,
            Arc::new(current_health),
            events,
            options,
            Some(instance),
        )
    }

    /// Start the running Linux or Windows app's local API. Hex runs on
    /// without it, so a failure is only logged.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn start_for_app(
        event_path: &Path,
        control: SyncSender<crate::control::ControlCall>,
        dictation_control: Option<dictation::Control>,
    ) -> Option<Self> {
        let started = EventLog::create(event_path)
            .map_err(Into::into)
            .and_then(|events| {
                Self::start(
                    events,
                    LocalApiOptions {
                        control: Some(control),
                        dictation_control,
                        ..Default::default()
                    },
                )
            });
        started
            .inspect_err(
                |error| tracing::warn!(error = %format!("{error:#}"), "local API is unavailable"),
            )
            .ok()
    }

    fn start_bound(
        discovery_path: PathBuf,
        health: HealthProvider,
        events: EventLog,
        options: LocalApiOptions,
        instance: Option<crate::desktop::instance::InstanceLock>,
    ) -> Result<Self> {
        prepare_discovery_path(&discovery_path)?;
        let listener = TcpListener::bind("127.0.0.1:0").wrap_err("could not bind the local API")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let document = DiscoveryDocument {
            port: address.port(),
            token: generate_token()?,
            api_version: API_VERSION.into(),
            pid: std::process::id(),
        };
        write_discovery(&discovery_path, &document)?;
        let discovery = DiscoveryLease {
            path: discovery_path,
            document,
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let worker_events = events.clone();
        let token = discovery.document.token.clone();
        let worker_discovery = (discovery.path.clone(), discovery.document.clone());
        let stop_observed = Arc::new(AtomicBool::new(false));
        let worker_stop_observed = stop_observed.clone();
        let unexpected_shutdown = worker_shutdown.clone();
        let http_context = HttpContext {
            token: token.into(),
            health,
            events: worker_events.clone(),
            auth_limiter: AuthObservationLimiter::default(),
            shutdown: worker_shutdown,
            developer_control: options.developer_control,
            control: options.control,
            dictations: dictation::Dictations::new(options.dictation_control),
        };
        let (ready, readiness) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("local-api".into())
            .spawn(move || {
                serve(listener, http_context, ready);
                if !unexpected_shutdown.load(Ordering::Acquire) {
                    let (path, document) = worker_discovery;
                    remove_discovery_if_owned(&path, &document);
                    observe_stopped(&worker_events, &worker_stop_observed);
                    tracing::error!("local API stopped unexpectedly");
                }
            })
            .wrap_err("could not start the local API thread")?;
        match readiness.recv_timeout(Duration::from_secs(1)) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                shutdown.store(true, Ordering::Release);
                let _ = worker.join();
                bail!("local API HTTP workers could not start");
            }
        }
        if let Err(error) = events.emit(&VoiceEvent::ApiServerStarted {
            timestamp_ms: now_ms(),
            port: discovery.document.port,
        }) {
            tracing::error!(%error, "could not record local API startup");
        }
        tracing::info!(port = discovery.document.port, "local API started");
        Ok(Self {
            shutdown,
            worker: Some(worker),
            discovery,
            events,
            stop_observed,
            _instance: instance,
        })
    }

    pub fn shutdown(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        // Retract discovery before draining so a replacement Hex can publish
        // while this listener is still bound and finishing in-flight requests.
        self.discovery.remove_if_owned();
        self.shutdown.store(true, Ordering::Release);
        if worker.join().is_err() {
            tracing::error!("local API thread panicked during shutdown");
        }
        observe_stopped(&self.events, &self.stop_observed);
        tracing::info!("local API stopped");
    }
}

impl Drop for LocalApi {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn observe_stopped(events: &EventLog, observed: &AtomicBool) {
    if observed.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Err(error) = events
        .emit(&VoiceEvent::ApiServerStopped {
            timestamp_ms: now_ms(),
        })
        .and_then(|()| events.flush())
    {
        tracing::error!(%error, "could not record local API shutdown");
    }
}

fn serve(listener: TcpListener, context: HttpContext, ready: mpsc::SyncSender<bool>) {
    let (connections, pending) = mpsc::sync_channel(CONNECTION_QUEUE_CAPACITY);
    let pending = Arc::new(Mutex::new(pending));
    let shutdown = context.shutdown.clone();
    let context = Arc::new(context);
    let workers = (0..HTTP_WORKERS)
        .filter_map(|index| {
            let pending = pending.clone();
            let context = context.clone();
            thread::Builder::new()
                .name(format!("local-api-http-{index}"))
                .spawn(move || http_worker(pending, context))
                .map_err(|error| {
                    tracing::error!(%error, "could not start local API HTTP worker");
                })
                .ok()
        })
        .collect::<Vec<_>>();
    if workers.is_empty() {
        let _ = ready.send(false);
        tracing::error!("local API has no HTTP workers");
        return;
    }
    if ready.send(true).is_err() {
        return;
    }

    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => match connections.try_send(stream) {
                Ok(()) => {}
                Err(TrySendError::Full(mut stream)) => {
                    let _ = write_response(&mut stream, HttpResponse::empty(503));
                }
                Err(TrySendError::Disconnected(_)) => break,
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                #[cfg(unix)]
                {
                    use std::os::fd::AsRawFd;
                    let mut pfd = libc::pollfd {
                        fd: listener.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // Waking on the next connection instead of sleeping took
                    // a loopback request from 12.5 ms to 0.12 ms.
                    // SAFETY: `pfd` is one valid pollfd for the listener's fd.
                    unsafe {
                        libc::poll(&raw mut pfd, 1, ACCEPT_INTERVAL.as_millis() as libc::c_int);
                    }
                }
                #[cfg(not(unix))]
                thread::sleep(ACCEPT_INTERVAL);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                tracing::error!(%error, "local API accept failed");
                thread::sleep(ACCEPT_INTERVAL);
            }
        }
    }
    drop(connections);
    for worker in workers {
        if worker.join().is_err() {
            tracing::error!("local API HTTP worker panicked during shutdown");
        }
    }
}

fn http_worker(pending: Arc<Mutex<Receiver<TcpStream>>>, context: Arc<HttpContext>) {
    loop {
        let stream = pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recv();
        let Ok(mut stream) = stream else {
            return;
        };
        let _ = stream.set_nodelay(true);
        if let Err(error) = stream.set_read_timeout(Some(CONNECTION_TIMEOUT)) {
            tracing::debug!(%error, "could not set local API read timeout");
            continue;
        }
        if let Err(error) = stream.set_write_timeout(Some(CONNECTION_TIMEOUT)) {
            tracing::debug!(%error, "could not set local API write timeout");
            continue;
        }
        let action = match read_request(&mut stream, &context.shutdown) {
            Ok(request) => handle_request(request, &context),
            Err(RequestReadError::TooLarge) => RequestAction::Respond(HttpResponse::empty(431)),
            Err(RequestReadError::Invalid) => RequestAction::Respond(HttpResponse::empty(400)),
            Err(RequestReadError::Deadline) => RequestAction::Respond(HttpResponse::empty(408)),
            Err(RequestReadError::Shutdown) => return,
            Err(RequestReadError::Io(error)) => {
                tracing::debug!(%error, "local API client disconnected while sending a request");
                continue;
            }
        };
        match action {
            RequestAction::Respond(response) => {
                if let Err(error) = write_response(&mut stream, response) {
                    tracing::debug!(%error, "local API client disconnected");
                }
            }
            RequestAction::DeveloperControl {
                content_length,
                body_prefix,
            } => {
                let response = developer_control(
                    &mut stream,
                    content_length,
                    body_prefix,
                    context.developer_control.as_ref(),
                    &context.shutdown,
                );
                if let Err(error) = write_response(&mut stream, response) {
                    tracing::debug!(%error, "developer control client disconnected");
                }
            }
            RequestAction::Control {
                content_length,
                body_prefix,
            } => {
                let response = control(
                    &mut stream,
                    content_length,
                    body_prefix,
                    context.control.as_ref(),
                    &context.shutdown,
                );
                if let Err(error) = write_response(&mut stream, response) {
                    tracing::debug!(%error, "control client disconnected");
                }
            }
            RequestAction::DictationStart {
                content_length,
                body_prefix,
            } => {
                let response = dictation::start(&mut stream, content_length, body_prefix, &context);
                if let Err(error) = write_response(&mut stream, response) {
                    tracing::debug!(%error, "dictation client disconnected");
                }
            }
            RequestAction::Dictation {
                id,
                owner_token,
                action,
            } => {
                dictation::handle(&mut stream, id, owner_token, action, &context);
            }
        }
    }
}

fn handle_request(request: HttpRequest, context: &HttpContext) -> RequestAction {
    let path = request.path.split('?').next().unwrap_or("/");
    if request.authorization.as_deref() != Some(&format!("Bearer {}", context.token)) {
        context
            .auth_limiter
            .observe(&context.events, &request.method, path);
        return RequestAction::Respond(HttpResponse::empty(401));
    }
    if request.method == "POST" && path == "/dev/control" {
        return developer_control_action(request, context.developer_control.is_some());
    }
    if request.method == "POST" && path == "/control" {
        return json_post_action(
            request,
            context.control.is_some(),
            "control-unavailable",
            MAX_CONTROL_BYTES,
            "control-too-large",
            |content_length, body_prefix| RequestAction::Control {
                content_length,
                body_prefix,
            },
        );
    }
    if request.method == "POST" && path == "/dictations" {
        return dictation_start_action(request, context.dictations.available());
    }
    if let Some((id, action)) = dictation_path(path) {
        let Some(owner_token) = request.dictation_token else {
            return RequestAction::Respond(HttpResponse::code(404, "dictation-not-found"));
        };
        let action = match (request.method.as_str(), action) {
            ("POST", "finish") => Some(DictationAction::Finish),
            ("POST", "cancel") => Some(DictationAction::Cancel),
            ("GET", "levels") => Some(DictationAction::Levels),
            ("GET", "audio") => Some(DictationAction::Audio),
            ("POST", "heartbeat") => Some(DictationAction::Heartbeat),
            _ => None,
        };
        return match action {
            Some(action) if context.dictations.available() => RequestAction::Dictation {
                id,
                owner_token,
                action,
            },
            _ => RequestAction::Respond(HttpResponse::code(404, "service-capture-unavailable")),
        };
    }

    let response = match (request.method.as_str(), path) {
        ("GET", "/health") => HttpResponse::json(200, &(context.health)()),
        ("GET", "/capabilities") => HttpResponse::json(
            200,
            &CapabilitiesResponse {
                service_capture: context.dictations.available(),
                developer_control: context.developer_control.is_some(),
                control: context.control.is_some(),
            },
        ),
        ("GET", "/profile/vitals") => match procwatch::snapshot() {
            Some(snapshot) => HttpResponse {
                status: 200,
                reason: reason(200),
                content_type: Some("application/json"),
                body: snapshot.to_json().into_bytes(),
            },
            None => HttpResponse::code(503, "vitals-unavailable"),
        },
        _ => HttpResponse::code(404, "not_found"),
    };
    RequestAction::Respond(response)
}

fn developer_control_action(request: HttpRequest, available: bool) -> RequestAction {
    json_post_action(
        request,
        available,
        "developer-control-unavailable",
        MAX_DEVELOPER_CONTROL_BYTES,
        "developer-control-too-large",
        |content_length, body_prefix| RequestAction::DeveloperControl {
            content_length,
            body_prefix,
        },
    )
}

fn dictation_start_action(request: HttpRequest, available: bool) -> RequestAction {
    json_post_action(
        request,
        available,
        "service-capture-unavailable",
        MAX_DICTATION_SOURCE_BYTES + 32,
        "source-too-large",
        |content_length, body_prefix| RequestAction::DictationStart {
            content_length,
            body_prefix,
        },
    )
}

fn json_post_action(
    request: HttpRequest,
    available: bool,
    unavailable_code: &'static str,
    max_bytes: usize,
    too_large_code: &'static str,
    build: impl FnOnce(usize, Vec<u8>) -> RequestAction,
) -> RequestAction {
    if !available {
        return RequestAction::Respond(HttpResponse::code(404, unavailable_code));
    }
    if !request.has_content_type("application/json") {
        return RequestAction::Respond(HttpResponse::code(415, "unsupported-content-type"));
    }
    let Some(content_length) = request.content_length else {
        return RequestAction::Respond(HttpResponse::code(411, "length-required"));
    };
    if content_length > max_bytes {
        return RequestAction::Respond(HttpResponse::code(413, too_large_code));
    }
    build(content_length, request.body_prefix)
}

fn dictation_path(path: &str) -> Option<(u64, &str)> {
    let mut parts = path.strip_prefix("/dictations/")?.split('/');
    let id = parts.next()?.parse().ok()?;
    let action = parts.next()?;
    (parts.next().is_none()).then_some((id, action))
}

fn developer_control(
    stream: &mut TcpStream,
    content_length: usize,
    mut body: Vec<u8>,
    controls: Option<&SyncSender<crate::control::developer_control::DeveloperCall>>,
    shutdown: &AtomicBool,
) -> HttpResponse {
    if let Err(error) = read_body_bounded(
        stream,
        content_length,
        &mut body,
        shutdown,
        MAX_DEVELOPER_CONTROL_BYTES,
        DEVELOPER_CONTROL_TIMEOUT,
    ) {
        return body_read_response(error);
    }
    let Ok(command) = serde_json::from_slice(&body) else {
        return HttpResponse::code(400, "invalid-developer-command");
    };
    let Some(controls) = controls else {
        return HttpResponse::code(404, "developer-control-unavailable");
    };
    let (reply, response) = mpsc::sync_channel(1);
    match controls.try_send(crate::control::developer_control::DeveloperCall { command, reply }) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            return HttpResponse::code(503, "developer-control-busy");
        }
        Err(TrySendError::Disconnected(_)) => {
            return HttpResponse::code(503, "developer-control-unavailable");
        }
    }
    match response.recv_timeout(DEVELOPER_CONTROL_TIMEOUT) {
        Ok(reply) => HttpResponse::json(200, &reply),
        Err(RecvTimeoutError::Timeout) => HttpResponse::code(504, "developer-ui-timeout"),
        Err(RecvTimeoutError::Disconnected) => {
            HttpResponse::code(503, "developer-control-unavailable")
        }
    }
}

/// Hands one `hex` control request to the app and waits for its reply.
/// Refusals travel as a 200 `ControlReply::Error`; an HTTP error means the
/// request never reached the app.
fn control(
    stream: &mut TcpStream,
    content_length: usize,
    mut body: Vec<u8>,
    controls: Option<&SyncSender<crate::control::ControlCall>>,
    shutdown: &AtomicBool,
) -> HttpResponse {
    use crate::control::{ControlCall, ControlReply, ControlRequest, ControlResponse};

    if let Err(error) = read_body_bounded(
        stream,
        content_length,
        &mut body,
        shutdown,
        MAX_CONTROL_BYTES,
        DEVELOPER_CONTROL_TIMEOUT,
    ) {
        return body_read_response(error);
    }
    let request = match serde_json::from_slice::<ControlRequest>(&body) {
        Ok(request) => request,
        // A newer `hex` may name a command this app doesn't know.
        Err(_) => match serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("version")?.as_u64())
        {
            Some(version) if version != u64::from(crate::control::CONTROL_VERSION) => {
                let error = crate::control::version_mismatch(version);
                return HttpResponse::json(200, &ControlResponse::from(ControlReply::from(error)));
            }
            _ => return HttpResponse::code(400, "invalid-control-command"),
        },
    };
    if let Err(error) = crate::control::check_version(&request) {
        return HttpResponse::json(200, &ControlResponse::from(ControlReply::from(error)));
    }
    let Some(controls) = controls else {
        return HttpResponse::code(404, "control-unavailable");
    };
    let deadline = request.command.deadline();
    let (reply, response) = mpsc::sync_channel(1);
    match controls.try_send(ControlCall { request, reply }) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => return HttpResponse::code(503, "control-busy"),
        Err(TrySendError::Disconnected(_)) => {
            return HttpResponse::code(503, "control-unavailable");
        }
    }
    match response.recv_timeout(deadline) {
        Ok(reply) => HttpResponse::json(200, &reply),
        Err(RecvTimeoutError::Timeout) => HttpResponse::code(504, "control-timeout"),
        Err(RecvTimeoutError::Disconnected) => HttpResponse::code(503, "control-unavailable"),
    }
}

impl AuthObservationLimiter {
    fn observe(&self, events: &EventLog, method: &str, path: &str) {
        let now = Instant::now();
        let mut last = self.last.lock().unwrap_or_else(|error| error.into_inner());
        if last.is_some_and(|last| now.duration_since(last) < AUTH_OBSERVATION_INTERVAL) {
            return;
        }
        *last = Some(now);
        drop(last);
        if let Err(error) = events.emit(&VoiceEvent::ApiAuthFailed {
            timestamp_ms: now_ms(),
            method: method.into(),
            path: path.into(),
        }) {
            tracing::error!(%error, "could not record local API authentication failure");
        }
    }
}

enum RequestAction {
    Respond(HttpResponse),
    DeveloperControl {
        content_length: usize,
        body_prefix: Vec<u8>,
    },
    Control {
        content_length: usize,
        body_prefix: Vec<u8>,
    },
    DictationStart {
        content_length: usize,
        body_prefix: Vec<u8>,
    },
    Dictation {
        id: u64,
        owner_token: String,
        action: DictationAction,
    },
}

#[derive(Clone, Copy, Debug)]
enum DictationAction {
    Finish,
    Cancel,
    Levels,
    Audio,
    Heartbeat,
}

fn current_health() -> HealthResponse {
    HealthResponse {
        version: crate::VERSION,
        api_version: API_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::dictation::service::DictationControl;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    /// How long a test waits on another thread before failing instead of hanging.
    const TEST_WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn authenticated_server_reports_live_health_and_cleans_up_discovery() {
        let directory = temp_directory();
        let discovery_path = directory.join("local-api.json");
        let event_path = directory.join("events.ndjson");
        let events = EventLog::create(&event_path).unwrap();
        let mut api = start_published(discovery_path.clone(), events).unwrap();
        let document: DiscoveryDocument =
            serde_json::from_slice(&fs::read(&discovery_path).unwrap()).unwrap();

        let unauthorized = request(document.port, None, "/health");
        assert!(unauthorized.starts_with("HTTP/1.1 401"));
        assert_eq!(response_body(&unauthorized), "");
        let wrong_token = request(document.port, Some("hex_wrong"), "/health");
        assert!(wrong_token.starts_with("HTTP/1.1 401"));
        let preflight = raw_request(
            document.port,
            "OPTIONS /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        );
        assert!(preflight.starts_with("HTTP/1.1 401"));

        let first = request(document.port, Some(&document.token), "/health");
        assert!(first.starts_with("HTTP/1.1 200"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(response_body(&first)).unwrap()["apiVersion"],
            "2"
        );
        let capabilities = request(document.port, Some(&document.token), "/capabilities");
        let capabilities: serde_json::Value =
            serde_json::from_str(response_body(&capabilities)).unwrap();
        assert_eq!(
            capabilities,
            serde_json::json!({
                "serviceCapture": false,
                "developerControl": false,
                "control": false,
            })
        );
        let vitals = request(document.port, Some(&document.token), "/profile/vitals");
        #[cfg(target_os = "macos")]
        {
            assert!(vitals.starts_with("HTTP/1.1 200"));
            let vitals: serde_json::Value = serde_json::from_str(response_body(&vitals)).unwrap();
            assert!(vitals["footprint"].as_u64().unwrap() > 0);
            assert!(vitals["heap"]["live"].as_u64().unwrap() > 0);
        }
        // procwatch reads a process's footprint only on macOS.
        #[cfg(not(target_os = "macos"))]
        assert!(vitals.starts_with("HTTP/1.1 503"));

        api.shutdown();
        assert!(!discovery_path.exists());
        let observations = fs::read_to_string(event_path).unwrap();
        assert!(observations.contains("api_server_started"));
        assert!(observations.contains("api_auth_failed"));
        assert_eq!(observations.matches("api_auth_failed").count(), 1);
        assert!(observations.contains("api_server_stopped"));
        assert!(!observations.contains(&document.token));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn authenticated_developer_control_is_bounded_and_typed() {
        let directory = temp_directory();
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let (sender, controls) = mpsc::sync_channel(1);
        let mut api = start_with(
            directory.join("local-api.json"),
            events,
            LocalApiOptions {
                developer_control: Some(sender),
                ..Default::default()
            },
        )
        .unwrap();
        let port = api.discovery.document.port;
        let token = api.discovery.document.token.clone();
        let client = thread::spawn(move || {
            let body = r#"{"command":"hud","state":"recording"}"#;
            raw_request(
                port,
                &format!(
                    "POST /dev/control HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ),
            )
        });

        let call = controls.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            call.command,
            crate::control::developer_control::DeveloperCommand::Hud {
                state: crate::control::developer_control::DeveloperHudState::Recording
            }
        ));
        call.reply
            .send(crate::control::developer_control::DeveloperReply::Ok)
            .unwrap();
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert_eq!(response_body(&response), r#"{"status":"ok"}"#);

        api.shutdown();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn control_requests_are_authenticated_versioned_and_typed() {
        use crate::control::{ControlCommand, ControlReply, ControlResponse};

        let directory = temp_directory();
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let (sender, controls) = mpsc::sync_channel(1);
        let mut api = start_with(
            directory.join("local-api.json"),
            events,
            LocalApiOptions {
                control: Some(sender),
                ..Default::default()
            },
        )
        .unwrap();
        let port = api.discovery.document.port;
        let token = api.discovery.document.token.clone();
        fn post_to(port: u16, token: &str, body: &str) -> String {
            raw_request(
                port,
                &format!(
                    "POST /control HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ),
            )
        }
        let post = |token: &str, body: &str| post_to(port, token, body);

        let client_token = token.clone();
        let client = thread::spawn(move || {
            post_to(
                port,
                &client_token,
                r#"{"version":7,"command":"dictation","action":"start"}"#,
            )
        });
        let call = controls.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            call.request.command,
            ControlCommand::Dictation {
                action: crate::control::CaptureAction::Start,
                print: false,
                raw: false,
            }
        );
        call.reply
            .send(ControlReply::done("Started recording").into())
            .unwrap();
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert_eq!(
            serde_json::from_str::<ControlResponse>(response_body(&response))
                .unwrap()
                .reply,
            ControlReply::done("Started recording")
        );

        let unauthorized = post("hex_wrong", r#"{"version":7,"command":"status"}"#);
        assert!(unauthorized.starts_with("HTTP/1.1 401"));
        let unknown = post(&token, r#"{"version":7,"command":"explode"}"#);
        assert!(unknown.starts_with("HTTP/1.1 400"));
        // Another protocol version is answered without reaching the app.
        for body in [
            r#"{"version":1,"command":"status"}"#,
            r#"{"version":1,"command":"explode"}"#,
        ] {
            let mismatch = post(&token, body);
            let reply = serde_json::from_str::<ControlResponse>(response_body(&mismatch))
                .unwrap()
                .reply;
            assert!(
                matches!(&reply, ControlReply::Error { code, .. } if code == "version-mismatch"),
                "{reply:?}"
            );
        }
        let too_large = raw_request(
            port,
            &format!(
                "POST /control HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_CONTROL_BYTES + 1
            ),
        );
        assert!(too_large.starts_with("HTTP/1.1 413"));
        assert!(controls.try_recv().is_err());

        api.shutdown();
        fs::remove_dir_all(directory).unwrap();
    }

    /// The next `/dictations` control the capture loop receives.
    #[cfg(target_os = "macos")]
    fn next_dictation_control(
        controls: &mpsc::Receiver<crate::recognition::RecognitionControl>,
    ) -> DictationControl {
        match controls.recv_timeout(TEST_WAIT).unwrap() {
            crate::recognition::RecognitionControl::Dictation(control) => control,
            control => panic!("expected a dictation control, got {control:?}"),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn next_dictation_control(controls: &mpsc::Receiver<DictationControl>) -> DictationControl {
        controls.recv_timeout(TEST_WAIT).unwrap()
    }

    #[test]
    fn authenticated_running_app_dictation_streams_observations_and_returns_raw_text() {
        let directory = temp_directory();
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let (sender, controls) = mpsc::sync_channel(4);
        let mut api = start_with(
            directory.join("local-api.json"),
            events,
            LocalApiOptions {
                dictation_control: Some(sender),
                ..Default::default()
            },
        )
        .unwrap();
        let port = api.discovery.document.port;
        let token = api.discovery.document.token.clone();
        let (attached, audio_attached) = mpsc::channel();
        let controller = thread::spawn(move || {
            let DictationControl::Start {
                source,
                owner_token,
                levels,
                reply,
            } = next_dictation_control(&controls)
            else {
                panic!("expected dictation start")
            };
            assert_eq!(source, "tp7");
            assert!(owner_token.starts_with("hex_"));
            reply
                .send(Ok(crate::dictation::service::ProgrammaticDictationStart {
                    id: 7,
                    sample_rate: 48_000,
                }))
                .unwrap();
            levels
                .send(crate::dictation::service::DictationLevel {
                    rms_db: -24.5,
                    peak_db: -8.0,
                })
                .unwrap();

            let DictationControl::Heartbeat {
                id,
                owner_token: heartbeat_token,
                reply,
            } = next_dictation_control(&controls)
            else {
                panic!("expected dictation heartbeat")
            };
            assert_eq!(id, 7);
            assert_eq!(heartbeat_token, owner_token);
            reply.send(Ok(())).unwrap();

            let DictationControl::AttachAudio {
                id,
                owner_token: audio_token,
                audio,
                reply,
            } = next_dictation_control(&controls)
            else {
                panic!("expected dictation audio subscription")
            };
            assert_eq!(id, 7);
            assert_eq!(audio_token, owner_token);
            reply.send(Ok(())).unwrap();
            audio
                .send(crate::dictation::service::DictationAudioChunk {
                    samples: vec![0.25, -0.5],
                })
                .unwrap();
            attached.send(()).unwrap();

            let DictationControl::Finish {
                id,
                owner_token: finish_token,
                reply,
            } = next_dictation_control(&controls)
            else {
                panic!("expected dictation finish")
            };
            assert_eq!(id, 7);
            assert_eq!(finish_token, owner_token);
            reply
                .send(Ok(crate::dictation::service::ProgrammaticDictationResult {
                    transcript: "raw words".into(),
                    duration_ms: 875,
                }))
                .unwrap();
            drop(levels);
            drop(audio);
        });

        let body = r#"{"source":"tp7"}"#;
        let start = raw_request(
            port,
            &format!(
                "POST /dictations HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(start.starts_with("HTTP/1.1 201"));
        let started = serde_json::from_str::<serde_json::Value>(response_body(&start)).unwrap();
        assert_eq!(started["id"], 7);
        assert_eq!(started["sampleRate"], 48_000);
        let owner_token = started["ownerToken"].as_str().unwrap().to_owned();
        assert!(owner_token.starts_with("hex_"));

        let missing_owner = request_with_method(port, Some(&token), "POST", "/dictations/7/finish");
        assert!(missing_owner.starts_with("HTTP/1.1 404"));

        let heartbeat = request_with_dictation_token(
            port,
            &token,
            &owner_token,
            "POST",
            "/dictations/7/heartbeat",
        );
        assert!(heartbeat.starts_with("HTTP/1.1 204"));

        let wrong_levels =
            request_with_dictation_token(port, &token, "hex_wrong", "GET", "/dictations/7/levels");
        assert!(wrong_levels.starts_with("HTTP/1.1 404"));

        let level_token = token.clone();
        let level_owner = owner_token.clone();
        let (subscribed, levels_subscribed) = mpsc::channel();
        let levels = thread::spawn(move || {
            // The level sent at start arrives once the stream has subscribed.
            raw_request_signalling(
                port,
                &format!(
                    "GET /dictations/7/levels HTTP/1.1\r\nAuthorization: Bearer {level_token}\r\nX-Hex-Dictation-Token: {level_owner}\r\nConnection: close\r\n\r\n"
                ),
                b"rmsDb",
                subscribed,
            )
        });
        let audio_token = token.clone();
        let audio_owner = owner_token.clone();
        let audio = thread::spawn(move || {
            request_bytes(port, &audio_token, &audio_owner, "/dictations/7/audio")
        });
        levels_subscribed.recv_timeout(TEST_WAIT).unwrap();
        audio_attached.recv_timeout(TEST_WAIT).unwrap();
        let finish = request_with_dictation_token(
            port,
            &token,
            &owner_token,
            "POST",
            "/dictations/7/finish",
        );
        assert!(finish.starts_with("HTTP/1.1 200"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(response_body(&finish)).unwrap(),
            serde_json::json!({ "transcript": "raw words", "durationMs": 875 })
        );
        let levels = String::from_utf8(levels.join().unwrap()).unwrap();
        assert!(levels.starts_with("HTTP/1.1 200"));
        assert!(levels.contains(r#""rmsDb":-24.5"#));
        assert!(levels.contains(r#""peakDb":-8.0"#));
        let audio = audio.join().unwrap();
        let body = audio
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .map(|index| &audio[index + 4..])
            .unwrap();
        assert_eq!(
            body,
            [0.25_f32.to_le_bytes(), (-0.5_f32).to_le_bytes()].concat()
        );

        controller.join().unwrap();
        api.shutdown();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discovery_is_owner_only_and_stale_files_are_replaced() {
        let directory = temp_directory();
        let discovery_path = directory.join("local-api.json");
        fs::write(&discovery_path, b"not json").unwrap();
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let api = start_published(discovery_path.clone(), events).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&discovery_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        drop(api);
        assert!(!discovery_path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn current_pid_with_a_closed_port_is_stale() {
        let directory = temp_directory();
        let discovery_path = directory.join("local-api.json");
        let stale = DiscoveryDocument {
            port: unused_port(),
            token: "hex_0000000000000000000000000000000000000000000000000000000000000000".into(),
            api_version: API_VERSION.into(),
            pid: std::process::id(),
        };
        write_discovery(&discovery_path, &stale).unwrap();
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let api = start_published(discovery_path.clone(), events).unwrap();
        let current: DiscoveryDocument =
            serde_json::from_slice(&fs::read(&discovery_path).unwrap()).unwrap();
        assert_ne!(current.token, stale.token);
        drop(api);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn old_server_does_not_remove_newer_discovery_generation() {
        let directory = temp_directory();
        let discovery_path = directory.join("local-api.json");
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let api = start_published(discovery_path.clone(), events).unwrap();
        let replacement = DiscoveryDocument {
            port: 1234,
            token: "hex_replacement".into(),
            api_version: API_VERSION.into(),
            pid: std::process::id(),
        };
        write_discovery(&discovery_path, &replacement).unwrap();
        drop(api);
        assert_eq!(
            serde_json::from_slice::<DiscoveryDocument>(&fs::read(&discovery_path).unwrap())
                .unwrap(),
            replacement
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn live_discovery_cannot_be_replaced() {
        let directory = temp_directory();
        let discovery_path = directory.join("local-api.json");
        let events = EventLog::create(&directory.join("events.ndjson")).unwrap();
        let api = start_published(discovery_path.clone(), events.clone()).unwrap();
        assert!(start_published(discovery_path.clone(), events).is_err());
        drop(api);
        assert!(!discovery_path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn oversized_headers_are_rejected_without_reaching_auth() {
        let directory = temp_directory();
        let discovery_path = directory.join("local-api.json");
        let event_path = directory.join("events.ndjson");
        let events = EventLog::create(&event_path).unwrap();
        let api = start_published(discovery_path.clone(), events).unwrap();
        let document: DiscoveryDocument =
            serde_json::from_slice(&fs::read(&discovery_path).unwrap()).unwrap();
        let response = raw_request(
            document.port,
            &format!(
                "GET /health HTTP/1.1\r\nX-Oversized: {}\r\n\r\n",
                "a".repeat(MAX_HEADER_BYTES)
            ),
        );
        assert!(response.starts_with("HTTP/1.1 431"));
        assert!(
            !fs::read_to_string(&event_path)
                .unwrap()
                .contains("api_auth_failed")
        );
        drop(api);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn request_parser_preserves_bounded_body_bytes_and_metadata() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .write_all(
                    b"POST /control HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 3\r\n\r\nabc",
                )
                .unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();

        let request = read_request(&mut stream, &AtomicBool::new(false)).unwrap();

        client.join().unwrap();
        assert_eq!(request.content_type.as_deref(), Some("application/json"));
        assert_eq!(request.content_length, Some(3));
        assert_eq!(request.body_prefix, b"abc");
    }

    #[test]
    fn request_parser_observes_absolute_deadline_and_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let first_client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut first, _) = listener.accept().unwrap();
        assert!(matches!(
            read_request_until(&mut first, &AtomicBool::new(false), Instant::now()),
            Err(RequestReadError::Deadline)
        ));
        drop(first_client);

        let second_client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut second, _) = listener.accept().unwrap();
        assert!(matches!(
            read_request(&mut second, &AtomicBool::new(true)),
            Err(RequestReadError::Shutdown)
        ));
        drop(second_client);
    }

    #[test]
    fn tokens_have_256_bits_of_random_lowercase_hex() {
        let first = generate_token().unwrap();
        let second = generate_token().unwrap();
        assert!(valid_token(&first));
        assert_ne!(first, second);
    }

    fn start_published(discovery_path: PathBuf, events: EventLog) -> Result<LocalApi> {
        start_with(discovery_path, events, LocalApiOptions::default())
    }

    fn start_with(
        discovery_path: PathBuf,
        events: EventLog,
        options: LocalApiOptions,
    ) -> Result<LocalApi> {
        LocalApi::start_bound(discovery_path, Arc::new(test_health), events, options, None)
    }

    fn test_health() -> HealthResponse {
        HealthResponse {
            version: "test",
            api_version: API_VERSION,
        }
    }

    fn request(port: u16, token: Option<&str>, path: &str) -> String {
        request_with_method(port, token, "GET", path)
    }

    fn request_with_method(port: u16, token: Option<&str>, method: &str, path: &str) -> String {
        let authorization = token.map_or_else(String::new, |token| {
            format!("Authorization: Bearer {token}\r\n")
        });
        raw_request(
            port,
            &format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{authorization}Connection: close\r\n\r\n"
            ),
        )
    }

    fn raw_request(port: u16, request: &str) -> String {
        String::from_utf8(raw_request_bytes(port, request)).unwrap()
    }

    fn raw_request_bytes(port: u16, request: &str) -> Vec<u8> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        response
    }

    /// Like `raw_request_bytes`, signalling once the response contains `marker`.
    fn raw_request_signalling(
        port: u16,
        request: &str,
        marker: &[u8],
        ready: mpsc::Sender<()>,
    ) -> Vec<u8> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(TEST_WAIT)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        let mut buffer = [0; 1024];
        let mut signalled = false;
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                return response;
            }
            response.extend_from_slice(&buffer[..read]);
            if !signalled && response.windows(marker.len()).any(|bytes| bytes == marker) {
                signalled = true;
                let _ = ready.send(());
            }
        }
    }

    fn request_with_dictation_token(
        port: u16,
        token: &str,
        owner_token: &str,
        method: &str,
        path: &str,
    ) -> String {
        raw_request(
            port,
            &format!(
                "{method} {path} HTTP/1.1\r\nAuthorization: Bearer {token}\r\nX-Hex-Dictation-Token: {owner_token}\r\nConnection: close\r\n\r\n"
            ),
        )
    }

    fn request_bytes(port: u16, token: &str, owner_token: &str, path: &str) -> Vec<u8> {
        raw_request_bytes(
            port,
            &format!(
                "GET {path} HTTP/1.1\r\nAuthorization: Bearer {token}\r\nX-Hex-Dictation-Token: {owner_token}\r\nConnection: close\r\n\r\n"
            ),
        )
    }

    fn response_body(response: &str) -> &str {
        response.split_once("\r\n\r\n").unwrap().1
    }

    fn temp_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hex-local-api-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn unused_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }
}
