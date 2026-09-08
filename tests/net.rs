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
    assert!(requests[0].to_ascii_lowercase().contains(&format!(
        "user-agent: olivebrowser/{}",
        env!("CARGO_PKG_VERSION")
    )));
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

#[test]
fn loads_typed_resources_with_redirects_and_decoding() {
    use olive_html::net::{MAX_RESOURCE_BYTES, ResourceKind};
    let (location, server) = server(vec![
        response("302 Found", "Location: /assets/theme.css\r\n", b""),
        response(
            "200 OK",
            "Content-Type: text/css; charset=windows-1252\r\n",
            b"/* caf\xe9 */ p {color:red}",
        ),
        response(
            "200 OK",
            "Content-Type: application/javascript\r\n",
            b"console.log('loaded')",
        ),
    ]);
    let loader = DocumentLoader::new().unwrap();
    let css = loader
        .load_resource(
            &location,
            location.clone(),
            ResourceKind::Stylesheet,
            Duration::from_secs(2),
            MAX_RESOURCE_BYTES,
        )
        .unwrap();
    assert_eq!(css.location.url().path(), "/assets/theme.css");
    assert!(css.source.contains("café"));
    let js = loader
        .load_resource(
            &location,
            location.resolve("app.js").unwrap(),
            ResourceKind::Script,
            Duration::from_secs(2),
            MAX_RESOURCE_BYTES,
        )
        .unwrap();
    assert_eq!(js.source, "console.log('loaded')");
    let requests = server.join().unwrap();
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("accept: text/css")
    );
    assert!(requests[2].starts_with("GET /app.js "));
    assert!(!requests[2].to_ascii_lowercase().contains("referer:"));
}

#[test]
fn loads_png_image_resources_as_original_bytes() {
    use olive_html::net::{MAX_RESOURCE_BYTES, ResourceKind};
    let expected = include_bytes!("../assets/olive-browser.png");
    let (location, server) = server(vec![response(
        "200 OK",
        "Content-Type: image/png\r\n",
        expected,
    )]);
    let loaded = DocumentLoader::new()
        .unwrap()
        .load_resource(
            &location,
            location.resolve("olive.png").unwrap(),
            ResourceKind::Image,
            Duration::from_secs(2),
            MAX_RESOURCE_BYTES,
        )
        .unwrap();
    assert!(loaded.source.is_empty());
    assert_eq!(loaded.bytes, expected);
    assert!(
        server.join().unwrap()[0]
            .to_ascii_lowercase()
            .contains("accept: image/png")
    );
}

#[test]
fn resource_errors_mime_limits_and_origin_boundaries() {
    use olive_html::net::{MAX_RESOURCE_BYTES, ResourceKind};
    let loader = DocumentLoader::new().unwrap();
    for raw in [
        response(
            "404 Not Found",
            "Content-Type: text/css\r\n",
            b"p{color:red}",
        ),
        response("200 OK", "Content-Type: text/html\r\n", b"p{color:red}"),
        response("200 OK", "", b"p{color:red}"),
        response(
            "200 OK",
            "Content-Type: text/css\r\n",
            &vec![b'x'; MAX_RESOURCE_BYTES + 1],
        ),
        response(
            "200 OK",
            "Content-Type: text/css; charset=windows-1252\r\n",
            &vec![0xe9; MAX_RESOURCE_BYTES],
        ),
        response("302 Found", "Location: file:///secret.css\r\n", b""),
        response(
            "302 Found",
            "Location: http://user:pass@example.com/a.css\r\n",
            b"",
        ),
    ] {
        let (location, server) = server(vec![raw]);
        assert!(
            loader
                .load_resource(
                    &location,
                    location.clone(),
                    ResourceKind::Stylesheet,
                    Duration::from_secs(2),
                    MAX_RESOURCE_BYTES
                )
                .is_err()
        );
        server.join().unwrap();
    }
    let secure = Location::from_input("https://example.com").unwrap();
    let local_file = if cfg!(windows) {
        "file:///C:/secret.js"
    } else {
        "file:///secret.js"
    };
    for target in ["http://127.0.0.1:1/a.js", local_file] {
        assert!(
            loader
                .load_resource(
                    &secure,
                    Location::from_input(target).unwrap(),
                    ResourceKind::Script,
                    Duration::from_secs(2),
                    MAX_RESOURCE_BYTES
                )
                .is_err()
        );
    }
    let (location, server) = server(vec![response(
        "200 OK",
        "Content-Type: text/css\r\n",
        b"123456789",
    )]);
    assert!(
        loader
            .load_resource(
                &location,
                location.clone(),
                ResourceKind::Stylesheet,
                Duration::from_secs(2),
                8
            )
            .is_err()
    );
    server.join().unwrap();
}

