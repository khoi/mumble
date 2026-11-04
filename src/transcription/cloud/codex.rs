//! Codex's dictation service as Codex Desktop reaches it: the ChatGPT
//! sign-in Codex saved in `auth.json`, with Codex Desktop's identity, which
//! is the only one it answers. A recording streams to ChatGPT's dictation
//! WebSocket while it records; a whole clip is posted as a WAV to its
//! transcribe endpoint.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;

use super::CloudError;
use super::socket::{self, Event, Socket};
use super::stream::{Audio, joined, publish};
use crate::transcription::engine::Partials;

const URL: &str = "https://chatgpt.com/backend-api/transcribe";
const STREAM_HOST: &str = "chatgpt.com";
const STREAM_URL: &str =
    "wss://chatgpt.com/backend-api/dictation/stream?dictation_surface=composer";
const PROTOCOLS: &str = "chatgpt-dictation, codex-desktop";
/// Codex Desktop reports `app.getVersion()`; track a currently shipping
/// ChatGPT.app version (bump with it).
const DESKTOP_VERSION: &str = "26.1007.71326";
/// How long a new session may take to say it started.
const SESSION_START: Duration = Duration::from_secs(15);
/// How long a closing session may take to send its last words.
const SESSION_CLOSE: Duration = Duration::from_secs(15);
/// A session lives 300 s: a new one takes over at a pause after 240 s, and
/// at 285 s regardless.
const ROTATE_AFTER: Duration = Duration::from_secs(240);
const FORCE_ROTATE: Duration = Duration::from_secs(285);
/// The most audio one `audio.append` carries: one second.
const APPEND_SAMPLES: usize = 16_000;
/// A token this close to expiring reads as expired.
const EXPIRY_MARGIN_MS: f64 = 60_000.0;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

/// The clip's text, or why there is none, by `deadline`.
pub(super) fn transcribe(
    clip: &[f32],
    language: &str,
    deadline: Instant,
) -> Result<String, CloudError> {
    let login = saved_login()?;
    let boundary = boundary();
    let body = multipart(&boundary, &wav(clip)?, language_hint(language));
    let agent = crate::net::agent(URL, |config| {
        config
            .http_status_as_error(false)
            .timeout_global(Some(deadline.saturating_duration_since(Instant::now())))
    });
    let mut response = agent
        .post(URL)
        .header("authorization", format!("Bearer {}", login.token))
        .header("chatgpt-account-id", &login.account)
        .header("originator", "Codex Desktop")
        .header("user-agent", user_agent())
        .header("accept", "application/json")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .send(&body[..])
        .map_err(|error| CloudError::Failed(crate::net::unreachable(URL, &error)))?;
    let status = response.status().as_u16();
    let mitigated = response.headers().contains_key("cf-mitigated");
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()
        .unwrap_or_default();
    answer(status, mitigated, &body)
}

/// Sends `audio` as it arrives, one dictation session after another, and
/// returns their text once the last one closes after the end.
pub(super) fn stream(
    audio: &Audio,
    language: &str,
    partials: Option<&Partials>,
) -> Result<String, CloudError> {
    let login = saved_login()?;
    let mut finished: Vec<String> = Vec::new();
    let mut session = Session::open(&login, language)?;
    loop {
        let taken = audio.take()?;
        for chunk in taken.audio.chunks(APPEND_SAMPLES) {
            if session.due() {
                finished.push(session.close()?);
                session = Session::open(&login, language)?;
            }
            session.append(chunk)?;
        }
        if taken.ended {
            finished.push(session.close()?);
            return Ok(joined(finished.iter().map(String::as_str)));
        }
        if session.poll()? {
            publish(
                partials,
                joined(
                    finished
                        .iter()
                        .map(String::as_str)
                        .chain([session.dictation.text().as_str()]),
                ),
                String::new(),
            );
        }
    }
}

/// One dictation session on its own WebSocket.
struct Session {
    socket: Socket,
    dictation: Dictation,
    /// When the service started it; its lifetime counts from here.
    started: Instant,
}

