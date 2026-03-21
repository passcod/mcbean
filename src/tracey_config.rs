/// Parse a tracey config.styx file and return the config.
pub fn parse(input: &str) -> Result<tracey_config::Config, String> {
    facet_styx::from_str(input).map_err(|e| format!("failed to parse tracey config: {e}"))
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
        let config = parse(SAMPLE_CONFIG).expect("should parse");
        assert_eq!(config.specs.len(), 1);
        assert_eq!(config.specs[0].name, "editor");
        assert_eq!(config.specs[0].include, vec!["docs/spec/editor.md"]);
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
        let config = parse(input).expect("should parse");
        assert_eq!(config.specs.len(), 2);
        assert_eq!(config.specs[0].name, "first");
        assert_eq!(config.specs[0].include, vec!["a.md", "b.md"]);
        assert_eq!(config.specs[1].name, "second");
        assert_eq!(config.specs[1].include, vec!["c/d.md"]);
    }

    #[test]
    fn parse_spec_without_include() {
        let input = r#"specs (
    {
        name orphan
    }
)"#;
        let config = parse(input).expect("should parse");
        assert_eq!(config.specs.len(), 1);
        assert_eq!(config.specs[0].name, "orphan");
        assert!(config.specs[0].include.is_empty());
    }
}
