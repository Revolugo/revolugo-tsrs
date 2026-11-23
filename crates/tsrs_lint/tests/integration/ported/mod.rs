//! The ported rules (`src/ported`), through the headless command.

mod camelcase;
mod import_export;
mod no_invalid_this;

use std::io::Write;
use std::process::{Command, Stdio};

use tsrs_core::json::{self, JsonMap, Value};
use tsrs_lint::protocol;

/// What a rule reported in a file: the text it marks and its message.
pub type Reported = Vec<(String, String)>;

/// Writes `files` (path and text) under a directory of the test `name`, with a tsconfig, and returns the directory.
pub fn project(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let base = std::env::current_dir().unwrap().join("target/lint-ported").join(name);
    let _ = std::fs::remove_dir_all(&base);
    let tsconfig = r#"{ "compilerOptions": { "strict": true, "noEmit": true, "module": "esnext", "moduleResolution": "bundler", "target": "es2022", "allowJs": true, "types": [] }, "include": ["."] }"#;
    for (to, text) in std::iter::once(&("tsconfig.json", tsconfig)).chain(files) {
        let to = base.join(to);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::write(to, text).unwrap();
    }
    base
}

/// Runs `rule` with `options` (the JSON of the array of its options) on `file` of the project at `base`.
pub fn lint(base: &std::path::Path, file: &str, rule: &str, options: &str) -> Reported {
    let path = base.join(file).to_string_lossy().into_owned();
    let options = json::unmarshal(options).unwrap();
    let input = json::marshal(&protocol::object([
        ("version", Value::Integer(2)),
        (
            "configs",
            Value::Array(vec![protocol::object([
                ("file_paths", Value::Array(vec![Value::String(path)])),
                (
                    "rules",
                    Value::Array(vec![protocol::object([
                        ("name", Value::String(rule.into())),
                        ("options", options),
                    ])]),
                ),
            ])]),
        ),
    ]))
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tsrs-lint"))
        .arg("headless")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    let mut frames = Vec::new();
    let mut bytes = output.stdout.as_slice();
    while !bytes.is_empty() {
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        frames.push((
            bytes[4],
            json::unmarshal(std::str::from_utf8(&bytes[5..5 + len]).unwrap()).unwrap(),
        ));
        bytes = &bytes[5 + len..];
    }
    assert_eq!(output.status.code(), Some(0), "{frames:?}");
    let text = std::fs::read_to_string(base.join(file)).unwrap();
    frames
        .into_iter()
        .map(|(_, value)| {
            let Value::Object(diagnostic) = value else { panic!("{value:?}") };
            (marked(&text, &diagnostic), description(&diagnostic))
        })
        .collect()
}

fn marked(text: &str, diagnostic: &JsonMap) -> String {
    let Value::Object(range) = &diagnostic["range"] else { panic!("{diagnostic:?}") };
    let offset = |key: &str| match &range[key] {
        Value::Integer(n) => usize::try_from(*n).unwrap(),
        Value::Number(n) => *n as usize,
        other => panic!("{other:?}"),
    };
    text[offset("pos")..offset("end")].to_string()
}

fn description(diagnostic: &JsonMap) -> String {
    let Value::Object(message) = &diagnostic["message"] else { panic!("{diagnostic:?}") };
    let Value::String(description) = &message["description"] else { panic!("{diagnostic:?}") };
    description.clone()
}