impl Session {
    fn open(login: &Login, language: &str) -> Result<Self, CloudError> {
        let deadline = Instant::now() + SESSION_START;
        let socket = socket::connect(STREAM_HOST, stream_request(login)?, Some(deadline))?;
        let mut session = Self {
            socket,
            dictation: Dictation::default(),
            started: Instant::now(),
        };
        socket::send(&mut session.socket, Message::text(session_start(language)))?;
        while !session.dictation.started {
            session.poll()?;
        }
        socket::limit(&mut session.socket, None);
        session.started = Instant::now();
        Ok(session)
    }

    fn due(&self) -> bool {
        rotation_due(self.started.elapsed(), self.dictation.speaking)
    }

    fn append(&mut self, audio: &[f32]) -> Result<(), CloudError> {
        let pcm: Vec<u8> = super::pcm16(audio).flat_map(i16::to_le_bytes).collect();
        let frame = serde_json::json!({
            "type": "audio.append",
            "audio": data_encoding::BASE64.encode(&pcm),
        });
        socket::send(&mut self.socket, Message::text(frame.to_string()))
    }

    /// Reads one event; true when the text changed.
    fn poll(&mut self) -> Result<bool, CloudError> {
        match socket::next(&mut self.socket)? {
            Event::Text(frame) => self.dictation.frame(&frame),
            Event::Closed(_) => Err(CloudError::Failed(
                "Codex closed the session before it finished".into(),
            )),
            Event::Idle => Ok(false),
        }
    }

    /// Ends the audio and waits for the session's last words.
    fn close(mut self) -> Result<String, CloudError> {
        socket::send(
            &mut self.socket,
            Message::text(r#"{"type":"session.close"}"#),
        )?;
        socket::limit(&mut self.socket, Some(Instant::now() + SESSION_CLOSE));
        while !self.dictation.closed {
            self.poll()?;
        }
        Ok(self.dictation.text())
    }
}

/// Whether a session started `age` ago should hand over to a new one
/// before more audio.
fn rotation_due(age: Duration, speaking: bool) -> bool {
    age >= FORCE_ROTATE || (age >= ROTATE_AFTER && !speaking)
}

/// A session's state from its frames: each utterance's newest final text,
/// in the order the utterances began.
#[derive(Default)]
struct Dictation {
    started: bool,
    speaking: bool,
    closed: bool,
    finals: Vec<(String, u64, String)>,
}

impl Dictation {
    /// Takes one frame; true when the text changed.
    fn frame(&mut self, data: &str) -> Result<bool, CloudError> {
        #[derive(Deserialize)]
        struct Frame {
            #[serde(rename = "type")]
            kind: String,
            utterance_id: Option<String>,
            revision: Option<u64>,
            text: Option<String>,
            fatal: Option<bool>,
            session: Option<SessionState>,
        }
        #[derive(Deserialize)]
        struct SessionState {
            status: Option<String>,
        }
        let Ok(frame) = serde_json::from_str::<Frame>(data) else {
            return Ok(false);
        };
        match frame.kind.as_str() {
            "session.started" => self.started = true,
            "speech.started" => self.speaking = true,
            "speech.stopped" => self.speaking = false,
            "transcript.final" => {
                let Some(id) = frame.utterance_id else {
                    return Ok(false);
                };
                let revision = frame.revision.unwrap_or_default();
                let text = frame.text.unwrap_or_default();
                match self.finals.iter_mut().find(|(known, ..)| *known == id) {
                    Some((_, newest, _)) if *newest > revision => return Ok(false),
                    Some(utterance) => *utterance = (id, revision, text),
                    None => self.finals.push((id, revision, text)),
                }
                return Ok(true);
            }
            "transcript.failed" => {
                return Err(CloudError::Failed(
                    "Codex couldn't transcribe the audio".into(),
                ));
            }
            "session.error" if frame.fatal == Some(true) => {
                return Err(CloudError::Failed("Codex ended the session".into()));
            }
            "session.updated"
                if frame.session.and_then(|session| session.status).as_deref()
                    == Some("closed") =>
            {
                self.closed = true;
            }
            _ => {}
        }
        Ok(false)
    }

