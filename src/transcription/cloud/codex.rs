//! Codex's dictation service as Codex Desktop reaches it: the ChatGPT
//! sign-in Codex saved in `auth.json`, and a WAV posted to ChatGPT's
//! transcribe endpoint with Codex Desktop's identity, which is the only one
//! it answers.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use super::CloudError;

const URL: &str = "https://chatgpt.com/backend-api/transcribe";
const DESKTOP_VERSION: &str = "26.1021.31345";
const TIMEOUT: Duration = Duration::from_secs(120);
/// A token this close to expiring reads as expired.
const EXPIRY_MARGIN_MS: f64 = 60_000.0;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

pub(super) fn transcribe(clip: &[f32], language: &str) -> Result<String, CloudError> {
    let auth = codex_home()
        .and_then(|home| std::fs::read_to_string(home.join("auth.json")).ok())
        .ok_or(CloudError::SignedOut)?;
    let login = login(&auth, super::now_ms())?;
    let boundary = boundary();
    let body = multipart(&boundary, &wav(clip)?, language_hint(language));
    let agent = crate::net::agent(URL, |config| {
        config
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
    });
    let mut response = agent
        .post(URL)
        .header("authorization", format!("Bearer {}", login.token))
        .header("chatgpt-account-id", &login.account)
        .header("originator", "Codex Desktop")
        .header(
            "user-agent",
            format!("Codex Desktop/{DESKTOP_VERSION} (Mac OS; arm64)"),
        )
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

struct Login {
    token: String,
    account: String,
}

/// The access token and ChatGPT account in Codex's `auth.json`, unless it
/// expired: Codex renews it, Hex never does.
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
    let mut random = [0u8; 16];
    let _ = getrandom::fill(&mut random);
    format!("hex-{}", data_encoding::HEXLOWER.encode(&random))
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
