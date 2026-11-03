//! Claude's dictation service as Claude Code's voice mode reaches it: the
//! sign-in Claude Code saved (its token variable, then the macOS Keychain,
//! then `.credentials.json`), and its speech-to-text WebSocket with Claude
//! Code's own identity, fed while the dictation records.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::protocol::frame::coding::CloseCode;

use super::CloudError;
use super::socket::{self, Event};
use super::stream::{Audio, joined, publish};
use crate::transcription::engine::Partials;

const HOST: &str = "api.anthropic.com";
const PATH: &str = "/api/ws/speech_to_text/voice_stream";
const CLI_VERSION: &str = "2.1.317";
/// One second of 16 kHz 16-bit audio per binary frame at most.
const FRAME_BYTES: usize = 32_000;
const KEEP_ALIVE: &str = r#"{"type":"KeepAlive"}"#;
const CLOSE_STREAM: &str = r#"{"type":"CloseStream"}"#;
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(8);
/// After CloseStream, the service needs at least this long, or half the
/// length of audio still unheard (a whole clip sent at once).
const MIN_FINISH: Duration = Duration::from_secs(15);
const MAX_KEYTERMS: usize = 50;
const MAX_KEYTERM_CHARS: usize = 1_024;
const LANGUAGES: [&str; 20] = [
    "en", "es", "fr", "ja", "de", "pt", "it", "ko", "hi", "id", "ru", "pl", "tr", "nl", "uk", "el",
    "cs", "da", "sv", "no",
];

/// A whole clip's text, or why there is none, by `deadline`.
pub(super) fn transcribe(
    clip: &[f32],
    language: &str,
    keyterms: &[String],
    deadline: Instant,
) -> Result<String, CloudError> {
    stream(
        &Audio::whole(clip),
        language,
        keyterms,
        None,
        Some(deadline),
    )
}

/// Sends `audio` as it arrives and returns the text once the service
/// closes after the end; every read and write stops by `deadline`.
pub(super) fn stream(
    audio: &Audio,
    language: &str,
    keyterms: &[String],
    partials: Option<&Partials>,
    deadline: Option<Instant>,
) -> Result<String, CloudError> {
    let language = spoken_language(language)?;
    let mut socket = socket::connect(HOST, request(&token()?, language, keyterms)?, deadline)?;
    socket::send(&mut socket, Message::text(KEEP_ALIVE))?;
    let mut keep_alive = Instant::now() + KEEP_ALIVE_EVERY;
    let mut transcript = Transcript::default();
    let mut ending = false;
    loop {
        if !ending {
            let taken = audio.take()?;
            let pcm: Vec<u8> = super::pcm16(&taken.audio)
                .flat_map(i16::to_le_bytes)
                .collect();
            for frame in pcm.chunks(FRAME_BYTES) {
                socket::send(&mut socket, Message::binary(frame.to_vec()))?;
            }
            if taken.ended {
                socket::send(&mut socket, Message::text(CLOSE_STREAM))?;
                let unheard = Duration::from_secs_f64(taken.audio.len() as f64 / 16_000.0);
                let finish = Instant::now() + MIN_FINISH.max(unheard / 2);
                socket::limit(
                    &mut socket,
                    Some(deadline.map_or(finish, |deadline| deadline.min(finish))),
                );
                ending = true;
            }
        }
        match socket::next(&mut socket)? {
            Event::Text(frame) => {
                if transcript.frame(&frame)? {
                    publish(partials, transcript.committed(), transcript.pending.clone());
                }
            }
            Event::Closed(code) if ending => return transcript.closed(code),
            Event::Closed(_) => {
                return Err(CloudError::Failed(
                    "Claude closed the stream before the dictation ended".into(),
                ));
            }
            Event::Idle if Instant::now() >= keep_alive => {
                socket::send(&mut socket, Message::text(KEEP_ALIVE))?;
                keep_alive = Instant::now() + KEEP_ALIVE_EVERY;
            }
            Event::Idle => {}
        }
    }
}

/// The language Claude transcribes in: Auto is English, as in Claude Code.
fn spoken_language(language: &str) -> Result<&str, CloudError> {
    match language {
        super::super::models::AUTO_LANGUAGE | "" => Ok("en"),
        language if LANGUAGES.contains(&language) => Ok(language),
        language => Err(CloudError::Language(language.into())),
    }
}

