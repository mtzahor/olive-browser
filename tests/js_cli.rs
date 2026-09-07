#![cfg(feature = "js")]
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn run(source: &[u8], args: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_olive-js"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn evaluates_stdin_and_files_and_check_is_inert() {
    let output = run(b"console.log('hello'); 6*7", &["-"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    assert_eq!(output.stderr, b"hello\n");
    let output = run(b"console.log('not run'); throw 1", &["--check"]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    let output = run(
        b"",
        &[concat!(env!("CARGO_MANIFEST_DIR"), "/examples/hello.js")],
    );
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Kalamata")
    );
}
#[test]
fn errors_and_control_characters_are_reported_safely() {
    for (input, args) in [
        (&b"let = ;"[..], &[][..]),
        (&b"throw 1"[..], &[][..]),
        (&b"while(true){}"[..], &[][..]),
        (&b""[..], &["--unknown"][..]),
        (&b""[..], &["one", "two"][..]),
        (&b""[..], &["/olive-missing.js"][..]),
        (&b"\xff"[..], &[][..]),
    ] {
        assert!(!run(input, args).status.success());
    }
    let output = run(b"console.log('\x1b[2J'); '\x07'", &[]);
    assert!(output.status.success());
    assert!(!output.stdout.contains(&7));
    assert!(!output.stderr.contains(&27));
    assert!(run(b"", &["--help"]).status.success());
}
