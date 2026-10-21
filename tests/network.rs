#![cfg(feature = "gui")]
// Exercise the real network process that the browser starts from its own
// executable.
use olive_html::gui::network::{Fetch, Network};
use olive_html::net::{CookieJar, Location};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::atomic::AtomicBool,
    time::Duration,
};

fn network() -> Network {
    Network::new(env!("CARGO_BIN_EXE_olive-gui"))
}

/// Answer `count` requests with `body`, returning each request's first line.
fn serve(count: usize, body: Vec<u8>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..count {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut headers = String::new();
            while !headers.ends_with("\r\n\r\n") {
                if reader.read_line(&mut headers).unwrap() == 0 {
                    break;
                }
            }
            requests.push(headers.lines().next().unwrap_or_default().to_owned());
            let stream = reader.get_mut();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
        requests
    });
    (address, server)
}

#[test]
fn the_network_process_fetches_documents_and_streams_downloads() {
    let body: Vec<u8> = (0..300_000_u32).map(|n| b'a' + (n % 26) as u8).collect();
    let (address, server) = serve(3, body.clone());
    let network = network();
    let page = Location::from_input(&format!("{address}/page")).unwrap();
    let response = network
        .fetch(
            Fetch::Document {
                location: page.clone(),
                initiator: None,
            },
            CookieJar::default(),
            Duration::from_secs(10),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(response.location, page);
    assert_eq!(response.bytes, body);

    let mut downloaded = Vec::new();
    let mut announced = None;
    let file = Location::from_input(&format!("{address}/file.bin")).unwrap();
    let size = network
        .download(file.clone(), |bytes, total| {
            announced = total;
            downloaded.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
    assert_eq!(size, body.len() as u64);
    assert_eq!(announced, Some(body.len() as u64));
    assert_eq!(downloaded, body);

    // Refusing a piece cancels the download.
    let error = network
        .download(file, |_, _| Err("Download cancelled.".into()))
        .unwrap_err();
    assert_eq!(error, "Download cancelled.");
    assert_eq!(
        server.join().unwrap(),
        [
            "GET /page HTTP/1.1",
            "GET /file.bin HTTP/1.1",
            "GET /file.bin HTTP/1.1"
        ]
    );
}

#[test]
fn a_stopped_fetch_returns_without_waiting_for_the_website() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/slow", listener.local_addr().unwrap());
    // Accept and never answer.
    let _server = std::thread::spawn(move || {
        let connection = listener.accept();
        std::thread::sleep(Duration::from_secs(20));
        drop(connection);
    });
    let stopped = AtomicBool::new(false);
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(500));
            stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let error = network()
            .fetch(
                Fetch::Document {
                    location: Location::from_input(&address).unwrap(),
                    initiator: None,
                },
                CookieJar::default(),
                Duration::from_secs(15),
                &stopped,
            )
            .err()
            .unwrap();
        assert_eq!(error, "Loading stopped.");
    });
    assert!(started.elapsed() < Duration::from_secs(5));
}
