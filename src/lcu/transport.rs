use super::{
    discovery::LcuCredentials,
    normalize::{normalize_pickable_ids, normalize_session},
};
use crate::runtime::{DraftSnapshot, IntegrationError};
use base64::Engine;
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme,
    StreamOwned,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, client::IntoClientRequest, protocol::WebSocketConfig};
pub const SESSION: &str = "/lol-champ-select/v1/session";
pub const GAMEFLOW: &str = "/lol-gameflow/v1/gameflow-phase";
pub const PICKABLE: &str = "/lol-champ-select/v1/pickable-champion-ids";
const LIMIT: usize = 2 * 1024 * 1024;
#[derive(Clone)]
pub struct LcuTrust {
    config: Arc<ClientConfig>,
}
#[derive(Debug)]
struct LoopbackVerifier {
    inner: Arc<WebPkiServerVerifier>,
}
impl ServerCertVerifier for LoopbackVerifier {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(end, chain, name, ocsp, now) {
            // rustls 0.23.45 verifies chain, validity and server usage before name.
            // Riot's certificate identity is its dedicated trust root, not a DNS host.
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::NotValidForName
                | rustls::CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            result => result,
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, signature)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}
impl LcuTrust {
    pub fn riot() -> Result<Self, IntegrationError> {
        let pem = include_str!("../../assets/riotgames.pem");
        let encoded = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<String>();
        let der = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| IntegrationError::Invalid("Certificado Riot inválido"))?;
        Self::from_der(&der)
    }
    pub fn from_der(der: &[u8]) -> Result<Self, IntegrationError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(der.to_vec()))
            .map_err(|_| IntegrationError::Invalid("Certificado LCU inválido"))?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                .build()
                .map_err(|_| IntegrationError::Invalid("Trust LCU inválido"))?;
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| IntegrationError::Invalid("TLS LCU inválido"))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(LoopbackVerifier { inner: verifier }))
            .with_no_client_auth();
        config.enable_sni = false;
        config.resumption = rustls::client::Resumption::disabled();
        Ok(Self {
            config: Arc::new(config),
        })
    }
}
#[derive(Clone, Default)]
pub struct ConnectionStop {
    inner: Arc<StopState>,
}
#[derive(Default)]
struct StopState {
    cancelled: AtomicBool,
    next: AtomicU64,
    sockets: Mutex<BTreeMap<u64, TcpStream>>,
}
impl ConnectionStop {
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        for socket in self.inner.sockets.lock().unwrap().values() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }
    fn check(&self) -> Result<(), IntegrationError> {
        if self.is_cancelled() {
            Err(IntegrationError::Cancelled)
        } else {
            Ok(())
        }
    }
}
struct DeadlineSocket {
    tcp: TcpStream,
    deadline: Option<Instant>,
}
impl DeadlineSocket {
    fn timeout(&self) -> std::io::Result<()> {
        let remaining = match self.deadline {
            Some(end) => end
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::TimedOut))?,
            None => Duration::from_secs(1),
        };
        self.tcp.set_read_timeout(Some(remaining))?;
        self.tcp.set_write_timeout(Some(remaining))
    }
}
impl Read for DeadlineSocket {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.timeout()?;
        self.tcp.read(b)
    }
}
impl Write for DeadlineSocket {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.timeout()?;
        self.tcp.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.timeout()?;
        self.tcp.flush()
    }
}
struct Stream {
    tls: StreamOwned<ClientConnection, DeadlineSocket>,
    stop: ConnectionStop,
    id: u64,
}
impl Read for Stream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.tls.read(b)
    }
}
impl Write for Stream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.tls.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.tls.flush()
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.inner.sockets.lock().unwrap().remove(&self.id);
        let _ = self.tls.sock.tcp.shutdown(Shutdown::Both);
    }
}
#[derive(Debug)]
pub enum LcuEvent {
    Session(Box<Value>),
    Gameflow(String),
    Pickable(Value),
}
pub struct LcuTransport {
    credentials: LcuCredentials,
    trust: LcuTrust,
    stop: ConnectionStop,
    websocket: WebSocket<Stream>,
}
fn network(_: impl std::fmt::Debug) -> IntegrationError {
    IntegrationError::Network
}
fn open(
    credentials: &LcuCredentials,
    trust: &LcuTrust,
    stop: &ConnectionStop,
) -> Result<Stream, IntegrationError> {
    stop.check()?;
    let deadline = Some(Instant::now() + Duration::from_secs(15));
    let conn = ClientConnection::new(
        trust.config.clone(),
        ServerName::IpAddress(Ipv4Addr::LOCALHOST.into()),
    )
    .map_err(network)?;
    let tcp = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, credentials.port)),
        Duration::from_secs(2),
    )
    .map_err(network)?;
    tcp.set_nodelay(true).map_err(network)?;
    let id = stop.inner.next.fetch_add(1, Ordering::Relaxed);
    {
        let mut sockets = stop.inner.sockets.lock().unwrap();
        stop.check()?;
        sockets.insert(id, tcp.try_clone().map_err(network)?);
    }
    let mut stream = Stream {
        tls: StreamOwned::new(conn, DeadlineSocket { tcp, deadline }),
        stop: stop.clone(),
        id,
    };
    while stream.tls.conn.is_handshaking() {
        stream
            .tls
            .conn
            .complete_io(&mut stream.tls.sock)
            .map_err(network)?;
        stop.check()?;
    }
    Ok(stream)
}
impl LcuTransport {
    pub fn connect(
        credentials: LcuCredentials,
        trust: &LcuTrust,
    ) -> Result<Self, IntegrationError> {
        Self::connect_cancellable(credentials, trust, ConnectionStop::default())
    }
    pub fn connect_cancellable(
        credentials: LcuCredentials,
        trust: &LcuTrust,
        stop: ConnectionStop,
    ) -> Result<Self, IntegrationError> {
        let stream = open(&credentials, trust, &stop)?;
        let mut request = format!("wss://127.0.0.1:{}/", credentials.port)
            .into_client_request()
            .map_err(network)?;
        request.headers_mut().insert(
            "Authorization",
            authorization(&credentials).parse().map_err(network)?,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(LIMIT))
            .max_frame_size(Some(LIMIT));
        let (mut websocket, _) =
            tungstenite::client::client_with_config(request, stream, Some(config))
                .map_err(network)?;
        websocket
            .send(Message::Text("[5,\"OnJsonApiEvent\"]".into()))
            .map_err(network)?;
        websocket.get_mut().tls.sock.deadline = None;
        Ok(Self {
            credentials,
            trust: trust.clone(),
            stop,
            websocket,
        })
    }
    pub fn interrupter(&self) -> ConnectionStop {
        self.stop.clone()
    }
    pub fn get_json(&mut self, path: &str) -> Result<Value, IntegrationError> {
        if !matches!(path, SESSION | GAMEFLOW | PICKABLE | "/help") {
            return Err(IntegrationError::Invalid("Endpoint LCU não permitido"));
        }
        let mut stream = open(&self.credentials, &self.trust, &self.stop)?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nAccept: application/json\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
            self.credentials.port,
            authorization(&self.credentials)
        );
        stream.write_all(request.as_bytes()).map_err(network)?;
        stream.flush().map_err(network)?;
        let mut head = Vec::new();
        let mut byte = [0u8];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() >= 16384 {
                return Err(IntegrationError::Invalid("Cabeçalho LCU excede limite"));
            }
            stream.read_exact(&mut byte).map_err(network)?;
            head.push(byte[0]);
        }
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut response = httparse::Response::new(&mut headers);
        if !response.parse(&head).map_err(network)?.is_complete() {
            return Err(IntegrationError::Invalid("Resposta LCU inválida"));
        }
        let status = response
            .code
            .ok_or(IntegrationError::Invalid("Resposta LCU inválida"))?;
        if status == 404 {
            return Ok(Value::Null);
        }
        if !(200..300).contains(&status) {
            return Err(IntegrationError::Http {
                status,
                retry_after: None,
            });
        }
        let mut size = None;
        let mut chunked = false;
        for header in response.headers.iter() {
            if header.name.eq_ignore_ascii_case("Content-Length") {
                let value = std::str::from_utf8(header.value)
                    .map_err(network)?
                    .trim()
                    .parse::<usize>()
                    .map_err(network)?;
                if size.is_some_and(|old| old != value) {
                    return Err(IntegrationError::Invalid("Tamanho LCU contraditório"));
                }
                size = Some(value);
            }
            if header.name.eq_ignore_ascii_case("Transfer-Encoding") {
                if header.value.eq_ignore_ascii_case(b"chunked") {
                    chunked = true;
                } else {
                    return Err(IntegrationError::Invalid("Codificação LCU incompatível"));
                }
            }
            if header.name.eq_ignore_ascii_case("Content-Encoding")
                && !header.value.eq_ignore_ascii_case(b"identity")
            {
                return Err(IntegrationError::Invalid("Compressão LCU inesperada"));
            }
        }
        if chunked && size.is_some() {
            return Err(IntegrationError::Invalid("Enquadramento LCU contraditório"));
        }
        let mut body = Vec::new();
        if chunked {
            loop {
                let mut line = Vec::new();
                while !line.ends_with(b"\r\n") {
                    if line.len() > 128 {
                        return Err(IntegrationError::Invalid("Chunk LCU inválido"));
                    }
                    stream.read_exact(&mut byte).map_err(network)?;
                    line.push(byte[0]);
                }
                let text = std::str::from_utf8(&line).map_err(network)?.trim();
                let count = usize::from_str_radix(text.split(';').next().unwrap_or(""), 16)
                    .map_err(network)?;
                if count == 0 {
                    break;
                }
                if count > LIMIT - head.len() - body.len() {
                    return Err(IntegrationError::Invalid("Resposta LCU excede limite"));
                }
                let start = body.len();
                body.resize(start + count, 0);
                stream.read_exact(&mut body[start..]).map_err(network)?;
                let mut end = [0; 2];
                stream.read_exact(&mut end).map_err(network)?;
                if end != *b"\r\n" {
                    return Err(IntegrationError::Invalid("Chunk LCU inválido"));
                }
            }
        } else if let Some(size) = size {
            if size > LIMIT - head.len() {
                return Err(IntegrationError::Invalid("Resposta LCU excede limite"));
            }
            body.resize(size, 0);
            stream.read_exact(&mut body).map_err(network)?;
        } else {
            stream
                .take((LIMIT - head.len() + 1) as u64)
                .read_to_end(&mut body)
                .map_err(network)?;
            if body.len() > LIMIT - head.len() {
                return Err(IntegrationError::Invalid("Resposta LCU excede limite"));
            }
        }
        if body.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_slice(&body).map_err(Into::into)
        }
    }
    pub fn read_initial_snapshot(
        &mut self,
        generation: u64,
        revision: u64,
    ) -> Result<DraftSnapshot, IntegrationError> {
        let phase = self.get_json(GAMEFLOW)?;
        let raw = match phase.as_str() {
            Some("ChampSelect") => self.get_json(SESSION)?,
            Some(_) => Value::Null,
            None if phase.is_null() => self.get_json(SESSION)?,
            None => {
                return Err(IntegrationError::Invalid(
                    "Formato de gameflow incompatível",
                ));
            }
        };
        let mut snapshot = normalize_session(&raw, generation, revision)?;
        if !raw.is_null() {
            let ids = self.get_json(PICKABLE)?;
            if !ids.is_null() {
                snapshot.pickable_ids = Some(normalize_pickable_ids(&ids)?);
            }
        }
        Ok(snapshot)
    }
    pub fn next_event(&mut self) -> Result<Option<LcuEvent>, IntegrationError> {
        loop {
            self.stop.check()?;
            let message = match self.websocket.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(network(e)),
            };
            match message {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)?;
                    let Some(frame) = value.as_array() else {
                        continue;
                    };
                    if frame.len() != 3 || frame[0] != 8 || frame[1] != "OnJsonApiEvent" {
                        continue;
                    }
                    let event = &frame[2];
                    let uri = event["uri"].as_str().unwrap_or("");
                    let kind = event["eventType"].as_str().unwrap_or("");
                    if !matches!(kind, "Create" | "Update" | "Delete") {
                        continue;
                    }
                    match uri {
                        SESSION => {
                            return Ok(Some(LcuEvent::Session(Box::new(if kind == "Delete" {
                                Value::Null
                            } else {
                                event["data"].clone()
                            }))));
                        }
                        GAMEFLOW => {
                            if let Some(phase) = event["data"].as_str() {
                                return Ok(Some(LcuEvent::Gameflow(phase.into())));
                            }
                        }
                        PICKABLE => return Ok(Some(LcuEvent::Pickable(event["data"].clone()))),
                        _ => {}
                    }
                }
                Message::Ping(_) => self.websocket.flush().map_err(network)?,
                Message::Close(_) => return Err(IntegrationError::Network),
                _ => {}
            }
        }
    }
    pub fn close(&mut self) {
        self.stop.cancel();
    }
}
impl Drop for LcuTransport {
    fn drop(&mut self) {
        self.close();
    }
}
fn authorization(credentials: &LcuCredentials) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", credentials.token))
    )
}
