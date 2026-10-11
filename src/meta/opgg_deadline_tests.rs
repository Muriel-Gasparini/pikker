use super::*;
use std::{io::Write, net::TcpListener, time::Instant};
#[derive(Debug)]
struct Local(u16);
impl ureq::unversioned::resolver::Resolver for Local {
    fn resolve(
        &self,
        _: &ureq::http::Uri,
        _: &ureq::config::Config,
        _: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let mut addresses = self.empty();
        addresses.push(std::net::SocketAddr::from(([127, 0, 0, 1], self.0)));
        Ok(addresses)
    }
}
#[test]
fn shutdown_joins_public_request_even_when_tls_peer_keeps_dripping() {
    use crate::{
        domain::PlayerProfile,
        runtime::{RuntimeCommand, RuntimeConfig, start_runtime},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (entered, observed) = std::sync::mpsc::channel();
    let peer = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.write_all(&[0x16, 3, 3, 0x3f, 0xff]).unwrap();
        entered.send(()).unwrap();
        for _ in 0..12 {
            if socket.write_all(&[0]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    struct Source(PublicTransport);
    impl MetaProvider for Source {
        fn fetch_general(&mut self) -> Result<MetaSnapshot, IntegrationError> {
            self.0.send(HttpRequest {
                url: ENDPOINT.into(),
                body: None,
                headers: vec![],
            })?;
            Err(IntegrationError::Network)
        }
        fn fetch_details(
            &mut self,
            _: &ChampionIdentity,
            _: Role,
            _: Option<&str>,
        ) -> Result<ChampionDetails, IntegrationError> {
            unreachable!()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let config = RuntimeConfig::with_provider(
        dir.path().join("meta.json"),
        PlayerProfile::default(),
        Role::Support,
        None,
        std::sync::Arc::new(move |_, _, _| {
            let config = ureq::Agent::config_builder()
                .https_only(true)
                .proxy(None)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_millis(150)))
                .max_idle_connections(0)
                .build();
            Ok(Box::new(Source(PublicTransport {
                agent: ureq::Agent::with_parts(config, public_connector(), Local(port)),
            })))
        }),
    );
    let handle = start_runtime(config, std::sync::Arc::new(|_| {})).unwrap();
    handle.send(RuntimeCommand::StartBootstrap).unwrap();
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    handle.shutdown().unwrap();
    let elapsed = start.elapsed();
    peer.join().unwrap();
    assert!(
        elapsed < Duration::from_millis(350),
        "shutdown outlived absolute request deadline: {elapsed:?}"
    );
}
#[test]
fn public_https_slow_tls_record_cannot_reset_absolute_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.write_all(&[0x16, 3, 3, 0x3f, 0xff]).unwrap();
        for _ in 0..12 {
            if socket.write_all(&[0]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    // DNS is the only external boundary replaced. HTTPS restrictions,
    // public TLS verification and connector are the production path.
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_millis(150)))
        .max_idle_connections(0)
        .build();
    let transport = PublicTransport {
        agent: ureq::Agent::with_parts(config, public_connector(), Local(port)),
    };
    let start = Instant::now();
    let result = transport.send(HttpRequest {
        url: ENDPOINT.into(),
        body: None,
        headers: vec![],
    });
    let elapsed = start.elapsed();
    peer.join().unwrap();
    assert!(matches!(result, Err(IntegrationError::Network)));
    assert!(
        elapsed < Duration::from_millis(350),
        "slow TLS extended deadline: {elapsed:?}"
    );
}