    fn text(&self) -> String {
        joined(self.finals.iter().map(|(_, _, text)| text.as_str()))
    }
}

/// Codex Desktop's `session.start`: 16 kHz mono PCM, final text per
/// utterance, and the service's own voice detection.
fn session_start(language: &str) -> String {
    let mut config = serde_json::json!({
        "input_audio_format": "pcm16",
        "sample_rate_hz": 16_000,
        "num_channels": 1,
        "max_buffer_size_bytes": 4_194_304,
        "max_utterance_duration_ms": 30_000,
        "session_ttl_ms": 300_000,
        "provider_mode": "streaming_sse",
        "transcript_delivery_mode": "final_only",
        "vad": {
            "type": "server_vad",
            "threshold": 0.5,
            "prefix_padding_ms": 300,
            "silence_duration_ms": 500,
        },
    });
    if let Some(language) = language_hint(language) {
        config["language"] = language.into();
    }
    serde_json::json!({
        "type": "session.start",
        "dictation_session_id": uuid(),
        "attempt_id": uuid(),
        "config": config,
    })
    .to_string()
}

fn stream_request(login: &Login) -> Result<tungstenite::handshake::client::Request, CloudError> {
    let invalid = |what: &str| CloudError::Failed(format!("Codex: invalid {what}"));
    let mut request = STREAM_URL
        .into_client_request()
        .map_err(|_| invalid("request"))?;
    let headers = request.headers_mut();
    for (name, value) in [
        ("authorization", format!("Bearer {}", login.token)),
        ("chatgpt-account-id", login.account.clone()),
        ("accept-language", crate::desktop::i18n::preferred_locale()),
        ("originator", "Codex Desktop".into()),
        ("user-agent", user_agent()),
        ("sec-websocket-protocol", PROTOCOLS.into()),
    ] {
        headers.insert(
            name,
            HeaderValue::from_str(&value).map_err(|_| invalid(name))?,
        );
    }
    Ok(request)
}

fn user_agent() -> String {
    format!("Codex Desktop/{DESKTOP_VERSION} (Mac OS; arm64)")
}

/// A random version 4 UUID.
fn uuid() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = data_encoding::HEXLOWER.encode(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

struct Login {
    token: String,
    account: String,
}

fn saved_login() -> Result<Login, CloudError> {
    let auth = codex_home()
        .and_then(|home| std::fs::read_to_string(home.join("auth.json")).ok())
        .ok_or(CloudError::SignedOut)?;
    login(&auth, super::now_ms())
}

/// The access token and ChatGPT account in Codex's `auth.json`, unless it
/// expired: Codex renews it, Mumble never does.
fn login(auth: &str, now_ms: f64) -> Result<Login, CloudError> {
    #[derive(Deserialize)]
    struct AuthFile {
        tokens: Option<Tokens>,
    }
    #[derive(Deserialize)]
    struct Tokens {
        access_token: Option<String>,
        account_id: Option<String>,
    }
    #[derive(Deserialize)]
    struct Claims {
        exp: Option<f64>,
        #[serde(rename = "https://api.openai.com/auth")]
        auth: Option<AuthClaims>,
    }
    #[derive(Deserialize)]
    struct AuthClaims {
        chatgpt_account_id: Option<String>,
    }
    let tokens = serde_json::from_str::<AuthFile>(auth)
        .ok()
        .and_then(|file| file.tokens)
        .ok_or(CloudError::SignedOut)?;
    let token = tokens
        .access_token
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .ok_or(CloudError::SignedOut)?;
    let claims = token
        .split('.')
        .nth(1)
        .and_then(|payload| {
            data_encoding::BASE64URL_NOPAD
                .decode(payload.as_bytes())
                .ok()
        })
        .and_then(|json| serde_json::from_slice::<Claims>(&json).ok());
    if let Some(exp) = claims.as_ref().and_then(|claims| claims.exp)
        && exp * 1000.0 - EXPIRY_MARGIN_MS <= now_ms
    {
        return Err(CloudError::SignedOut);
    }
    let account = tokens
        .account_id
        .map(|account| account.trim().to_owned())
        .filter(|account| !account.is_empty())
        .or_else(|| claims?.auth?.chatgpt_account_id)
        .ok_or(CloudError::SignedOut)?;
    Ok(Login { token, account })
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

/// The language to name, if any: Auto leaves it to the service.
fn language_hint(language: &str) -> Option<&str> {
    (matches!(language.len(), 2 | 3) && language.chars().all(|c| c.is_ascii_lowercase()))
        .then_some(language)
}

fn wav(clip: &[f32]) -> Result<Vec<u8>, CloudError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = Cursor::new(Vec::new());
    let failed =
        |error: hound::Error| CloudError::Failed(format!("couldn't encode audio: {error}"));
    let mut writer = hound::WavWriter::new(&mut out, spec).map_err(failed)?;
    for sample in super::pcm16(clip) {
        writer.write_sample(sample).map_err(failed)?;
    }
    writer.finalize().map_err(failed)?;
    Ok(out.into_inner())
}

fn boundary() -> String {
    format!("----codex-transcribe-{}", uuid())
}

/// The form Codex Desktop posts: the WAV as `file`, then the language.
fn multipart(boundary: &str, wav: &[u8], language: Option<&str>) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(b"\r\n");
    if let Some(language) = language {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{language}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// The text in a 200 answer, or why there is none.
fn answer(status: u16, mitigated: bool, body: &str) -> Result<String, CloudError> {
    #[derive(Deserialize)]
    struct Transcribed {
        text: String,
    }
    if mitigated {
        return Err(CloudError::Refused(status));
    }
    let code = |code: &str| body.contains(&format!("\"{code}\""));
    match status {
        200 => serde_json::from_str::<Transcribed>(body)
            .map(|transcribed| transcribed.text)
            .map_err(|_| CloudError::Failed("Codex answered without text".into())),
        429 => Err(CloudError::RateLimited),
        401 => Err(CloudError::SignedOut),
        _ if code("audio_too_short") => Err(CloudError::NoSpeech),
        413 => Err(CloudError::Failed(
            "the dictation is too long for Codex".into(),
        )),
        _ if code("file_above_max_size") => Err(CloudError::Failed(
            "the dictation is too long for Codex".into(),
        )),
        400 | 415 | 422 => Err(CloudError::Failed("Codex couldn't read the audio".into())),
        403 | 404 | 405 | 410 => Err(CloudError::Refused(status)),
        status => Err(CloudError::Failed(format!("Codex answered HTTP {status}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: serde_json::Value) -> String {
        let payload = data_encoding::BASE64URL_NOPAD.encode(claims.to_string().as_bytes());
        format!("e30.{payload}.sig")
    }

    fn auth(token: &str, account: Option<&str>) -> String {
        serde_json::json!({"tokens": {"access_token": token, "account_id": account}}).to_string()
    }

    #[test]
    fn a_saved_chatgpt_sign_in_gives_its_token_and_account() {
        let token = jwt(serde_json::json!({"exp": 2_000}));
        let login = login(&auth(&token, Some("acct-1")), 1_000_000.0).unwrap();
        assert_eq!(login.token, token);
        assert_eq!(login.account, "acct-1");
    }

    #[test]
    fn the_account_falls_back_to_the_tokens_claim() {
        let token = jwt(serde_json::json!({
            "exp": 2_000,
            "https://api.openai.com/auth": {"chatgpt_account_id": "acct-2"}
        }));
        assert_eq!(login(&auth(&token, None), 0.0).unwrap().account, "acct-2");
        assert!(matches!(
            login(&auth(&jwt(serde_json::json!({})), None), 0.0),
            Err(CloudError::SignedOut)
        ));
    }

    #[test]
    fn an_expired_or_missing_sign_in_reads_as_signed_out() {
        let token = jwt(serde_json::json!({"exp": 2_000}));
        assert!(matches!(
            login(&auth(&token, Some("a")), 1_950_000.0),
            Err(CloudError::SignedOut)
        ));
        assert!(matches!(login("{}", 0.0), Err(CloudError::SignedOut)));
        assert!(matches!(
            login(r#"{"OPENAI_API_KEY":"sk"}"#, 0.0),
            Err(CloudError::SignedOut)
        ));
        assert!(matches!(
            login(&auth("", Some("a")), 0.0),
            Err(CloudError::SignedOut)
        ));
    }

    #[test]
    fn the_form_carries_the_wav_and_a_named_language() {
        let wav = wav(&[0.0; 1_600]).unwrap();
        let body = multipart("b", &wav, language_hint("de"));
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF"));
        assert!(text.contains("name=\"language\"\r\n\r\nde\r\n"));
        assert!(text.ends_with("--b--\r\n"));
        let auto = multipart("b", &wav, language_hint("auto"));
        assert!(!String::from_utf8_lossy(&auto).contains("name=\"language\""));
    }

    #[test]
    fn the_wav_is_16_khz_mono_16_bit() {
        let reader = hound::WavReader::new(Cursor::new(wav(&[0.5; 16_000]).unwrap())).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 16_000, 16)
        );
        assert_eq!(reader.len(), 16_000);
    }

    fn dictation(frames: &[&str]) -> Result<Dictation, CloudError> {
        let mut dictation = Dictation::default();
        for frame in frames {
            dictation.frame(frame)?;
        }
        Ok(dictation)
    }

    #[test]
    fn each_utterance_keeps_its_newest_final_in_the_order_it_began() {
        let dictation = dictation(&[
            r#"{"type":"session.started"}"#,
            r#"{"type":"transcript.final","utterance_id":"a","revision":0,"text":"so the"}"#,
            r#"{"type":"transcript.final","utterance_id":"b","revision":0,"text":"is ready"}"#,
            r#"{"type":"transcript.final","utterance_id":"a","revision":2,"text":"so the plan"}"#,
            r#"{"type":"transcript.final","utterance_id":"a","revision":1,"text":"stale"}"#,
            r#"{"type":"session.updated","session":{"status":"closed"}}"#,
        ])
        .unwrap();
        assert!(dictation.started && dictation.closed);
        assert_eq!(dictation.text(), "so the plan is ready");
    }

    #[test]
    fn speech_frames_track_whether_someone_is_talking() {
        let mut dictation = Dictation::default();
        dictation.frame(r#"{"type":"speech.started"}"#).unwrap();
        assert!(dictation.speaking);
        dictation.frame(r#"{"type":"speech.stopped"}"#).unwrap();
        assert!(!dictation.speaking);
    }

    #[test]
    fn failures_end_the_session() {
        assert!(dictation(&[r#"{"type":"transcript.failed"}"#]).is_err());
        assert!(dictation(&[r#"{"type":"session.error","fatal":true}"#]).is_err());
        assert!(dictation(&[r#"{"type":"session.error","fatal":false}"#]).is_ok());
        assert!(dictation(&["not json", r#"{"type":"unknown"}"#]).is_ok());
    }

    #[test]
    fn a_session_rotates_at_a_pause_after_four_minutes_and_always_before_five() {
        let minutes = |m: f64| Duration::from_secs_f64(m * 60.0);
        assert!(!rotation_due(minutes(3.9), false));
        assert!(rotation_due(minutes(4.0), false));
        assert!(!rotation_due(minutes(4.5), true));
        assert!(rotation_due(minutes(4.75), true));
    }

    #[test]
    fn the_session_starts_as_codex_desktop_starts_it() {
        let start: serde_json::Value = serde_json::from_str(&session_start("de")).unwrap();
        assert_eq!(start["type"], "session.start");
        assert_eq!(start["config"]["input_audio_format"], "pcm16");
        assert_eq!(start["config"]["sample_rate_hz"], 16_000);
        assert_eq!(start["config"]["language"], "de");
        assert_ne!(start["dictation_session_id"], start["attempt_id"]);
        let auto: serde_json::Value = serde_json::from_str(&session_start("auto")).unwrap();
        assert!(auto["config"].get("language").is_none());
    }

    #[test]
    fn session_ids_are_version_4_uuids() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_eq!(id.matches('-').count(), 4);
    }

    #[test]
    fn answers_map_to_text_or_why_not() {
        assert_eq!(answer(200, false, r#"{"text":"hi"}"#), Ok("hi".into()));
        assert_eq!(
            answer(200, true, r#"{"text":"hi"}"#),
            Err(CloudError::Refused(200))
        );
        assert_eq!(answer(401, false, ""), Err(CloudError::SignedOut));
        assert_eq!(answer(429, false, ""), Err(CloudError::RateLimited));
        assert_eq!(answer(403, false, ""), Err(CloudError::Refused(403)));
        assert_eq!(
            answer(400, false, r#"{"error":{"code":"audio_too_short"}}"#),
            Err(CloudError::NoSpeech)
        );
        assert!(matches!(
            answer(200, false, "<html>"),
            Err(CloudError::Failed(_))
        ));
        assert!(matches!(answer(502, false, ""), Err(CloudError::Failed(_))));
    }
}
