use super::client_builder;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

const MESSAGE: &str = "synthetic notification diagnostics";

struct Server {
    url: String,
    certificate: Option<reqwest::Certificate>,
    stop: mpsc::Sender<()>,
    thread: thread::JoinHandle<Vec<String>>,
}

impl Server {
    fn start(https: bool, responses: Vec<String>) -> Self {
        let (tls, certificate) = if https {
            let rcgen::CertifiedKey { cert, signing_key } =
                rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
            let certificate = reqwest::Certificate::from_der(cert.der()).unwrap();
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
            (Some(Arc::new(config)), Some(certificate))
        } else {
            (None, None)
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "{}://{}",
            if https { "https" } else { "http" },
            listener.local_addr().unwrap()
        );
        listener.set_nonblocking(true).unwrap();
        let (stop, stopped) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut requests = Vec::new();
            let mut responses = responses.into_iter();
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let response = responses.next().unwrap_or_else(|| response(200, None));
                        let request = if let Some(config) = &tls {
                            let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                            exchange(&mut rustls::StreamOwned::new(connection, stream), &response)
                        } else {
                            exchange(&mut stream, &response)
                        };
                        requests.push(request);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        match stopped.recv_timeout(Duration::from_millis(5)) {
                            Err(mpsc::RecvTimeoutError::Timeout) => (),
                            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    Err(error) => panic!("Accept notification test request: {error}"),
                }
            }
            requests
        });
        Self {
            url,
            certificate,
            stop,
            thread,
        }
    }

    fn client(&self) -> reqwest::blocking::Client {
        client_builder()
            .no_proxy()
            .add_root_certificate(self.certificate.clone().unwrap())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    fn finish(self) -> Vec<String> {
        self.stop.send(()).unwrap();
        self.thread.join().unwrap()
    }
}

fn response(status: u16, location: Option<&str>) -> String {
    let location = location.map_or_else(String::new, |url| format!("Location: {url}\r\n"));
    format!("HTTP/1.1 {status} Test\r\n{location}Content-Length: 0\r\nConnection: close\r\n\r\n")
}

fn exchange(stream: &mut (impl Read + Write), response: &str) -> String {
    let mut request = Vec::new();
    let mut chunk = [0; 2048];
    loop {
        let size = stream.read(&mut chunk).unwrap();
        assert_ne!(size, 0, "Incomplete notification test request");
        request.extend_from_slice(&chunk[..size]);
        assert!(request.len() < 16384, "Unexpectedly large test request");
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&request[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                break;
            }
        }
    }
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
    String::from_utf8(request).unwrap()
}

#[test]
fn notification_transport_rejects_plain_http() {
    let sink = Server::start(false, vec![]);
    let result = client_builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(&sink.url)
        .body(MESSAGE)
        .send();
    let leaked = sink.finish();
    assert!(result.is_err());
    assert!(leaked.is_empty(), "Plain HTTP leaked a request: {leaked:?}");
}

#[test]
fn notification_transport_rejects_http_redirects() {
    for status in [301, 302, 303, 307, 308] {
        let sink = Server::start(false, vec![]);
        let server = Server::start(true, vec![response(status, Some(&sink.url))]);
        let result = server.client().post(&server.url).body(MESSAGE).send();
        let requests = server.finish();
        let leaked = sink.finish();
        assert!(
            leaked.is_empty(),
            "HTTP {status} leaked a request: {leaked:?}"
        );
        assert!(
            result.unwrap_err().is_redirect(),
            "HTTP {status} must refuse the redirect"
        );
        assert_eq!(requests.len(), 1);
        assert!(requests[0].ends_with(MESSAGE));
    }
}

#[test]
fn notification_transport_rejects_downgrade_after_https_hop() {
    let sink = Server::start(false, vec![]);
    let server = Server::start(
        true,
        vec![
            response(307, Some("/next")),
            response(308, Some(&sink.url.replacen("http:", "HTTP:", 1))),
        ],
    );
    let result = server.client().post(&server.url).body(MESSAGE).send();
    let requests = server.finish();
    let leaked = sink.finish();
    assert!(
        leaked.is_empty(),
        "Chained downgrade leaked a request: {leaked:?}"
    );
    assert!(result.unwrap_err().is_redirect());
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.ends_with(MESSAGE)));
}

#[test]
fn notification_transport_keeps_https_delivery_and_redirects() {
    for status in [200, 307, 308] {
        let server = Server::start(true, vec![response(status, Some("/next"))]);
        let result = server
            .client()
            .post(&server.url)
            .bearer_auth("synthetic-test-token")
            .body(MESSAGE)
            .send();
        let requests = server.finish();
        assert!(result.unwrap().status().is_success());
        assert_eq!(requests.len(), if status == 200 { 1 } else { 2 });
        for request in requests {
            assert!(request.starts_with("POST "));
            assert!(request.contains("authorization: Bearer synthetic-test-token\r\n"));
            assert!(request.ends_with(MESSAGE));
        }
    }
}

#[test]
fn notification_transport_keeps_https_form_posts() {
    let server = Server::start(true, vec![]);
    let result = server
        .client()
        .post(&server.url)
        .form(&[
            ("token", "synthetic-token"),
            ("user", "synthetic-user"),
            ("message", MESSAGE),
        ])
        .send();
    let requests = server.finish();
    assert!(result.unwrap().status().is_success());
    assert_eq!(requests.len(), 1);
    assert!(requests[0].ends_with(
        "token=synthetic-token&user=synthetic-user&message=synthetic+notification+diagnostics"
    ));
}
