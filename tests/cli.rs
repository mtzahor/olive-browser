use std::{
    io::Write,
    process::{Command, Stdio},
};

fn run(input: &[u8], args: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_olive"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn reads_stdin_and_recovers_html_successfully() {
    let output = run(b"<p>Olive &amp; Rust", &["-"]);
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("\"Olive & Rust\"")
    );
    assert!(!output.stderr.is_empty());
}

#[test]
fn reads_example_file() {
    let output = run(
        b"",
        &[concat!(env!("CARGO_MANIFEST_DIR"), "/examples/hello.html")],
    );
    assert!(output.status.success(), "{:?}", output);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("<title>")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn help_and_version() {
    assert!(
        String::from_utf8(run(b"", &["--help"]).stdout)
            .unwrap()
            .contains("Usage:")
    );
    assert_eq!(
        run(b"", &["--version"]).stdout,
        format!("olive {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
}

#[test]
fn input_and_usage_errors_exit_unsuccessfully() {
    for args in [
        &["--unknown"][..],
        &["one", "two"][..],
        &["/olive-missing-file.html"][..],
    ] {
        let output = run(b"", args);
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
    let output = run(&vec![b'x'; 1024 * 1024 + 1], &[]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("1048576-byte limit")
    );
    assert!(output.stdout.is_empty());
}
