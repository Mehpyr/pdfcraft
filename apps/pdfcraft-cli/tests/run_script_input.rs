//! `pdfcraft-cli run --script`: a script saved with a UTF-8 byte order mark, as Windows PowerShell
//! 5.1 writes one (`Set-Content -Encoding utf8`), or in another encoding, and a script read from
//! standard input (`--script -`).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_pdfcraft-cli");
const BOM: &[u8] = b"\xEF\xBB\xBF";
/// With and without `--report`.
const MODES: [&[&str]; 2] = [&[], &["--report"]];

/// An empty folder for one test, removed afterwards.
struct TestDir(PathBuf);

impl TestDir {
    fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pdfcraft-cli-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.0.join(name), bytes).unwrap();
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A blank page, then the list of open documents.
fn steps() -> Vec<u8> {
    serde_json::json!([
        { "tool": "doc_create", "args": { "from": "blank", "width": 100, "height": 100 } },
        { "tool": "doc_list", "args": {} },
    ])
    .to_string()
    .into_bytes()
}

/// Run `pdfcraft-cli run --script <script> <extra…>` in `dir`, with `input` on its standard input.
fn run(dir: &TestDir, script: &str, extra: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(BIN)
        .current_dir(&dir.0)
        .args(["run", "--script", script])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    // Written from another thread so the child's output can't fill up while this one waits on it.
    // The child may stop reading early (a file script never reads it): that is not an error here.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    out
}

/// The JSON documents a run printed, one per step.
fn documents(out: &Output) -> Vec<serde_json::Value> {
    serde_json::Deserializer::from_slice(&out.stdout).into_iter::<serde_json::Value>().collect::<Result<_, _>>().unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_script_file_may_start_with_a_byte_order_mark() {
    let dir = TestDir::new("script-bom");
    dir.write("steps.json", &[BOM, steps().as_slice()].concat());
    let out = run(&dir, "steps.json", &[], b"");
    assert!(out.status.success(), "{}", stderr(&out));
    let docs = documents(&out);
    assert_eq!(docs.len(), 2, "{docs:?}");
    assert_eq!(docs[1]["documents"][0]["pages"], 1, "{docs:?}");

    // One mark is skipped, not any number of them.
    dir.write("twice.json", &[BOM, BOM, steps().as_slice()].concat());
    let out = run(&dir, "twice.json", &[], b"");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out).trim(), "pdfcraft-cli: twice.json: expected value at line 1 column 1");
    assert!(out.stdout.is_empty());
}

#[test]
fn a_script_that_is_not_utf8_is_refused_with_how_to_save_it() {
    let dir = TestDir::new("script-not-utf8");
    // What Windows PowerShell 5.1's `>` and `Out-File` write unless told otherwise: UTF-16.
    let text = String::from_utf8(steps()).unwrap();
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect();
    dir.write("utf16.json", &utf16);
    // And what its `Set-Content` writes: the ANSI code page, here `ü` as Windows-1252's 0xFC.
    dir.write("ansi.json", b"[{\"tool\": \"doc_set_info\", \"args\": {\"doc\": 1, \"key\": \"Title\", \"value\": \"Z\xFCrich\"}}]");
    for (script, found) in [("utf16.json", "utf16.json: is UTF-16;"), ("ansi.json", "ansi.json: is not UTF-8 (")] {
        let out = run(&dir, script, &[], b"");
        assert_eq!(out.status.code(), Some(1), "{script}");
        let stderr = stderr(&out);
        assert!(stderr.contains(found) && stderr.contains("save the script as UTF-8"), "{script}: {stderr}");
        assert!(out.stdout.is_empty(), "{script}");
    }
}

#[test]
fn a_dash_reads_the_script_from_stdin() {
    let dir = TestDir::new("script-stdin");
    let out = run(&dir, "-", &[], &steps());
    assert!(out.status.success(), "{}", stderr(&out));
    let docs = documents(&out);
    assert_eq!(docs.len(), 2, "{docs:?}");
    assert_eq!(docs[1]["documents"][0]["pages"], 1, "{docs:?}");

    // With a report, and with the byte order mark Windows PowerShell 5.1 sends down a pipe once
    // `$OutputEncoding` is UTF-8.
    let out = run(&dir, "-", &["--report"], &[BOM, steps().as_slice()].concat());
    assert!(out.status.success(), "{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is one JSON document");
    assert_eq!(report["ok"], true, "{report}");
    let statuses: Vec<_> = report["steps"].as_array().unwrap().iter().map(|s| s["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["ok", "ok"], "{report}");
    // `-` is stdin, not a file of that name.
    assert!(!dir.0.join("-").exists());
}

#[test]
fn stdin_and_a_file_run_and_report_alike() {
    let dir = TestDir::new("script-same");
    // A failing step, so the error and the skipped step after it are compared too.
    let script = serde_json::json!([
        { "tool": "doc_create", "args": { "from": "blank", "width": 100, "height": 100 } },
        { "tool": "page_rotate", "args": { "doc": 1, "pages": [999], "degrees": 90 } },
        { "tool": "doc_close", "args": { "doc": 1 } },
    ])
    .to_string();
    dir.write("steps.json", script.as_bytes());
    for extra in MODES {
        let from_file = run(&dir, "steps.json", extra, b"");
        assert_eq!(from_file.status.code(), Some(1), "{extra:?}");
        assert!(stderr(&from_file).contains("step 2 (page_rotate)"), "{extra:?}: {}", stderr(&from_file));
        let from_stdin = run(&dir, "-", extra, script.as_bytes());
        assert_eq!(from_stdin.status.code(), from_file.status.code(), "{extra:?}");
        assert_eq!(stderr(&from_stdin), stderr(&from_file), "{extra:?}");
        assert_eq!(String::from_utf8_lossy(&from_stdin.stdout), String::from_utf8_lossy(&from_file.stdout), "{extra:?}");
    }
}

#[test]
fn a_script_on_stdin_that_is_not_json_names_stdin() {
    let dir = TestDir::new("script-stdin-invalid");
    for extra in MODES {
        let out = run(&dir, "-", extra, b"steps go here");
        assert_eq!(out.status.code(), Some(1), "{extra:?}");
        assert_eq!(stderr(&out).trim(), "pdfcraft-cli: stdin: expected value at line 1 column 1", "{extra:?}");
        assert!(out.stdout.is_empty(), "{extra:?}: nothing ran");

        for input in [&b""[..], b"[{\"tool\": \"doc_list\"}"] {
            let out = run(&dir, "-", extra, input);
            assert_eq!(out.status.code(), Some(1), "{extra:?}");
            let stderr = stderr(&out);
            assert!(stderr.starts_with("pdfcraft-cli: stdin: EOF while parsing"), "{extra:?}: {stderr}");
            assert!(out.stdout.is_empty(), "{extra:?}: nothing ran");
        }
    }
}
