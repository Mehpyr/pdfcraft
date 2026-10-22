//! `tool_search`: find a tool without reading the whole tool table.
//!
//! The table with every input schema is large (MCP `tools/list`, `pdfcraft-cli tools`). A search
//! answers with one entry per tool instead, its description cut to the first sentence, filtered by
//! words and by category (the name up to the first underscore); `name` answers with one tool in
//! full, its input schema included. The compact MCP server offers it as the `tool_search` meta
//! tool and `pdfcraft-cli tools` as `--query`, `--category` and `--name`, so both give the same
//! answer.

use serde_json::{Value, json};

use crate::ToolError;

/// The name of the meta tool in compact MCP mode, which its messages use too.
pub(crate) const TOOL_SEARCH: &str = "tool_search";

/// The category of a tool: its name up to the first underscore (`page_rotate` → `page`).
fn category_of(name: &str) -> &str {
    name.split('_').next().unwrap_or(name)
}

/// A description up to and including its first sentence.
fn first_sentence(text: &str) -> &str {
    // ASCII lowercasing keeps byte offsets, so positions found in `lower` are valid in `text`.
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|rest| rest.find(". ")).map(|i| i + from) {
        // "e.g. " and "i.e. " are abbreviations, not the end of the sentence.
        if !lower.get(..i).is_some_and(|before| before.ends_with("e.g") || before.ends_with("i.e")) {
            return text.get(..=i).unwrap_or(text);
        }
        from = i + 2;
    }
    text
}

/// Reject keys other than `allowed`, as the tool table does for a tool's own arguments.
pub(crate) fn check_keys(tool: &str, args: &Value, allowed: &[&str]) -> Result<(), String> {
    match args.as_object().and_then(|o| o.keys().find(|k| !allowed.contains(&k.as_str()))) {
        Some(k) => Err(format!("{tool}: unknown argument {k:?} (expected: {})", allowed.join(", "))),
        None => Ok(()),
    }
}

/// Filter the tool table by query and category, or describe one tool in full.
///
/// `args` is an object (or `null`) with the optional strings `query` (words that must all appear
/// in a tool's name, title or description, in any case), `category` (the name prefix: `doc`,
/// `page`, `text`, …) and `name`. Without `name` the answer is `{count, tools, categories}`, one
/// entry per matching tool with its description's first sentence; with it, `{tool}`, that tool in
/// full. A `name` that no tool has is [`ToolError::UnknownTool`], so each caller can say how to
/// list the names; other bad arguments are [`ToolError::InvalidArgs`].
pub fn search_tools(args: &Value) -> Result<Value, ToolError> {
    if !(args.is_object() || args.is_null()) {
        return Err(ToolError::InvalidArgs(format!("{TOOL_SEARCH} takes an object with optional query, category and name")));
    }
    check_keys(TOOL_SEARCH, args, &["query", "category", "name"]).map_err(ToolError::InvalidArgs)?;
    let text = |key: &str| match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim())),
        Some(_) => Err(ToolError::InvalidArgs(format!("{TOOL_SEARCH}: {key} must be a string"))),
    };
    let (query, category, name) = (text("query")?, text("category")?, text("name")?);
    let all = crate::tools();
    if let Some(name) = name {
        let t = all.iter().find(|t| t.name == name).ok_or_else(|| ToolError::UnknownTool(name.to_owned()))?;
        return Ok(json!({ "tool": {
            "name": t.name, "title": t.title, "category": category_of(t.name), "description": t.description,
            "read_only": t.read_only, "destructive": t.destructive, "input_schema": t.input_schema,
        } }));
    }
    let category = category.map(|c| c.trim_end_matches('_').to_lowercase()).filter(|c| !c.is_empty());
    let words: Vec<String> = query.map(|q| q.to_lowercase().split_whitespace().map(str::to_owned).collect()).unwrap_or_default();
    let mut categories: Vec<&str> = all.iter().map(|t| category_of(t.name)).collect();
    categories.sort_unstable();
    categories.dedup();
    let found: Vec<Value> = all
        .iter()
        .filter(|t| category.as_deref().is_none_or(|c| category_of(t.name) == c))
        .filter(|t| {
            let haystack = format!("{} {} {}", t.name, t.title, t.description).to_lowercase();
            words.iter().all(|w| haystack.contains(w.as_str()))
        })
        .map(|t| json!({ "name": t.name, "category": category_of(t.name), "description": first_sentence(t.description), "read_only": t.read_only }))
        .collect();
    Ok(json!({ "count": found.len(), "tools": found, "categories": categories }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{first_sentence, search_tools};
    use crate::ToolError;

    #[test]
    fn first_sentence_skips_abbreviations() {
        assert_eq!(first_sentence("Rotate pages. Undoable."), "Rotate pages.");
        assert_eq!(first_sentence("Label pages (i.e. roman numerals). Undoable."), "Label pages (i.e. roman numerals).");
        assert_eq!(first_sentence("Label pages (I.E. roman numerals, E.G. i, ii). Undoable."), "Label pages (I.E. roman numerals, E.G. i, ii).");
        assert_eq!(first_sentence("Use a value, e.g. en-US, or none. Then more."), "Use a value, e.g. en-US, or none.");
        assert_eq!(first_sentence("No full stop here"), "No full stop here");
        assert_eq!(first_sentence("Only an abbreviation, e.g. this"), "Only an abbreviation, e.g. this");
        assert_eq!(first_sentence(""), "");
        assert_eq!(first_sentence("Zażółć gęślą, i.e. jaźń. Dalej."), "Zażółć gęślą, i.e. jaźń.");
    }

    /// Built without the `mcp` feature too: the CLI's `tools --query` uses it there.
    #[test]
    fn search_tools_answers_without_the_mcp_server() {
        let page = search_tools(&json!({ "category": "page" })).unwrap();
        let names: Vec<&str> = page["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"page_rotate") && names.iter().all(|n| n.starts_with("page_")), "{names:?}");
        assert_eq!(page["count"], names.len());

        let one = search_tools(&json!({ "name": " doc_create " })).unwrap();
        assert_eq!(one["tool"]["name"], "doc_create");
        assert_eq!(one["tool"]["input_schema"]["type"], "object");

        assert_eq!(search_tools(&json!({ "name": "no_such_tool" })), Err(ToolError::UnknownTool("no_such_tool".into())));
        assert!(matches!(search_tools(&json!({ "query": 3 })), Err(ToolError::InvalidArgs(m)) if m == "tool_search: query must be a string"));
        assert!(matches!(search_tools(&json!({ "qurey": "x" })), Err(ToolError::InvalidArgs(m)) if m.contains("\"qurey\"")));
        assert!(matches!(search_tools(&json!("page")), Err(ToolError::InvalidArgs(_))));
        assert_eq!(search_tools(&serde_json::Value::Null).unwrap()["count"], crate::tools().len());
    }
}
