//! The WebSocket both services speak: a TCP connection through the
//! computer's proxy (`net::tunnel`), platform TLS, and a socket that hands
//! control back at least every [`SLICE`], however the service sends, and
//! refuses everything once its deadline passes.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use tungstenite::handshake::client::Request;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{HandshakeError, Message};

use super::CloudError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The longest a read waits for a byte.
const POLL: Duration = Duration::from_millis(50);
/// The longest one look at the socket lasts, even while a frame trickles
/// in; tungstenite keeps the partial frame for the next look.
const SLICE: Duration = Duration::from_millis(250);

pub(super) type Socket = tungstenite::WebSocket<MaybeTlsStream<Bounded>>;

/// What one read of the socket brought.
pub(super) enum Event {
    Text(String),
    Closed(Option<CloseCode>),
    /// Nothing within [`POLL`].
    Idle,
}

/// Opens `request` to `host`, connecting within [`CONNECT_TIMEOUT`] (or
/// `deadline`, if sooner); the socket then lives until `deadline`, if any.
pub(super) fn connect(
    host: &str,
    request: Request,
    deadline: Option<Instant>,
) -> Result<Socket, CloudError> {
    let unreachable =
        |detail: String| CloudError::Failed(format!("Couldn't reach {host}: {detail}"));
    let connected_by = deadline.map_or(Instant::now() + CONNECT_TIMEOUT, |deadline| {
        deadline.min(Instant::now() + CONNECT_TIMEOUT)
    });
    let step = connected_by
        .saturating_duration_since(Instant::now())
        .max(POLL);
    let stream = crate::net::tunnel(host, 443, step).map_err(CloudError::Failed)?;
    let timeouts = |read| {
        stream
            .set_read_timeout(Some(read))
            .and_then(|()| stream.set_write_timeout(Some(CONNECT_TIMEOUT)))
            .map_err(|error| unreachable(error.to_string()))
    };
    timeouts(CONNECT_TIMEOUT)?;
    // A clone shares the socket, so its timeouts still apply under TLS.
    let control = stream
        .try_clone()
        .map_err(|error| unreachable(error.to_string()))?;
    let bounded = Bounded {
        stream,
        deadline: Some(connected_by),
        slice: None,
    };
    let mut socket = match tungstenite::client_tls(request, bounded) {
        Ok((socket, _)) => socket,
        Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            return Err(refusal(
                response.status().as_u16(),
                response.headers().contains_key("cf-mitigated"),
            ));
        }
        Err(HandshakeError::Failure(error)) => return Err(unreachable(error.to_string())),
        Err(HandshakeError::Interrupted(_)) => {
            return Err(unreachable("the connection timed out".into()));
        }
    };
    control
        .set_read_timeout(Some(POLL))
        .map_err(|error| unreachable(error.to_string()))?;
    limit(&mut socket, deadline);
    Ok(socket)
}

/// Why a service refused the WebSocket. A 403 can mean the account has no
/// dictation access, so it isn't read as signed out.
pub(super) fn refusal(status: u16, mitigated: bool) -> CloudError {
    match status {
        _ if mitigated => CloudError::Refused(status),
        401 => CloudError::SignedOut,
        429 => CloudError::RateLimited,
        403 | 404 | 405 | 410 => CloudError::Refused(status),
        status => CloudError::Failed(format!("the service answered HTTP {status}")),
    }
}

/// From now on, every read and write fails once `deadline` passes; `None`
/// lifts it.
pub(super) fn limit(socket: &mut Socket, deadline: Option<Instant>) {
    if let Some(bounded) = bounded(socket) {
        bounded.deadline = deadline;
    }
}

fn bounded(socket: &mut Socket) -> Option<&mut Bounded> {
    match socket.get_mut() {
        MaybeTlsStream::NativeTls(tls) => Some(tls.get_mut()),
        _ => None,
    }
}

pub(super) fn send(socket: &mut Socket, message: Message) -> Result<(), CloudError> {
    socket.send(message).map_err(failed)
}

/// The next frame, a close, or nothing within [`SLICE`].
pub(super) fn next(socket: &mut Socket) -> Result<Event, CloudError> {
    if let Some(bounded) = bounded(socket) {
        bounded.slice = Some(Instant::now() + SLICE);
    }
    let read = socket.read();
    let expired = bounded(socket).is_some_and(|bounded| {
        bounded.slice = None;
        bounded.check().is_err()
    });
    match read {
        Ok(Message::Text(text)) => Ok(Event::Text(text.as_str().to_owned())),
        Ok(Message::Close(frame)) => Ok(Event::Closed(frame.map(|frame| frame.code))),
        Ok(_) => Ok(Event::Idle),
        Err(tungstenite::Error::Io(error))
            if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) && !expired =>
        {
            Ok(Event::Idle)
        }
        Err(error) => Err(failed(error)),
    }
}

fn failed(error: tungstenite::Error) -> CloudError {
    CloudError::Failed(format!("the connection failed ({error})"))
}

/// The socket, refusing every read and write once its deadline passes, so a
/// service that trickles bytes can't hold a read open past it.
pub(super) struct Bounded {
    stream: TcpStream,
    deadline: Option<Instant>,
    /// When the current look at the socket ends: reads past it would block.
    slice: Option<Instant>,
}

impl Bounded {
    fn check(&self) -> std::io::Result<()> {
        match self.deadline {
            Some(deadline) if Instant::now() >= deadline => Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "the service didn't finish in time",
            )),
            _ => Ok(()),
        }
    }
}

impl Read for Bounded {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        if self.slice.is_some_and(|slice| Instant::now() >= slice) {
            return Err(ErrorKind::WouldBlock.into());
        }
        self.stream.read(buffer)
    }
}

impl Write for Bounded {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.check()?;
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passed_deadline_refuses_reads_and_writes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut bounded = Bounded {
            stream,
            deadline: Some(Instant::now()),
            slice: None,
        };
        assert_eq!(
            bounded.read(&mut [0; 4]).unwrap_err().kind(),
            ErrorKind::TimedOut
        );
        assert_eq!(
            bounded.write(b"late").unwrap_err().kind(),
            ErrorKind::TimedOut
        );
    }

    #[test]
    fn a_look_that_ran_its_slice_yields_but_keeps_the_socket_open() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server.write_all(b"data").unwrap();
        let mut bounded = Bounded {
            stream,
            deadline: None,
            slice: Some(Instant::now()),
        };
        assert_eq!(
            bounded.read(&mut [0; 4]).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        bounded.slice = None;
        let mut read = [0; 4];
        bounded.stream.set_read_timeout(Some(POLL)).unwrap();
        assert_eq!(bounded.read(&mut read).unwrap(), 4);
        assert_eq!(&read, b"data");
    }

    #[test]
    fn refusals_name_why() {
        assert_eq!(refusal(401, false), CloudError::SignedOut);
        assert_eq!(refusal(401, true), CloudError::Refused(401));
        assert_eq!(refusal(403, false), CloudError::Refused(403));
        assert_eq!(refusal(404, false), CloudError::Refused(404));
        assert_eq!(refusal(429, false), CloudError::RateLimited);
        assert!(matches!(refusal(500, false), CloudError::Failed(_)));
    }
}