/// The text so far: `TranscriptText` replaces the segment in progress and
/// `TranscriptEndpoint` commits it. Claude Code stops at the first endpoint
/// after CloseStream, which cuts a dictation short, so this waits for the
/// service to close the socket.
#[derive(Default)]
struct Transcript {
    committed: Vec<String>,
    pending: String,
}

impl Transcript {
    /// Takes one frame; true when the text changed.
    fn frame(&mut self, data: &str) -> Result<bool, CloudError> {
        #[derive(Deserialize)]
        struct Frame {
            #[serde(rename = "type")]
            kind: String,
            data: Option<String>,
        }
        let Ok(frame) = serde_json::from_str::<Frame>(data) else {
            return Ok(false);
        };
        match frame.kind.as_str() {
            "TranscriptText" => {
                let text = frame.data.unwrap_or_default();
                let changed = text != self.pending;
                self.pending = text;
                Ok(changed)
            }
            "TranscriptEndpoint" if !self.pending.is_empty() => {
                self.committed.push(std::mem::take(&mut self.pending));
                Ok(true)
            }
            "TranscriptError" | "error" => Err(CloudError::Failed(
                "Claude reported a transcription error".into(),
            )),
            _ => Ok(false),
        }
    }

    fn committed(&self) -> String {
        joined(self.committed.iter().map(String::as_str))
    }

    /// The text once the service closed the socket with `code`.
    fn closed(self, code: Option<CloseCode>) -> Result<String, CloudError> {
        match code {
            None | Some(CloseCode::Normal) => Ok(joined(
                self.committed
                    .iter()
                    .chain([&self.pending])
                    .map(String::as_str),
            )),
            Some(code) => Err(CloudError::Failed(format!(
                "Claude closed the connection ({})",
                u16::from(code)
            ))),
        }
    }
}

/// Claude Code's own request: its query, identity headers, and keyterms.
fn request(
    token: &str,
    language: &str,
    keyterms: &[String],
) -> Result<tungstenite::handshake::client::Request, CloudError> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("encoding", "linear16"),
            ("sample_rate", "16000"),
            ("channels", "1"),
            ("endpointing_ms", "300"),
            ("utterance_end_ms", "1000"),
            ("language", language),
            ("use_conversation_engine", "true"),
        ])
        .finish();
    let invalid = |what: &str| CloudError::Failed(format!("Claude: invalid {what}"));
    let mut request = format!("wss://{HOST}{PATH}?{query}")
        .into_client_request()
        .map_err(|_| invalid("request"))?;
    let headers = request.headers_mut();
    let mut set = |name: &'static str, value: String| -> Result<(), CloudError> {
        headers.insert(
            name,
            HeaderValue::from_str(&value).map_err(|_| invalid(name))?,
        );
        Ok(())
    };
    set("authorization", format!("Bearer {token}"))?;
    set(
        "user-agent",
        format!("claude-cli/{CLI_VERSION} (external, cli)"),
    )?;
    set("x-app", "cli".into())?;
    set("anthropic-client-platform", "claude_code_cli".into())?;
    let keyterms = keyterms_header(keyterms);
    if !keyterms.is_empty() {
        set("x-config-keyterms", keyterms)?;
    }
    Ok(request)
}

/// Claude Code's rules for keyterms: commas split terms, so they become
/// spaces; printable ASCII only; at most 50 terms and 1,024 characters.
fn keyterms_header(terms: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::new();
    let mut length = 0;
    for term in terms {
        let clean = term
            .chars()
            .map(|c| if c == ',' { ' ' } else { c })
            .filter(|c| (' '..='~').contains(c))
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if clean.is_empty() || !seen.insert(clean.to_lowercase()) {
            continue;
        }
        let cost = clean.len() + usize::from(!kept.is_empty());
        if kept.len() == MAX_KEYTERMS || length + cost > MAX_KEYTERM_CHARS {
            break;
        }
        length += cost;
        kept.push(clean);
    }
    kept.join(",")
}

/// Claude Code's sign-in, read the way it reads it.
fn token() -> Result<String, CloudError> {
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN")
        && !token.trim().is_empty()
    {
        return Ok(token.trim().into());
    }
    let stored = keychain()?
        .or_else(|| std::fs::read_to_string(config_dir()?.join(".credentials.json")).ok());
    access_token(stored.as_deref(), super::now_ms())
}

