#![cfg(feature = "gui")]
// Exercise the real packaged executable and the same controller used by the GUI.
// These private GUI modules are shared here so lifecycle tests can inspect worker PIDs.
#![allow(dead_code)]
#[path = "../src/gui/document.rs"]
mod document;
#[path = "../src/gui/find.rs"]
mod find;
#[path = "../src/gui/focus.rs"]
mod focus;
#[path = "../src/gui/fonts.rs"]
mod fonts;
#[path = "../src/gui/forms.rs"]
mod forms;
#[path = "../src/gui/render.rs"]
mod render;
#[path = "../src/gui/worker.rs"]
mod worker;
use document::ClickRequest;
use eframe::egui;
use olive_html::{NodeId, net::Location};
use std::{
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    time::{Duration, Instant},
};
use worker::{Command, Event, Worker};

const HTML: &str = "<!doctype html><title>Before</title><script>let count=1; document.title='Loaded '+count; function increment(){count++; document.title='Clicked '+count; alert(count)}</script><button id=increment onclick='increment(); return false'>Increment</button><p>A readable article.</p>";

fn spawn(path: &Path) -> Worker {
    Worker::spawn_at(
        Path::new(env!("CARGO_BIN_EXE_olive-gui")),
        Location::from_path(path).unwrap(),
        true,
        &egui::Context::default(),
    )
    .unwrap()
}
fn reply(worker: &mut Worker) -> Result<Event, String> {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(event) = worker.poll() {
            return event;
        }
        assert!(
            Instant::now() < until,
            "worker {} did not reply",
            worker.pid().unwrap_or(0)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn target() -> NodeId {
    let doc = olive_html::parse(HTML).unwrap().document;
    doc.descendants(doc.root())
        .find(|&id| {
            doc.node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.attribute("id") == Some("increment"))
        })
        .unwrap()
}
fn loaded(worker: &mut Worker) {
    match reply(worker).unwrap() {
        Event::Loaded(page) => {
            assert_eq!(page.page.title, "Loaded 1");
            assert_eq!(page.scripts.executed, 1);
            assert!(page.scripting_enabled);
        }
        _ => panic!("expected loaded page"),
    }
}
fn click(worker: &mut Worker, count: usize) {
    worker
        .send(Command::Click(ClickRequest {
            target: target(),
            href: Some("/must-not-navigate".into()),
        }))
        .unwrap();
    match reply(worker).unwrap() {
        Event::Updated(update) => {
            assert_eq!(update.page.title, format!("Clicked {count}"));
            assert_eq!(update.scripts.executed, 1);
            assert!(update.scripts.diagnostics.is_empty());
            assert_eq!(update.alert, Some(count.to_string()));
            assert!(update.link.is_none());
        }
        _ => panic!("expected click update"),
    }
}
fn crash(pid: u32) {
    #[cfg(unix)]
    let status = ProcessCommand::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .unwrap();
    #[cfg(windows)]
    let status = ProcessCommand::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn tabs_have_distinct_processes_and_a_crash_does_not_interrupt_another_realm() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("counter.html");
    std::fs::write(&page, HTML).unwrap();
    let mut first = spawn(&page);
    let mut second = spawn(&page);
    assert_ne!(first.pid(), second.pid());
    assert_ne!(first.pid(), Some(std::process::id()));
    loaded(&mut first);
    loaded(&mut second);
    click(&mut first, 2);
    click(&mut second, 2);
    let crashed_pid = first.pid().unwrap();
    crash(crashed_pid);
    assert!(reply(&mut first).is_err());
    click(&mut second, 3);
    drop(first);
    let mut reloaded = spawn(&page);
    assert_ne!(reloaded.pid(), Some(crashed_pid));
    loaded(&mut reloaded);
    click(&mut reloaded, 2);
    click(&mut second, 4);
}

#[cfg(unix)]
fn exists(pid: u32) -> bool {
    ProcessCommand::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}
#[cfg(unix)]
fn gone(pid: u32) {
    let until = Instant::now() + Duration::from_secs(5);
    while exists(pid) {
        assert!(Instant::now() < until, "orphaned worker {pid}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[cfg(unix)]
#[test]
fn a_suspended_tab_cannot_block_other_tabs_or_closing_the_stalled_tab() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("counter.html");
    std::fs::write(&page, HTML).unwrap();
    let mut first = spawn(&page);
    let mut second = spawn(&page);
    loaded(&mut first);
    loaded(&mut second);
    let pid = first.pid().unwrap();
    assert!(
        ProcessCommand::new("kill")
            .args(["-STOP", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    first
        .send(Command::Click(ClickRequest {
            target: target(),
            href: None,
        }))
        .unwrap();
    assert!(first.poll().is_none());
    click(&mut second, 2);
    assert!(first.busy());
    let before = Instant::now();
    drop(first);
    assert!(before.elapsed() < Duration::from_millis(500));
    gone(pid);
    click(&mut second, 3);
    let second_pid = second.pid().unwrap();
    drop(second);
    gone(second_pid);
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn parent_pipe_closure_terminates_even_a_worker_blocked_loading_a_document() {
    use std::net::TcpListener;
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut child = ChildGuard(
        ProcessCommand::new(env!("CARGO_BIN_EXE_olive-gui"))
            .arg(worker::WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut input = child.0.stdin.take().unwrap();
    worker::write_frame(
        &mut input,
        &Command::Load {
            location: Location::from_input(&format!("http://{}/", server.local_addr().unwrap()))
                .unwrap(),
            scripting: false,
        },
        32768,
    )
    .unwrap();
    // Wait until the child really is blocked on a response, rather than only testing idle EOF.
    server.set_nonblocking(true).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    let connection = loop {
        if let Ok((connection, _)) = server.accept() {
            break connection;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(input);
    let until = Instant::now() + Duration::from_secs(5);
    while child.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < until,
            "worker survived parent pipe closure"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(connection);
}

#[test]
fn decoded_images_and_reading_views_survive_the_process_boundary() {
    let mut worker = spawn(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/reading.html"));
    match reply(&mut worker).unwrap() {
        Event::Loaded(page) => {
            assert_eq!(page.page.image_count(), 1);
            assert_eq!(page.reading.image_count(), 1);
            let ctx = egui::Context::default();
            let mut page = page;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                page.page.show(ui);
            });
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        }
        _ => panic!("expected image page"),
    }
}

#[cfg(unix)]
#[test]
fn the_watchdog_terminates_a_hung_tab_without_resetting_other_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("counter.html");
    std::fs::write(&page, HTML).unwrap();
    let mut hung = spawn(&page);
    let mut healthy = spawn(&page);
    loaded(&mut hung);
    loaded(&mut healthy);
    let pid = hung.pid().unwrap();
    assert!(
        ProcessCommand::new("kill")
            .args(["-STOP", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    hung.send(Command::Click(ClickRequest {
        target: target(),
        href: None,
    }))
    .unwrap();
    click(&mut healthy, 2);
    assert!(
        reply(&mut hung)
            .err()
            .unwrap()
            .contains("stopped responding")
    );
    gone(pid);
    click(&mut healthy, 3);
}

#[test]
fn form_submission_crosses_a_fresh_worker_and_reload_uses_get() {
    use olive_html::net::FormRequest;
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let action = format!("http://{}/submit", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut methods = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            let headers = String::from_utf8(headers).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            methods.push((headers.lines().next().unwrap().to_owned(), body));
            let body = b"<title>Submitted</title><p>Form received</p><script>document.title='Must stay disabled'</script>";
            write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
            stream.write_all(body).unwrap();
        }
        methods
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("form.html");
    std::fs::write(&path, format!("<form action='{action}' method=post><input name=q value=initial><button>Send</button></form>")).unwrap();
    let mut original = spawn(&path);
    let mut page = match reply(&mut original).unwrap() {
        Event::Loaded(page) => page,
        _ => panic!("expected form"),
    };
    page.page.forms.controls[0].value = "edited & שלום".into();
    let request = page
        .page
        .forms
        .submit(page.page.forms.controls[1].node, &page.location, &page.base)
        .unwrap();
    let body = request.body.clone().unwrap();
    let executable = Path::new(env!("CARGO_BIN_EXE_olive-gui"));
    let ctx = egui::Context::default();
    let mut submitted =
        Worker::spawn_command_at(executable, Command::Submit(request), &ctx).unwrap();
    assert_ne!(original.pid(), submitted.pid());
    let loaded = match reply(&mut submitted).unwrap() {
        Event::Loaded(page) => page,
        _ => panic!("expected response"),
    };
    assert_eq!(loaded.page.title, "Submitted");
    assert!(!loaded.scripting_enabled);
    let mut reloaded = Worker::spawn_at(executable, loaded.location, false, &ctx).unwrap();
    assert!(matches!(reply(&mut reloaded).unwrap(), Event::Loaded(_)));
    let requests = server.join().unwrap();
    assert_eq!(
        requests[0],
        ("POST /submit HTTP/1.1".into(), body.into_bytes())
    );
    assert_eq!(requests[1], ("GET /submit HTTP/1.1".into(), Vec::new()));
    // The command wire payload preserves ordinary URL-encoded bytes, including Unicode.
    let encoded = serde_json::to_vec(&Command::Submit(FormRequest {
        location: Location::from_input(&action).unwrap(),
        body: Some("q=%D7%A9".into()),
    }))
    .unwrap();
    assert!(matches!(
        serde_json::from_slice::<Command>(&encoded).unwrap(),
        Command::Submit(_)
    ));
}
