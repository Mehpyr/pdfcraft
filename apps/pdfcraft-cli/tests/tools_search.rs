//! `pdfcraft-cli tools --query/--category/--name`: what the compact MCP server's `tool_search`
//! answers, without starting a server. Plain `tools` still prints every tool with its schema.

use std::process::{Command, Output};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_pdfcraft-cli");

fn tools(args: &[&str]) -> Output {
    Command::new(BIN).arg("tools").args(args).output().unwrap()
}

/// What a successful `tools` call printed.
fn json(args: &[&str]) -> Value {
    let out = tools(args);
    assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn names(found: &Value) -> Vec<&str> {
    found["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect()
}

#[test]
fn name_returns_one_tool_with_its_input_schema() {
    let all = json(&[]);
    let listed = all.as_array().unwrap().iter().find(|t| t["name"] == "doc_create").unwrap();
    let one = json(&["--name", "doc_create"]);
    assert_eq!(one["tool"]["name"], "doc_create");
    assert_eq!(one["tool"]["category"], "doc");
    assert_eq!(one["tool"]["input_schema"], listed["input_schema"]);
    assert_eq!(one["tool"]["description"], listed["description"]);
}

#[test]
fn category_lists_only_its_tools_one_sentence_each() {
    let all = json(&[]);
    let expected: Vec<&str> = all.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).filter(|n| n.starts_with("page_")).collect();
    assert!(expected.len() > 1, "{expected:?}");
    let page = json(&["--category", "page"]);
    assert_eq!(names(&page), expected);
    assert_eq!(page["count"], expected.len());
    for t in page["tools"].as_array().unwrap() {
        assert_eq!(t["category"], "page", "{t}");
        assert!(t.get("input_schema").is_none(), "{t}");
    }
    // The categories there are, for the next search.
    assert!(page["categories"].as_array().unwrap().iter().any(|c| c == "doc"), "{page}");
}

#[test]
fn query_keeps_the_tools_that_match_every_word() {
    let total = json(&[]).as_array().unwrap().len();
    let rotate = json(&["--query", "ROTATE"]);
    assert!(names(&rotate).contains(&"page_rotate"), "{rotate}");
    assert!(names(&rotate).len() < total);
    let both = json(&["--query", "rotate", "--category", "page"]);
    assert!(names(&both).contains(&"page_rotate") && names(&both).iter().all(|n| n.starts_with("page_")), "{both}");
    assert_eq!(json(&["--query", "rotate no-such-word"])["count"], 0);
}

#[test]
fn an_unknown_tool_or_a_bad_option_fails_with_what_to_do() {
    for (args, expected) in [
        (vec!["--name", "no_such_tool"], "pdfcraft-cli: tools: unknown tool \"no_such_tool\"; find its name with --query WORDS or --category PREFIX"),
        (vec!["--nmae", "doc_create"], "pdfcraft-cli: tools: unknown option --nmae"),
        (vec!["--query"], "pdfcraft-cli: tools: --query needs a value"),
        (vec!["--category", "page", "--name"], "pdfcraft-cli: tools: --name needs a value"),
        (vec!["rotate"], "pdfcraft-cli: tools: unexpected argument \"rotate\"; search with --query WORDS, --category PREFIX or --name TOOL"),
    ] {
        let out = tools(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), expected, "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn without_options_every_tool_is_listed_with_its_schema_as_before() {
    let all = json(&[]);
    let list = all.as_array().unwrap();
    assert!(list.len() > 100, "{} tools", list.len());
    for t in list {
        let mut keys: Vec<&str> = t.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["command", "description", "input_schema", "name", "read_only"], "{t}");
    }
}

/// The CLI prints exactly the text the compact MCP server answers `tool_search` with: one search.
#[cfg(feature = "mcp")]
#[test]
fn the_answer_is_the_mcp_tool_search_answer() {
    use std::io::Write as _;
    use std::process::Stdio;

    let searches: [(&[&str], Value); 3] = [
        (&["--category", "page"], serde_json::json!({ "category": "page" })),
        (&["--name", "doc_create"], serde_json::json!({ "name": "doc_create" })),
        (&["--query", "rotate", "--category", "page"], serde_json::json!({ "query": "rotate", "category": "page" })),
    ];
    let mut input = String::from("{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{}}\n");
    for (id, (_, arguments)) in searches.iter().enumerate() {
        let call = serde_json::json!({ "jsonrpc": "2.0", "id": id + 1, "method": "tools/call", "params": { "name": "tool_search", "arguments": arguments } });
        input.push_str(&format!("{call}\n"));
    }
    let mut child = Command::new(BIN).args(["mcp", "--compact"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), searches.len());
    for ((flags, _), reply) in searches.iter().zip(&replies) {
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let mcp = reply["result"]["content"][0]["text"].as_str().unwrap();
        let cli = tools(flags);
        assert!(cli.status.success(), "{flags:?}: {}", String::from_utf8_lossy(&cli.stderr));
        assert_eq!(String::from_utf8(cli.stdout).unwrap(), format!("{mcp}\n"), "{flags:?}");
    }
}
