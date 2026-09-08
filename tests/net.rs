#![cfg(feature = "net")]

use flate2::{Compression, write::GzEncoder};
use olive_html::net::{DocumentLoader, Location, MAX_DOCUMENT_BYTES};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

fn server(responses: Vec<Vec<u8>>) -> (Location, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let location =
        Location::from_input(&format!("http://{}/start", listener.local_addr().unwrap())).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
                let mut byte = [0];
                if stream.read(&mut byte).unwrap() == 0 {
                    break;
                }
                request.push(byte[0]);
            }
            requests.push(String::from_utf8(request).unwrap());
            // A bounded client can intentionally close while we send a large body.
            let _ = stream.write_all(&response);
        }
        requests
    });
    (location, handle)
}

fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

#[test]
fn fetches_html_follows_relative_redirects_and_preserves_final_url() {
    let (location, server) = server(vec![
        response("302 Found", "Location: /docs/page?q=olive\r\n", b""),
        response(
            "200 OK",
            "Content-Type: text/html; charset=utf-8\r\n",
            b"<!doctype html><title>Online</title><a href='../next'>Next</a>",
        ),
    ]);
    let source = DocumentLoader::new()
        .unwrap()
        .load(location.resolve("#section").unwrap())
        .unwrap();
    assert_eq!(source.location.url().path(), "/docs/page");
    assert_eq!(source.location.url().query(), Some("q=olive"));
    assert_eq!(source.location.url().fragment(), Some("section"));
    assert_eq!(source.status, Some(200));
    assert!(source.parse(false).is_ok());
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /start HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("GET /docs/page?q=olive HTTP/1.1\r\n"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("user-agent: olivebrowser/0.1.0")
    );
    assert!(!requests[1].to_ascii_lowercase().contains("referer:"));
}

#[test]
fn http_error_documents_are_readable_and_plain_text_stays_inert() {
    let (location, server) = server(vec![response(
        "404 Not Found",
        "Content-Type: text/plain; charset=windows-1252\r\n",
        b"caf\xe9 <script>alert('inert')</script>",
    )]);
    let source = DocumentLoader::new().unwrap().load(location).unwrap();
    assert_eq!(source.status, Some(404));
    assert!(source.plain_text);
    let mut tree = Vec::new();
    source
        .parse(false)
        .unwrap()
        .document
        .write_tree(&mut tree)
        .unwrap();
    let tree = String::from_utf8(tree).unwrap();
    assert!(tree.contains("café <script>"));
    assert!(!tree.contains("\n      <script>"));
    server.join().unwrap();
}

#[test]
fn redirects_keep_new_fragments_and_allow_exactly_ten_hops() {
    let mut responses = vec![response("302 Found", "Location: /middle#new\r\n", b"")];
    responses.extend((0..9).map(|i| {
        response(
            "307 Temporary Redirect",
            &format!("Location: /step{i}\r\n"),
            b"",
        )
    }));
    responses.push(response(
        "200 OK",
        "Content-Type: text/html\r\n",
        b"<p>Arrived",
    ));
    let (location, server) = server(responses);
    let source = DocumentLoader::new()
        .unwrap()
        .load(location.resolve("#old").unwrap())
        .unwrap();
    assert_eq!(source.location.fragment().as_deref(), Some("new"));
    assert_eq!(source.location.url().path(), "/step8");
    assert_eq!(server.join().unwrap().len(), 11);
}

#[test]
fn decompresses_gzip_and_bounds_the_expanded_body() {
    for size in [128, MAX_DOCUMENT_BYTES + 1] {
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&vec![b'x'; size]).unwrap();
        let compressed = gzip.finish().unwrap();
        let (location, server) = server(vec![response(
            "200 OK",
            "Content-Type: text/html\r\nContent-Encoding: gzip\r\n",
            &compressed,
        )]);
        let loaded = DocumentLoader::new().unwrap().load(location);
        if size > MAX_DOCUMENT_BYTES {
            assert!(loaded.err().unwrap().contains("byte limit"));
        } else {
            assert_eq!(loaded.unwrap().bytes, vec![b'x'; size]);
        }
        server.join().unwrap();
    }
}

#[test]
fn reads_chunked_documents_and_rejects_invalid_chunks() {
    for (body, valid) in [
        ("5\r\nHello\r\n0\r\n\r\n", true),
        ("Z\r\nHello\r\n0\r\n\r\n", false),
    ] {
        let raw = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\n\r\n{body}"
        );
        let (location, server) = server(vec![raw.into_bytes()]);
        let loaded = DocumentLoader::new().unwrap().load(location);
        if valid {
            assert_eq!(loaded.unwrap().bytes, b"Hello");
        } else {
            assert!(loaded.is_err());
        }
        server.join().unwrap();
    }
}

#[test]
fn rejects_binary_oversized_and_truncated_responses() {
    for raw in [
        response("200 OK", "Content-Type: image/png\r\n", b"PNG"),
        response(
            "200 OK",
            "Content-Type: text/html\r\n",
            &vec![b'x'; MAX_DOCUMENT_BYTES + 1],
        ),
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".to_vec(),
        [
            b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".as_slice(),
            &vec![b'x'; MAX_DOCUMENT_BYTES + 1],
        ]
        .concat(),
    ] {
        let (location, server) = server(vec![raw]);
        assert!(DocumentLoader::new().unwrap().load(location).is_err());
        server.join().unwrap();
    }
}

#[test]
fn rejects_redirect_loops_and_forbidden_redirects() {
    let redirect = response("302 Found", "Location: /loop\r\n", b"");
    let (location, server) = server(vec![redirect; 11]);
    assert!(DocumentLoader::new().unwrap().load(location).is_err());
    assert_eq!(server.join().unwrap().len(), 11);
    for target in [
        "file:///etc/passwd",
        "ftp://example.com",
        "http://user:password@example.com/",
    ] {
        let (location, server) = self::server(vec![response(
            "302 Found",
            &format!("Location: {target}\r\n"),
            b"",
        )]);
        assert!(DocumentLoader::new().unwrap().load(location).is_err());
        server.join().unwrap();
    }
}

#[test]
fn requests_time_out_without_waiting_forever() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let location =
        Location::from_input(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(300));
    });
    let error = DocumentLoader::with_timeout(Duration::from_millis(75))
        .unwrap()
        .load(location)
        .err()
        .unwrap();
    assert!(error.contains("too long"), "{error}");
    server.join().unwrap();
}

#[test]
fn cli_reads_an_http_url_without_executing_scripts() {
    let (location, server) = server(vec![response(
        "200 OK",
        "Content-Type: text/html\r\n",
        b"<!doctype html><p>Online CLI</p><script>throw Error('must not run')</script>",
    )]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_olive"))
        .arg(location.as_str())
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Online CLI")
    );
    assert!(output.stderr.is_empty());
    server.join().unwrap();
}
