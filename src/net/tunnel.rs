//! A TCP connection for protocols ureq doesn't speak (WebSocket), through
//! the proxy the computer uses: directly, or an HTTP proxy's CONNECT tunnel.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// The longest proxy answer read before giving up on it.
const MAX_PROXY_ANSWER: usize = 8 * 1024;

/// A connection to `host:port`, the way an `https://host` request would
/// reach it, each step bounded by `timeout`. Errors read "Couldn't reach …".
pub fn tunnel(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let unreachable = |reason: String| format!("Couldn't reach {host}: {reason}");
    let Some(proxy) = super::proxy_for(&format!("https://{host}/")) else {
        return connect(host, port, timeout).map_err(unreachable);
    };
    let via = |reason: String| {
        unreachable(format!(
            "through the proxy {}:{}, {reason}",
            proxy.host(),
            proxy.port()
        ))
    };
    if !matches!(proxy.protocol(), ureq::ProxyProtocol::Http) {
        return Err(via("which isn't an HTTP proxy".into()));
    }
    let credentials = proxy
        .username()
        .map(|user| format!("{user}:{}", proxy.password().unwrap_or_default()));
    let stream = connect(proxy.host(), proxy.port(), timeout).map_err(via)?;
    open(stream, host, port, credentials.as_deref(), timeout).map_err(via)
}

fn connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| "the name couldn't be looked up (DNS)".to_string())?;
    let mut last = "the connection failed".to_string();
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = format!("the connection failed ({error})"),
        }
    }
    Err(last)
}

/// Asks the proxy on `stream` to CONNECT to `host:port`.
fn open(
    mut stream: TcpStream,
    host: &str,
    port: u16,
    credentials: Option<&str>,
    timeout: Duration,
) -> Result<TcpStream, String> {
    let failed = |error: std::io::Error| format!("the connection failed ({error})");
    stream.set_read_timeout(Some(timeout)).map_err(failed)?;
    stream.set_write_timeout(Some(timeout)).map_err(failed)?;
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some(credentials) = credentials {
        let encoded = data_encoding::BASE64.encode(credentials.as_bytes());
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).map_err(failed)?;
    // Byte by byte, so nothing past the answer is taken from the tunnel.
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        if answer.len() == MAX_PROXY_ANSWER {
            return Err("the proxy's answer was too long".into());
        }
        match stream.read(&mut byte).map_err(failed)? {
            0 => return Err("the proxy closed the connection".into()),
            _ => answer.push(byte[0]),
        }
    }
    let status = String::from_utf8_lossy(&answer)
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        Some(200) => Ok(stream),
        Some(407) => Err("the proxy needs a sign-in Hex can't give".into()),
        Some(code) => Err(format!("the proxy refused (HTTP {code})")),
        None => Err("the proxy's answer wasn't HTTP".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A proxy on loopback that answers `answer` to the first request and
    /// returns it, then echoes the tunnel's first bytes.
    fn proxy(answer: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                client.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            client.write_all(answer.as_bytes()).unwrap();
            let mut echo = [0u8; 4];
            if client.read_exact(&mut echo).is_ok() {
                client.write_all(&echo).unwrap();
            }
            String::from_utf8(request).unwrap()
        });
        (port, handle)
    }

    #[test]
    fn a_proxy_that_connects_hands_over_the_tunnel() {
        let (port, proxy) = proxy("HTTP/1.1 200 Connection established\r\n\r\n");
        let stream = connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        let mut tunnel = open(
            stream,
            "api.example.com",
            443,
            Some("me:secret"),
            Duration::from_secs(5),
        )
        .unwrap();
        tunnel.write_all(b"ping").unwrap();
        let mut echo = [0u8; 4];
        tunnel.read_exact(&mut echo).unwrap();
        assert_eq!(&echo, b"ping");
        let request = proxy.join().unwrap();
        assert!(request.starts_with("CONNECT api.example.com:443 HTTP/1.1\r\n"));
        assert!(request.contains("Proxy-Authorization: Basic bWU6c2VjcmV0\r\n"));
    }

    #[test]
    fn a_proxy_that_refuses_names_why() {
        for (answer, reason) in [
            (
                "HTTP/1.1 407 Proxy Authentication Required\r\n\r\n",
                "sign-in",
            ),
            ("HTTP/1.1 403 Forbidden\r\n\r\n", "HTTP 403"),
        ] {
            let (port, _proxy) = proxy(answer);
            let stream = connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
            let error = open(stream, "h", 443, None, Duration::from_secs(5)).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }
}
