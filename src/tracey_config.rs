use winnow::combinator::{alt, delimited, preceded, repeat};
use winnow::prelude::*;
use winnow::token::take_while;

/// A spec definition extracted from a tracey config.
#[derive(Debug, Clone)]
pub struct TraceySpecDef {
    pub name: String,
    pub include: Vec<String>,
}

/// Parsed representation of styx values.
#[derive(Debug, Clone)]
enum Value {
    Atom(String),
    List(Vec<Value>),
    Block(Vec<(String, Value)>),
}

/// Parse a tracey config.styx file and extract spec definitions.
pub fn parse_tracey_config(input: &str) -> Result<Vec<TraceySpecDef>, String> {
    let entries = document
        .parse(input)
        .map_err(|e| format!("failed to parse tracey config: {e}"))?;

    let mut specs = Vec::new();
    for (key, value) in &entries {
        if key == "specs" {
            if let Value::List(items) = value {
                for item in items {
                    if let Value::Block(block_entries) = item {
                        if let Some(spec) = extract_spec(block_entries) {
                            specs.push(spec);
                        }
                    }
                }
            }
        }
    }
    Ok(specs)
}

fn extract_spec(entries: &[(String, Value)]) -> Option<TraceySpecDef> {
    let name = entries.iter().find_map(|(k, v)| {
        if k == "name" {
            if let Value::Atom(s) = v {
                Some(s.clone())
            } else {
                None
            }
        } else {
            None
        }
    })?;

    let include = entries
        .iter()
        .find_map(|(k, v)| {
            if k == "include" {
                if let Value::List(items) = v {
                    Some(
                        items
                            .iter()
                            .filter_map(|i| {
                                if let Value::Atom(s) = i {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap_or_default();

    Some(TraceySpecDef { name, include })
}

// ---------------------------------------------------------------------------
// Winnow parsers
// ---------------------------------------------------------------------------

fn ws<'i>(input: &mut &'i str) -> ModalResult<()> {
    take_while(0.., |c: char| c.is_ascii_whitespace())
        .void()
        .parse_next(input)
}

/// An atom is a contiguous run of non-delimiter, non-whitespace characters.
/// This handles bare strings like `editor`, URLs like `https://github.com/x/y`,
/// and paths like `src/**/*.rs`.
fn atom<'i>(input: &mut &'i str) -> ModalResult<Value> {
    take_while(1.., |c: char| {
        !c.is_ascii_whitespace() && !matches!(c, '(' | ')' | '{' | '}')
    })
    .map(|s: &str| Value::Atom(s.to_string()))
    .parse_next(input)
}

fn list<'i>(input: &mut &'i str) -> ModalResult<Value> {
    delimited('(', list_items, (ws, ')'))
        .map(Value::List)
        .parse_next(input)
}

fn list_items<'i>(input: &mut &'i str) -> ModalResult<Vec<Value>> {
    let mut items = Vec::new();
    loop {
        ws.parse_next(input)?;
        if input.is_empty() || input.starts_with(')') {
            break;
        }
        items.push(value.parse_next(input)?);
    }
    Ok(items)
}

fn block<'i>(input: &mut &'i str) -> ModalResult<Value> {
    delimited('{', block_entries, (ws, '}'))
        .map(Value::Block)
        .parse_next(input)
}

fn block_entries<'i>(input: &mut &'i str) -> ModalResult<Vec<(String, Value)>> {
    let mut entries = Vec::new();
    loop {
        ws.parse_next(input)?;
        if input.is_empty() || input.starts_with('}') {
            break;
        }
        let k = key.parse_next(input)?;
        ws.parse_next(input)?;
        let v = value.parse_next(input)?;
        entries.push((k, v));
    }
    Ok(entries)
}

fn key<'i>(input: &mut &'i str) -> ModalResult<String> {
    take_while(1.., |c: char| c.is_alphanumeric() || matches!(c, '_' | '-'))
        .map(|s: &str| s.to_string())
        .parse_next(input)
}

fn value<'i>(input: &mut &'i str) -> ModalResult<Value> {
    alt((list, block, atom)).parse_next(input)
}

/// Skip `@schema { ... }` declarations at the top of the file.
fn skip_schema<'i>(input: &mut &'i str) -> ModalResult<()> {
    '@'.parse_next(input)?;
    take_while(0.., |c: char| c != '{')
        .void()
        .parse_next(input)?;
    // Skip the brace-delimited block (handles nesting via the block parser)
    block.void().parse_next(input)?;
    Ok(())
}

fn document<'i>(input: &mut &'i str) -> ModalResult<Vec<(String, Value)>> {
    let mut entries = Vec::new();
    loop {
        ws.parse_next(input)?;
        if input.is_empty() {
            break;
        }
        // Try skipping @schema or other @ directives
        if input.starts_with('@') {
            skip_schema.parse_next(input)?;
            continue;
        }
        let k = key.parse_next(input)?;
        ws.parse_next(input)?;
        let v = value.parse_next(input)?;
        entries.push((k, v));
    }
    Ok(entries)
}

/// Skip `@schema { ... }` and similar directives that may appear before the
/// `specs` block.  This is kept public so callers can strip them if they need
/// to pre-process the input.
fn _skip_at_directives<'i>(input: &mut &'i str) -> ModalResult<()> {
    repeat(0.., preceded(ws, skip_schema))
        .map(|(): ()| ())
        .parse_next(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CONFIG: &str = r#"@schema {id crate:tracey-config@1, cli tracey}

specs (
    {
        name editor
        source_url https://github.com/passcod/mcbean
        include (docs/spec/editor.md)
        impls (
            {
                name main
                include (
                    src/*.rs
                    src/**/*.rs
                )
            }
        )
    }
)"#;

    #[test]
    fn parse_sample_config() {
        let specs = parse_tracey_config(SAMPLE_CONFIG).expect("should parse");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "editor");
        assert_eq!(specs[0].include, vec!["docs/spec/editor.md"]);
    }

    #[test]
    fn parse_multiple_specs() {
        let input = r#"@schema {id crate:tracey-config@1, cli tracey}

specs (
    {
        name first
        include (a.md b.md)
    }
    {
        name second
        include (c/d.md)
    }
)"#;
        let specs = parse_tracey_config(input).expect("should parse");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "first");
        assert_eq!(specs[0].include, vec!["a.md", "b.md"]);
        assert_eq!(specs[1].name, "second");
        assert_eq!(specs[1].include, vec!["c/d.md"]);
    }

    #[test]
    fn parse_spec_without_include() {
        let input = r#"specs (
    {
        name orphan
    }
)"#;
        let specs = parse_tracey_config(input).expect("should parse");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "orphan");
        assert!(specs[0].include.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        let result = parse_tracey_config("{{{{");
        assert!(result.is_err());
    }
}