fn access_token(stored: Option<&str>, now_ms: f64) -> Result<String, CloudError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Credentials {
        claude_ai_oauth: Option<OAuth>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OAuth {
        access_token: Option<String>,
        expires_at: Option<f64>,
    }
    let oauth = stored
        .and_then(|stored| serde_json::from_str::<Credentials>(stored.trim()).ok())
        .and_then(|credentials| credentials.claude_ai_oauth)
        .ok_or(CloudError::SignedOut)?;
    match (oauth.access_token, oauth.expires_at) {
        (_, Some(expires_at)) if expires_at <= now_ms => Err(CloudError::SignedOut),
        (Some(token), _) if !token.trim().is_empty() => Ok(token.trim().into()),
        _ => Err(CloudError::SignedOut),
    }
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
}

/// The Keychain item Claude Code keeps its sign-in in, read through
/// `security`, which the item trusts, so it never prompts.
#[cfg(target_os = "macos")]
fn keychain() -> Result<Option<String>, CloudError> {
    use std::process::Stdio;
    const NOT_FOUND: i32 = 44;
    const LOCKED: i32 = 36;
    let mut child = match crate::desktop::background_process::command("/usr/bin/security")
        .args(["find-generic-password", "-a"])
        .arg(keychain_account(std::env::var("USER").ok().as_deref()))
        .args(["-w", "-s"])
        .arg(keychain_service(
            std::env::var("CLAUDE_SECURESTORAGE_CONFIG_DIR")
                .ok()
                .as_deref(),
            std::env::var("CLAUDE_CONFIG_DIR").ok().as_deref(),
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Ok(None),
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            // An unlock prompt only someone at the Mac can answer.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CloudError::Failed("the Keychain is locked".into()));
            }
        }
    };
    match status.code() {
        Some(0) => {
            use std::io::Read;
            let mut stored = String::new();
            let read = child
                .stdout
                .take()
                .is_some_and(|mut stdout| stdout.read_to_string(&mut stored).is_ok());
            Ok(read.then_some(stored))
        }
        Some(LOCKED) => Err(CloudError::Failed("the Keychain is locked".into())),
        Some(NOT_FOUND) => Ok(None),
        code => {
            tracing::warn!(
                ?code,
                "reading Claude Code's sign-in from the Keychain failed"
            );
            Ok(None)
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn keychain() -> Result<Option<String>, CloudError> {
    Ok(None)
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn keychain_service(secure_storage_dir: Option<&str>, config_dir: Option<&str>) -> String {
    let suffix = secure_storage_dir
        .or(config_dir)
        .filter(|dir| !dir.is_empty())
        .map(|dir| {
            use sha2::Digest;
            use unicode_normalization::UnicodeNormalization;
            let digest = sha2::Sha256::digest(dir.nfc().collect::<String>().as_bytes());
            format!("-{}", data_encoding::HEXLOWER.encode(&digest[..4]))
        })
        .unwrap_or_default();
    format!("Claude Code-credentials{suffix}")
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn keychain_account(user: Option<&str>) -> String {
    match user {
        Some(user)
            if !user.is_empty()
                && user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) =>
        {
            user.into()
        }
        _ => "claude-code-user".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(frames: &[&str], code: Option<CloseCode>) -> Result<String, CloudError> {
        let mut transcript = Transcript::default();
        for frame in frames {
            transcript.frame(frame)?;
        }
        transcript.closed(code)
    }

    #[test]
    fn live_text_shows_committed_words_then_the_segment_in_progress() {
        let mut transcript = Transcript::default();
        assert!(
            transcript
                .frame(r#"{"type":"TranscriptText","data":"so the"}"#)
                .unwrap()
        );
        assert!(
            !transcript
                .frame(r#"{"type":"TranscriptText","data":"so the"}"#)
                .unwrap()
        );
        assert!(
            transcript
                .frame(r#"{"type":"TranscriptEndpoint"}"#)
                .unwrap()
        );
        assert!(
            transcript
                .frame(r#"{"type":"TranscriptText","data":"plan is"}"#)
                .unwrap()
        );
        assert_eq!(transcript.committed(), "so the");
        assert_eq!(transcript.pending, "plan is");
        assert!(!transcript.frame(r#"{"type":"Metadata"}"#).unwrap());
    }

    #[test]
    fn segments_join_once_the_service_closes_normally() {
        let frames = [
            r#"{"type":"TranscriptText","data":"hello"}"#,
            r#"{"type":"TranscriptText","data":"hello world"}"#,
            r#"{"type":"TranscriptEndpoint"}"#,
            r#"{"type":"TranscriptText","data":" how are you "}"#,
        ];
        assert_eq!(
            heard(&frames, Some(CloseCode::Normal)),
            Ok("hello world how are you".into())
        );
        assert_eq!(heard(&frames, None), Ok("hello world how are you".into()));
    }

    #[test]
    fn an_error_frame_or_abnormal_close_gives_no_text() {
        assert!(matches!(
            heard(&[r#"{"type":"TranscriptError"}"#], None),
            Err(CloudError::Failed(_))
        ));
        assert!(matches!(
            heard(
                &[r#"{"type":"TranscriptText","data":"partial"}"#],
                Some(CloseCode::Away)
            ),
            Err(CloudError::Failed(_))
        ));
    }

    #[test]
    fn unknown_frames_are_ignored() {
        assert_eq!(
            heard(
                &[
                    "not json",
                    r#"{"type":"Metadata"}"#,
                    r#"{"type":"TranscriptText","data":"ok"}"#
                ],
                None
            ),
            Ok("ok".into())
        );
    }

    #[test]
    fn auto_dictates_in_english_and_unsupported_languages_stay_local() {
        assert_eq!(spoken_language("auto"), Ok("en"));
        assert_eq!(spoken_language("de"), Ok("de"));
        assert_eq!(
            spoken_language("vi"),
            Err(CloudError::Language("vi".into()))
        );
    }

    #[test]
    fn keyterms_follow_claude_codes_rules() {
        let terms: Vec<String> = ["Rune, Kit", "rune kit", "Hëx", "  ", "OpenCode"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(keyterms_header(&terms), "Rune Kit,Hx,OpenCode");
        let many: Vec<String> = (0..80).map(|n| format!("term{n}")).collect();
        assert_eq!(keyterms_header(&many).split(',').count(), MAX_KEYTERMS);
        let long: Vec<String> = (0..10).map(|n| format!("{n}{}", "x".repeat(200))).collect();
        assert!(keyterms_header(&long).len() <= MAX_KEYTERM_CHARS);
    }

    #[test]
    fn a_saved_sign_in_gives_its_token_until_it_expires() {
        let stored = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01","expiresAt":2000}}"#;
        assert_eq!(
            access_token(Some(stored), 1000.0),
            Ok("sk-ant-oat01".into())
        );
        assert_eq!(
            access_token(Some(stored), 2000.0),
            Err(CloudError::SignedOut)
        );
        assert_eq!(access_token(Some("{}"), 0.0), Err(CloudError::SignedOut));
        assert_eq!(access_token(None, 0.0), Err(CloudError::SignedOut));
    }

    #[test]
    fn the_keychain_item_follows_claude_codes_config_folder() {
        assert_eq!(keychain_service(None, None), "Claude Code-credentials");
        assert_eq!(
            keychain_service(Some(""), Some("/x")),
            "Claude Code-credentials"
        );
        let custom = keychain_service(None, Some("/Users/k/.claude-work"));
        assert!(custom.starts_with("Claude Code-credentials-"));
        assert_eq!(custom.len(), "Claude Code-credentials-".len() + 8);
        assert_eq!(
            custom,
            keychain_service(Some("/Users/k/.claude-work"), None)
        );
        assert_eq!(keychain_account(Some("khoi")), "khoi");
        assert_eq!(keychain_account(Some("a b")), "claude-code-user");
        assert_eq!(keychain_account(None), "claude-code-user");
    }

    #[test]
    fn the_request_carries_claude_codes_identity() {
        let request = request("tok", "en", &["Hex".into()]).unwrap();
        let uri = request.uri().to_string();
        assert!(uri.starts_with("wss://api.anthropic.com/api/ws/speech_to_text/voice_stream?"));
        assert!(uri.contains("encoding=linear16") && uri.contains("language=en"));
        let headers = request.headers();
        assert_eq!(headers["authorization"], "Bearer tok");
        assert_eq!(headers["x-app"], "cli");
        assert_eq!(headers["x-config-keyterms"], "Hex");
        assert!(
            headers["user-agent"]
                .to_str()
                .unwrap()
                .starts_with("claude-cli/")
        );
    }
}
