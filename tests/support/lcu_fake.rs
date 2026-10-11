use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types::PrivatePkcs8KeyDer};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
type Routes = Arc<Mutex<BTreeMap<String, (u16, Vec<u8>)>>>;
type ResponseGate = (mpsc::Sender<()>, Arc<Mutex<mpsc::Receiver<()>>>);
type ResponseGates = Arc<Mutex<BTreeMap<String, ResponseGate>>>;
pub struct Fake {
    pub port: u16,
    pub certificate: Vec<u8>,
    pub requests: Arc<Mutex<Vec<(String, String)>>>,
    routes: Routes,
    response_gates: ResponseGates,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<String>>>>,
    stop: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<TcpStream>>>,
    worker: Option<thread::JoinHandle<()>>,
}
struct Prefix<S> {
    prefix: std::io::Cursor<Vec<u8>>,
    stream: S,
}
impl<S: Read> Read for Prefix<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.prefix.position() < self.prefix.get_ref().len() as u64 {
            self.prefix.read(buf)
        } else {
            self.stream.read(buf)
        }
    }
}
impl<S: Write> Write for Prefix<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}
impl Fake {
    pub fn start() -> Self {
        Self::with_expiration(false)
    }
    pub fn expired() -> Self {
        Self::with_expiration(true)
    }
    fn with_expiration(expired: bool) -> Self {
        let rcgen::CertifiedKey { cert, signing_key } = if expired {
            let mut params =
                rcgen::CertificateParams::new(vec!["riot-test-only.invalid".into()]).unwrap();
            params.not_before = rcgen::date_time_ymd(2020, 1, 1);
            params.not_after = rcgen::date_time_ymd(2021, 1, 1);
            let signing_key = rcgen::KeyPair::generate().unwrap();
            let cert = params.self_signed(&signing_key).unwrap();
            rcgen::CertifiedKey { cert, signing_key }
        } else {
            rcgen::generate_simple_self_signed(vec!["riot-test-only.invalid".into()]).unwrap()
        };
        let certificate = cert.der().to_vec();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let sockets = Arc::new(Mutex::new(Vec::<TcpStream>::new()));
        let routes = Arc::new(Mutex::new(BTreeMap::<String, (u16, Vec<u8>)>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let response_gates: ResponseGates = Default::default();
        let gates = response_gates.clone();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (halt, streams, table, record, clients) = (
            stop.clone(),
            sockets.clone(),
            routes.clone(),
            requests.clone(),
            subscribers.clone(),
        );
        let config = Arc::new(config);
        let worker = thread::spawn(move || {
            let mut children = Vec::new();
            while !halt.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((tcp, _)) => {
                        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                        tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
                        streams.lock().unwrap().push(tcp.try_clone().unwrap());
                        let (config, table, record, clients, halt) = (
                            config.clone(),
                            table.clone(),
                            record.clone(),
                            clients.clone(),
                            halt.clone(),
                        );
                        let gates = gates.clone();
                        children.push(thread::spawn(move|| {
                    let mut stream=StreamOwned::new(ServerConnection::new(config).unwrap(),tcp);let mut head=Vec::new();let mut byte=[0u8];
                    while !head.ends_with(b"\r\n\r\n")&&head.len()<16384 {if stream.read_exact(&mut byte).is_err(){return;}head.push(byte[0]);}
                    let text=String::from_utf8_lossy(&head);let path=text.lines().next().and_then(|l|l.split_whitespace().nth(1)).unwrap_or("").to_owned();
                    let auth=text.lines().find_map(|line|line.split_once(':').filter(|(name,_)|name.eq_ignore_ascii_case("authorization")).map(|(_,v)|v.trim().to_owned())).unwrap_or_default();
                    record.lock().unwrap().push((path.clone(),auth));
                    if text.to_ascii_lowercase().contains("upgrade: websocket") {
                        let (sender,receiver)=mpsc::channel::<String>();clients.lock().unwrap().push(sender);
                        let Ok(mut ws)=tungstenite::accept(Prefix {prefix:std::io::Cursor::new(head),stream}) else{return;};
                        let Ok(message)=ws.read() else{return;};
                        if message.to_text().ok()!=Some("[5,\"OnJsonApiEvent\"]") {return;}
                        while !halt.load(Ordering::SeqCst) {match receiver.recv_timeout(Duration::from_millis(50)) {Ok(message)=>{if ws.send(tungstenite::Message::Text(message.into())).is_err(){break;}},Err(mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>break}}
                    } else {
                        let gate = gates.lock().unwrap().get(&path).cloned();
                        if let Some((entered, release)) = gate {
                            let _ = entered.send(());
                            let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(3));
                        }
                        let (status,body)=table.lock().unwrap().get(&path).cloned().unwrap_or((404,b"null".to_vec()));
                        let response=format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
                        let _=stream.write_all(response.as_bytes());let _=stream.write_all(&body);let _=stream.flush();
                    }
                }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
            for child in children {
                let _ = child.join();
            }
        });
        Self {
            port,
            certificate,
            requests,
            routes,
            response_gates,
            subscribers,
            stop,
            sockets,
            worker: Some(worker),
        }
    }
    pub fn route(&self, path: &str, status: u16, body: Value) {
        self.routes
            .lock()
            .unwrap()
            .insert(path.into(), (status, serde_json::to_vec(&body).unwrap()));
    }
    pub fn pause_response(&self, path: &str) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, observed) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        self.response_gates
            .lock()
            .unwrap()
            .insert(path.into(), (entered, Arc::new(Mutex::new(wait))));
        (observed, release)
    }
    pub fn emit(&self, uri: &str, kind: &str, data: Value) {
        let text = serde_json::json!([8,"OnJsonApiEvent",{"uri":uri,"eventType":kind,"data":data}])
            .to_string();
        for sender in self.subscribers.lock().unwrap().iter() {
            let _ = sender.send(text.clone());
        }
    }
    pub fn credentials(&self) -> pikker::lcu::discovery::LcuCredentials {
        pikker::lcu::discovery::parse_lockfile(&format!(
            "LeagueClient:1:{}:fake-password:https",
            self.port
        ))
        .unwrap()
    }
    pub fn disconnect_clients(&self) {
        for socket in self.sockets.lock().unwrap().iter() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for stream in self.sockets.lock().unwrap().iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