#[test]
fn resource_compression_obeys_decoded_limit() {
    use olive_html::net::ResourceKind;
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&[b'x'; 512]).unwrap();
    let (location, server) = server(vec![response(
        "200 OK",
        "Content-Type: text/javascript\r\nContent-Encoding: gzip\r\n",
        &gzip.finish().unwrap(),
    )]);
    assert!(
        DocumentLoader::new()
            .unwrap()
            .load_resource(
                &location,
                location.clone(),
                ResourceKind::Script,
                Duration::from_secs(2),
                128
            )
            .is_err()
    );
    server.join().unwrap();
}

#[cfg(all(feature = "css", feature = "js"))]
mod page_resources {
    use super::*;
    use olive_html::{
        css::{Color, ComputedStyle, StyleBudget, Stylesheet},
        js::{DocumentSession, ScriptOptions},
        resources::{MAX_RESOURCES, PageResources},
    };

    #[test]
    fn page_load_merges_css_order_and_shares_inline_external_script_state() {
        let (location, server) = server(vec![
            response("200 OK", "Content-Type: text/html\r\n", br#"<!doctype html><base href='/assets/'>
                <style>#result {color:red}</style><link rel='StyleSheet' href='theme.css'><style>#result {color:blue}</style>
                <script>let total=4;</script><script src='app.js'>throw 'fallback must not run'</script>
                <script>document.title='Total '+total;</script><button id='result' onclick='add()'>Before</button>"#),
            response("200 OK", "Content-Type: text/css\r\n", b"#result {color:green; background-color:yellow}"),
            response("200 OK", "Content-Type: text/javascript\r\n", b"total+=3; function add(){total++; document.getElementById('result').textContent=total; console.log(total)}"),
        ]);
        let loader = DocumentLoader::new().unwrap();
        let loaded = loader.load(location).unwrap();
        let document = loaded.parse(true).unwrap().document;
        let resources = PageResources::load(&loader, &loaded.location, &document, true);
        assert_eq!(resources.report.loaded, 2);
        assert!(resources.report.diagnostics.is_empty());
        let button = document
            .descendants(document.root())
            .find(|&id| {
                document
                    .node(id)
                    .unwrap()
                    .as_element()
                    .is_some_and(|e| e.attribute("id") == Some("result"))
            })
            .unwrap();
        let sheet = Stylesheet::from_document_with_sources(&document, &resources.styles);
        let style = sheet.compute(
            &document,
            button,
            ComputedStyle::default(),
            17.0,
            &mut StyleBudget::default(),
        );
        assert_eq!(style.color, Color(0, 0, 255, 255));
        let mut session =
            DocumentSession::with_sources(document, ScriptOptions::default(), &resources.scripts);
        assert_eq!(session.report().executed, 3);
        assert!(session.report().diagnostics.is_empty());
        assert!(session.click(button));
        assert!(session.click(button));
        assert_eq!(session.report().console, ["8", "9"]);
        let requests = server.join().unwrap();
        assert!(requests[1].starts_with("GET /assets/theme.css "));
        assert!(requests[2].starts_with("GET /assets/app.js "));
    }

    #[test]
    fn disabled_scripts_and_inapplicable_resources_are_not_requested() {
        let (location, server) = server(vec![response(
            "200 OK",
            "Content-Type: text/css\r\n",
            b"p{color:red}",
        )]);
        let doc = olive_html::parse(r#"<link rel=stylesheet href=good.css>
            <link rel='alternate stylesheet' href=alternate.css><link rel=stylesheet disabled href=disabled.css>
            <link rel=stylesheet media=print href=print.css><link rel=stylesheet type=text/plain href=wrong.css>
            <script src=disabled.js></script><script type=module src=module.js></script>
            <template><link rel=stylesheet href=template.css></template>
            <svg><foreignObject><link rel=stylesheet href=foreign.css></foreignObject></svg>"#).unwrap().document;
        let resources =
            PageResources::load(&DocumentLoader::new().unwrap(), &location, &doc, false);
        assert_eq!(resources.report.attempted, 1);
        assert_eq!(resources.report.loaded, 1);
        assert!(resources.scripts.is_empty());
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn page_load_collects_images_when_scripts_are_disabled() {
        let (location, server) = server(vec![response(
            "200 OK",
            "Content-Type: image/png\r\n",
            include_bytes!("../assets/olive-browser.png"),
        )]);
        let doc = olive_html::parse("<img src=olive.png alt=Olive>")
            .unwrap()
            .document;
        let resources =
            PageResources::load(&DocumentLoader::new().unwrap(), &location, &doc, false);
        assert_eq!(resources.report.attempted, 1);
        assert_eq!(resources.report.loaded, 1);
        assert_eq!(resources.images.len(), 1);
        assert_eq!(
            resources.images.values().next().unwrap().bytes.len(),
            include_bytes!("../assets/olive-browser.png").len()
        );
        server.join().unwrap();
    }

    #[test]
    fn failed_resources_preserve_other_resources_and_inline_execution() {
        let (location, server) = server(vec![
            response(
                "404 Not Found",
                "Content-Type: text/javascript\r\n",
                b"throw '404'",
            ),
            response("200 OK", "Content-Type: text/css\r\n", b"p{color:red}"),
        ]);
        let doc = olive_html::parse("<script src=missing.js>throw 'fallback'</script><link rel=stylesheet href=good.css><script>console.log('continued')</script>").unwrap().document;
        let resources = PageResources::load(&DocumentLoader::new().unwrap(), &location, &doc, true);
        assert_eq!(resources.report.loaded, 1);
        assert_eq!(resources.report.diagnostics.len(), 1);
        let session =
            DocumentSession::with_sources(doc, ScriptOptions::default(), &resources.scripts);
        assert_eq!(session.report().console, ["continued"]);
        assert_eq!(session.report().skipped, 1);
        server.join().unwrap();
    }

    #[test]
    fn resource_count_and_page_deadline_are_shared() {
        let doc = olive_html::parse(
            &"<link rel=stylesheet href='file:///blocked.css'>".repeat(MAX_RESOURCES + 2),
        )
        .unwrap()
        .document;
        let location = Location::from_input("https://example.com").unwrap();
        let loader = DocumentLoader::new().unwrap();
        let resources = PageResources::load(&loader, &location, &doc, true);
        assert_eq!(resources.report.attempted, MAX_RESOURCES);
        assert!(resources.report.limited);
        assert!(resources.report.diagnostics.iter().all(|d| d.len() <= 1024));
        let resources =
            PageResources::load_with_timeout(&loader, &location, &doc, true, Duration::ZERO);
        assert_eq!(resources.report.attempted, 0);
        assert!(resources.report.limited);
    }
}

#[cfg(all(feature = "css", feature = "js"))]
#[test]
fn megabyte_resources_use_separate_per_resource_and_page_budgets() {
    use olive_html::resources::PageResources;
    let (location, server) = server(vec![
        response(
            "200 OK",
            "Content-Type: text/css\r\n",
            &vec![b' '; 7 * 1024 * 1024],
        ),
        response(
            "200 OK",
            "Content-Type: text/css\r\n",
            &vec![b' '; 2 * 1024 * 1024],
        ),
        response("200 OK", "Content-Type: text/css\r\n", b"p{color:red}"),
        response(
            "200 OK",
            "Content-Type: text/javascript\r\n",
            &vec![b' '; 7 * 1024 * 1024],
        ),
        response(
            "200 OK",
            "Content-Type: text/javascript\r\n",
            &vec![b' '; 3 * 1024 * 1024],
        ),
    ]);
    let doc = olive_html::parse("<link rel=stylesheet href=large.css><link rel=stylesheet href=overflow.css><link rel=stylesheet href=small.css><script src=large.js></script><script src=second.js></script>").unwrap().document;
    let resources = PageResources::load(&DocumentLoader::new().unwrap(), &location, &doc, true);
    assert_eq!(resources.report.loaded, 4);
    assert_eq!(resources.report.diagnostics.len(), 1);
    assert!(resources.report.diagnostics[0].contains("Page Stylesheet budget exhausted"));
    assert!(resources.report.diagnostics[0].contains("1048576 bytes remain"));
    assert!(resources.report.limited);
    assert_eq!(resources.scripts.len(), 2);
    server.join().unwrap();
}
