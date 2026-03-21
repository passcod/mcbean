use leptos::prelude::*;
use serde::{Deserialize, Serialize};

const TOP_DROP_KEY: &str = "^^top";

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecBlockKind {
    Heading {
        level: u8,
        text: String,
        anchor: String,
    },
    Rule {
        id: String,
        text: String,
    },
    Paragraph {
        text: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpecBlock {
    pub key: String,
    pub kind: SpecBlockKind,
    pub html: String,
}

impl SpecBlock {
    pub fn edit_text(&self) -> &str {
        match &self.kind {
            SpecBlockKind::Heading { text, .. } => text,
            SpecBlockKind::Rule { text, .. } => text,
            SpecBlockKind::Paragraph { text } => text,
        }
    }
}

// ── Sidebar data ──────────────────────────────────────────────────────────────

pub fn blocks_to_sidebar_data(
    blocks: &[SpecBlock],
    spec_name: &str,
) -> (
    Vec<crate::components::sidebar::SpecOutline>,
    Vec<crate::components::sidebar::SearchEntry>,
) {
    use crate::components::sidebar::{HeadingEntry, SearchEntry, SpecOutline};

    let mut headings: Vec<HeadingEntry> = Vec::new();
    let mut search_entries: Vec<SearchEntry> = Vec::new();
    let mut current_anchor = String::new();

    for block in blocks {
        match &block.kind {
            SpecBlockKind::Heading {
                level,
                text,
                anchor,
            } => {
                current_anchor = anchor.clone();
                headings.push(HeadingEntry {
                    level: *level,
                    text: text.clone(),
                    anchor: anchor.clone(),
                });
                search_entries.push(SearchEntry {
                    spec_name: spec_name.to_string(),
                    text: text.clone(),
                    anchor: anchor.clone(),
                });
            }
            SpecBlockKind::Rule { text, .. } | SpecBlockKind::Paragraph { text } => {
                if !text.is_empty() {
                    search_entries.push(SearchEntry {
                        spec_name: spec_name.to_string(),
                        text: text.clone(),
                        anchor: current_anchor.clone(),
                    });
                }
            }
        }
    }

    (
        vec![SpecOutline {
            name: spec_name.to_string(),
            headings,
        }],
        search_entries,
    )
}

// ── Server functions ──────────────────────────────────────────────────────────

/// Ensure `proposals.base_snapshot_id` is set for `proposal_id`, lazily
/// linking it to the latest `spec_snapshots` row for the repository if it
/// is still NULL.  Returns the resolved snapshot id, or
/// `diesel::result::Error::NotFound` if no snapshot exists yet.
#[cfg(feature = "ssr")]
pub fn resolve_base_snapshot_id(
    proposal_id: i32,
    conn: &mut diesel::PgConnection,
) -> Result<i32, diesel::result::Error> {
    use diesel::prelude::*;

    use crate::db::schema::{proposals, spec_snapshots};

    let (repo_id, mut base_snapshot_id): (i32, Option<i32>) = proposals::table
        .find(proposal_id)
        .select((proposals::repository_id, proposals::base_snapshot_id))
        .first(conn)?;

    if base_snapshot_id.is_none() {
        let latest: Option<i32> = spec_snapshots::table
            .filter(spec_snapshots::repository_id.eq(repo_id))
            .order(spec_snapshots::id.desc())
            .select(spec_snapshots::id)
            .first(conn)
            .optional()?;

        if let Some(sid) = latest {
            diesel::update(proposals::table.find(proposal_id))
                .set(proposals::base_snapshot_id.eq(sid))
                .execute(conn)?;
            base_snapshot_id = Some(sid);
        }
    }

    base_snapshot_id.ok_or(diesel::result::Error::NotFound)
}

/// Return a full Loro snapshot for the proposal, lazily linking it to the
/// latest spec snapshot for the repository if not yet set.
#[server]
pub async fn get_proposal_doc(proposal_id: i32) -> Result<Vec<u8>, ServerFnError> {
    use diesel::prelude::*;
    use loro::ExportMode;

    use crate::components::loro_doc::reconstruct_doc;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let (base_bytes, update_rows) = conn
        .interact(move |conn| {
            use crate::db::schema::{proposal_loro_updates, spec_snapshots};

            let sid = resolve_base_snapshot_id(proposal_id, conn)?;

            let base_bytes: Vec<u8> = spec_snapshots::table
                .find(sid)
                .select(spec_snapshots::loro_bytes)
                .first(conn)?;

            let update_rows: Vec<Vec<u8>> = proposal_loro_updates::table
                .filter(proposal_loro_updates::proposal_id.eq(proposal_id))
                .order(proposal_loro_updates::id.asc())
                .select(proposal_loro_updates::update_bytes)
                .load(conn)?;

            Ok::<_, diesel::result::Error>((base_bytes, update_rows))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    let doc = reconstruct_doc(&base_bytes, &update_rows)
        .map_err(|e| ServerFnError::new(format!("reconstruct: {e}")))?;

    doc.export(ExportMode::Snapshot)
        .map_err(|e| ServerFnError::new(format!("export: {e}")))
}

/// Delta-sync: receive a client update, store it, return what the client is
/// missing.  The missing bytes are computed from the server state BEFORE
/// importing the client's update so the client never receives its own ops back.
// r[impl proposal.diff.semantic]
#[server]
pub async fn sync_proposal(
    proposal_id: i32,
    peer_id: String,
    client_vv: Vec<u8>,
    update: Vec<u8>,
) -> Result<Vec<u8>, ServerFnError> {
    use diesel::prelude::*;
    use loro::ExportMode;

    use crate::components::loro_doc::{decode_vv_or_empty, reconstruct_doc};

    let user_id = crate::auth::get_or_create_user_id().await?;
    let peer_id_u64: u64 = peer_id
        .parse()
        .map_err(|_| ServerFnError::new("invalid peer_id"))?;
    let peer_id_i64 = peer_id_u64 as i64;

    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let (base_bytes, update_rows) = conn
        .interact(move |conn| {
            use crate::db::schema::{proposal_loro_updates, spec_snapshots};

            let sid = resolve_base_snapshot_id(proposal_id, conn)?;

            let base_bytes: Vec<u8> = spec_snapshots::table
                .find(sid)
                .select(spec_snapshots::loro_bytes)
                .first(conn)?;

            let update_rows: Vec<Vec<u8>> = proposal_loro_updates::table
                .filter(proposal_loro_updates::proposal_id.eq(proposal_id))
                .order(proposal_loro_updates::id.asc())
                .select(proposal_loro_updates::update_bytes)
                .load(conn)?;

            Ok::<_, diesel::result::Error>((base_bytes, update_rows))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    let doc = reconstruct_doc(&base_bytes, &update_rows)
        .map_err(|e| ServerFnError::new(format!("reconstruct: {e}")))?;

    // Compute what the client is missing BEFORE importing its new update.
    let from_vv = decode_vv_or_empty(&client_vv);
    let missing = doc
        .export(ExportMode::updates(&from_vv))
        .map_err(|e| ServerFnError::new(format!("export missing: {e}")))?;

    if !update.is_empty() {
        doc.import(&update)
            .map_err(|e| ServerFnError::new(format!("import: {e}")))?;

        // r[impl edit.history]
        conn.interact(move |conn| {
            use crate::db::schema::proposal_loro_updates;
            diesel::insert_into(proposal_loro_updates::table)
                .values((
                    proposal_loro_updates::proposal_id.eq(proposal_id),
                    proposal_loro_updates::user_id.eq(user_id),
                    proposal_loro_updates::peer_id.eq(peer_id_i64),
                    proposal_loro_updates::update_bytes.eq(&update),
                ))
                .execute(conn)
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("insert: {e}")))?;
    }

    Ok(missing)
}

// ── Helpers available on both SSR and WASM ────────────────────────────────────

pub async fn parse_blocks_from_content(content: &str) -> Vec<SpecBlock> {
    use marq::{DocElement, RenderOptions, render};

    #[cfg(feature = "ssr")]
    tracing::info!(
        content_bytes = content.len(),
        content_lines = content.lines().count(),
        content_preview = %crate::components::loro_doc::preview(content, 200),
        "parse_blocks_from_content: entry"
    );

    #[cfg(feature = "ssr")]
    tracing::info!("parse_blocks_from_content: calling marq::render");
    #[cfg(feature = "ssr")]
    let t0 = std::time::Instant::now();

    let doc = match render(content, &RenderOptions::new()).await {
        Ok(d) => {
            #[cfg(feature = "ssr")]
            tracing::info!(
                elapsed_ms = t0.elapsed().as_millis(),
                element_count = d.elements.len(),
                req_count = d.reqs.len(),
                heading_count = d.headings.len(),
                "parse_blocks_from_content: marq::render returned"
            );
            d
        }
        Err(_e) => {
            #[cfg(feature = "ssr")]
            tracing::warn!(error = %_e, elapsed_ms = t0.elapsed().as_millis(), "marq render failed in parse_blocks_from_content");
            return Vec::new();
        }
    };

    let mut blocks = Vec::new();
    let mut seq: u64 = 0;

    for element in &doc.elements {
        seq += 1;
        let key = format!("s{seq}");

        match element {
            DocElement::Heading(h) => {
                #[cfg(feature = "ssr")]
                tracing::debug!(
                    seq,
                    level = h.level,
                    title = %h.title,
                    "parse_blocks_from_content: heading element"
                );
                blocks.push(SpecBlock {
                    key,
                    kind: SpecBlockKind::Heading {
                        level: h.level,
                        text: h.title.clone(),
                        anchor: h.id.clone(),
                    },
                    html: format!(
                        "<h{level} id=\"{id}\">{text}</h{level}>",
                        level = h.level,
                        id = h.id,
                        text = html_escape(&h.title),
                    ),
                });
            }
            DocElement::Req(r) => {
                #[cfg(feature = "ssr")]
                tracing::info!(
                    seq,
                    rule_id = %r.id,
                    raw_bytes = r.raw.len(),
                    raw_lines = r.raw.lines().count(),
                    raw_preview = %crate::components::loro_doc::preview(&r.raw, 300),
                    "parse_blocks_from_content: req element — about to strip_blockquote_prefixes"
                );

                // Strip the `> ` blockquote prefixes marq adds for blockquote-style
                // rules, then trim.  We use the stripped source directly rather than
                // round-tripping through parse_ast/render_to_markdown: the marq ast.rs
                // parse_blocks_until_end loop hangs on tight lists whose items start
                // with inline markup (e.g. `**bold:**`) because parse_blocks breaks on
                // End(Strong) without consuming it, causing an infinite retry loop.
                // The stripped text is already well-formed Markdown — no reformatting needed.
                let prose = strip_blockquote_prefixes(&r.raw).trim().to_string();

                #[cfg(feature = "ssr")]
                tracing::info!(
                    seq,
                    rule_id = %r.id,
                    prose_bytes = prose.len(),
                    prose_lines = prose.lines().count(),
                    prose_preview = %crate::components::loro_doc::preview(&prose, 300),
                    "parse_blocks_from_content: stripped prose"
                );

                blocks.push(SpecBlock {
                    key,
                    kind: SpecBlockKind::Rule {
                        id: r.id.to_string(),
                        text: prose,
                    },
                    html: r.html.clone(),
                });

                #[cfg(feature = "ssr")]
                tracing::info!(seq, rule_id = %r.id, "parse_blocks_from_content: req element done");
            }
            DocElement::Paragraph(p) => {
                #[cfg(feature = "ssr")]
                tracing::debug!(
                    seq,
                    offset = p.offset,
                    "parse_blocks_from_content: paragraph element"
                );
                let start = p.offset.min(content.len());
                let rest = &content[start..];
                let end = rest.find("\n\n").unwrap_or(rest.len());
                let text = join_hard_wraps(rest[..end].trim());
                if text.is_empty() || text.starts_with("r[") {
                    #[cfg(feature = "ssr")]
                    tracing::debug!(
                        seq,
                        "parse_blocks_from_content: paragraph skipped (empty or r[)"
                    );
                    continue;
                }
                blocks.push(SpecBlock {
                    key,
                    kind: SpecBlockKind::Paragraph { text: text.clone() },
                    html: format!("<p>{}</p>", html_escape(&text)),
                });
            }
        }
    }

    #[cfg(feature = "ssr")]
    tracing::info!(
        total_blocks = blocks.len(),
        total_elapsed_ms = t0.elapsed().as_millis(),
        "parse_blocks_from_content: complete"
    );

    blocks
}

pub fn slugify(text: &str) -> String {
    let raw: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    raw.split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Join soft-wrapped prose lines into a single logical line.
///
/// Used only for `Paragraph` blocks extracted by byte-offset from the raw
/// source.  Rule bodies are handled via the marq AST round-trip instead, so
/// this function never sees fenced code blocks or other block-level constructs.
fn join_hard_wraps(text: &str) -> String {
    text.lines().collect::<Vec<_>>().join(" ")
}

fn strip_blockquote_prefixes(raw: &str) -> String {
    raw.lines()
        .map(|line| {
            line.strip_prefix("> ")
                .or_else(|| line.strip_prefix('>'))
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: run the async function in a single-threaded tokio runtime.
    fn run(f: impl std::future::Future<Output = Vec<SpecBlock>>) -> Vec<SpecBlock> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    // ── strip_blockquote_prefixes ─────────────────────────────────────────────

    #[test]
    fn test_strip_blockquote_prefixes_simple() {
        let input = "> hello\n> world";
        assert_eq!(strip_blockquote_prefixes(input), "hello\nworld");
    }

    #[test]
    fn test_strip_blockquote_prefixes_empty_marker_line() {
        // A `>` with no trailing space (blank continuation line in a blockquote)
        let input = "> para one\n>\n> para two";
        assert_eq!(strip_blockquote_prefixes(input), "para one\n\npara two");
    }

    #[test]
    fn test_strip_blockquote_prefixes_no_prefix() {
        // Content without blockquote markers is returned unchanged.
        let input = "plain text\nno markers";
        assert_eq!(strip_blockquote_prefixes(input), "plain text\nno markers");
    }

    // ── parse_blocks_from_content: basic cases ────────────────────────────────

    #[test]
    fn test_parse_single_rule() {
        let md = "r[foo.bar]\nThis is the rule text.\n";
        let blocks = run(parse_blocks_from_content(md));
        assert_eq!(
            blocks.len(),
            1,
            "expected exactly one block, got: {blocks:#?}"
        );
        match &blocks[0].kind {
            SpecBlockKind::Rule { id, text } => {
                assert_eq!(id, "foo.bar");
                assert_eq!(text, "This is the rule text.");
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_heading_and_rule() {
        let md = "# My Section\n\nr[sec.rule]\nThe rule.\n";
        let blocks = run(parse_blocks_from_content(md));
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert!(matches!(
            &blocks[0].kind,
            SpecBlockKind::Heading { level: 1, .. }
        ));
        assert!(matches!(&blocks[1].kind, SpecBlockKind::Rule { .. }));
    }

    // ── multi-paragraph blockquote rule ──────────────────────────────────────
    //
    // This is the core regression: a rule whose body spans multiple blockquote
    // paragraphs (separated by `>` blank lines) must produce exactly ONE Rule
    // block containing all paragraphs, not one Rule + loose Paragraph blocks.

    const MULTI_PARA_RULE: &str = "\
> r[iso.cdrom-partscan+4]
> First paragraph of the rule, which is long enough
> to be hard-wrapped across several lines.
>
> Second paragraph of the same rule.
>
> Third paragraph, still part of the same rule.
";

    #[test]
    fn test_multi_para_rule_produces_one_block() {
        let blocks = run(parse_blocks_from_content(MULTI_PARA_RULE));
        assert_eq!(
            blocks.len(),
            1,
            "expected 1 Rule block but got {}:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_multi_para_rule_kind_is_rule() {
        let blocks = run(parse_blocks_from_content(MULTI_PARA_RULE));
        assert!(
            matches!(&blocks[0].kind, SpecBlockKind::Rule { .. }),
            "block 0 should be a Rule, got {:?}",
            blocks[0].kind
        );
    }

    #[test]
    fn test_multi_para_rule_id() {
        let blocks = run(parse_blocks_from_content(MULTI_PARA_RULE));
        match &blocks[0].kind {
            SpecBlockKind::Rule { id, .. } => assert_eq!(id, "iso.cdrom-partscan+4"),
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    #[test]
    fn test_multi_para_rule_text_contains_all_paragraphs() {
        let blocks = run(parse_blocks_from_content(MULTI_PARA_RULE));
        match &blocks[0].kind {
            SpecBlockKind::Rule { text, .. } => {
                assert!(
                    text.contains("First paragraph"),
                    "text missing first para: {text:?}"
                );
                assert!(
                    text.contains("Second paragraph"),
                    "text missing second para: {text:?}"
                );
                assert!(
                    text.contains("Third paragraph"),
                    "text missing third para: {text:?}"
                );
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // ── rule with a code block ────────────────────────────────────────────────

    const RULE_WITH_CODE: &str = "\
> r[test.code]
> Some prose before the code.
>
> ```rust
> let foo = 123;
> let bar = 456;
> ```
";

    #[test]
    fn test_rule_with_code_block_is_one_block() {
        let blocks = run(parse_blocks_from_content(RULE_WITH_CODE));
        assert_eq!(
            blocks.len(),
            1,
            "expected 1 block, got {}:\n{blocks:#?}",
            blocks.len()
        );
        assert!(matches!(&blocks[0].kind, SpecBlockKind::Rule { .. }));
    }

    #[test]
    fn test_rule_with_code_block_text_has_fence() {
        let blocks = run(parse_blocks_from_content(RULE_WITH_CODE));
        match &blocks[0].kind {
            SpecBlockKind::Rule { text, .. } => {
                assert!(
                    text.contains("```"),
                    "expected fenced code block in text, got: {text:?}"
                );
                assert!(
                    text.contains("let foo = 123"),
                    "expected code content in text, got: {text:?}"
                );
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // ── rule with a list ──────────────────────────────────────────────────────

    const RULE_WITH_LIST: &str = "\
> r[test.list]
> The following items must be present:
>
> - **NVMe:** `nvme`, `nvme_core`
> - **SATA/AHCI:** `ahci`
> - **RAID controllers:** `megaraid_sas`, `mpt3sas`
";

    #[test]
    fn test_rule_with_list_is_one_block() {
        let blocks = run(parse_blocks_from_content(RULE_WITH_LIST));
        assert_eq!(
            blocks.len(),
            1,
            "expected 1 block, got {}:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_rule_with_list_text_contains_items() {
        let blocks = run(parse_blocks_from_content(RULE_WITH_LIST));
        match &blocks[0].kind {
            SpecBlockKind::Rule { text, .. } => {
                assert!(text.contains("NVMe"), "missing NVMe item: {text:?}");
                assert!(text.contains("ahci"), "missing ahci item: {text:?}");
                assert!(
                    text.contains("megaraid_sas"),
                    "missing megaraid_sas item: {text:?}"
                );
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // ── realistic in-context document ────────────────────────────────────────
    //
    // A multi-paragraph rule surrounded by normal single-paragraph rules and a
    // heading — the structure that actually appears in production spec files.

    const IN_CONTEXT_DOC: &str = "\
# Live ISO

r[iso.simple-rule]
A simple single-paragraph rule before the multi-para one.

> r[iso.cdrom-partscan+4]
> When the ISO is booted as optical media (e.g. `/dev/sr0` in a VM), the
> Linux kernel does not parse the GPT appended partitions because the CD-ROM
> block device driver exposes the device as a single block device with an
> ISO 9660 filesystem. As a result, partition device nodes are never created
> and `/dev/disk/by-partuuid/` symlinks for the appended BESIMAGES and
> BESCONF partitions do not appear.
>
> The installer must handle this transparently. When the well-known
> PARTUUIDs are not present in `/dev/disk/by-partuuid/`, the installer
> must identify the boot device, create a loop device with partition
> scanning enabled.
>
> This must happen early in the installer's startup.

r[iso.after-rule]
A simple single-paragraph rule after the multi-para one.
";

    // Inspect raw marq elements — always passes, read with --nocapture.
    #[test]
    fn test_inspect_marq_elements_in_context() {
        use marq::{DocElement, RenderOptions, render};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let doc = rt
            .block_on(render(IN_CONTEXT_DOC, &RenderOptions::new()))
            .expect("marq render failed");

        println!(
            "\n=== marq elements for IN_CONTEXT_DOC ({} total) ===",
            doc.elements.len()
        );
        for (i, el) in doc.elements.iter().enumerate() {
            match el {
                DocElement::Heading(h) => {
                    println!("  [{i}] Heading(level={}, title={:?})", h.level, h.title);
                }
                DocElement::Req(r) => {
                    println!(
                        "  [{i}] Req(id={}, raw_lines={}, raw_bytes={})",
                        r.id,
                        r.raw.lines().count(),
                        r.raw.len(),
                    );
                    println!("       raw = {:?}", r.raw);
                }
                DocElement::Paragraph(p) => {
                    println!("  [{i}] Paragraph(offset={})", p.offset);
                    let start = p.offset.min(IN_CONTEXT_DOC.len());
                    let rest = &IN_CONTEXT_DOC[start..];
                    let end = rest.find("\n\n").unwrap_or(rest.len());
                    println!("       source_slice = {:?}", &rest[..end.min(120)]);
                }
            }
        }
        println!("=== end ===\n");
    }

    // The parsed blocks from the in-context document should be exactly:
    //   Heading, Rule(simple), Rule(cdrom-partscan+4), Rule(after)
    // — no stray Paragraph blocks from the inner paragraphs of the multi-para rule.

    #[test]
    fn test_in_context_block_count() {
        let blocks = run(parse_blocks_from_content(IN_CONTEXT_DOC));
        println!(
            "\n=== parse_blocks_from_content result ({} blocks) ===",
            blocks.len()
        );
        for (i, b) in blocks.iter().enumerate() {
            match &b.kind {
                SpecBlockKind::Heading { level, text, .. } => {
                    println!("  [{i}] Heading(level={level}, text={text:?})")
                }
                SpecBlockKind::Rule { id, text } => println!(
                    "  [{i}] Rule(id={id:?}, text_lines={})",
                    text.lines().count()
                ),
                SpecBlockKind::Paragraph { text } => {
                    println!("  [{i}] Paragraph(text={:?})", &text[..text.len().min(60)])
                }
            }
        }
        println!("=== end ===\n");
        assert_eq!(
            blocks.len(),
            4,
            "expected Heading + 3 Rules, got {} blocks:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_in_context_cdrom_rule_has_all_paragraphs() {
        let blocks = run(parse_blocks_from_content(IN_CONTEXT_DOC));
        let cdrom = blocks.iter().find(
            |b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "iso.cdrom-partscan+4"),
        );
        let cdrom = cdrom.expect("iso.cdrom-partscan+4 rule not found in blocks");
        match &cdrom.kind {
            SpecBlockKind::Rule { text, .. } => {
                assert!(
                    text.contains("optical media"),
                    "missing first para: {text:?}"
                );
                assert!(
                    text.contains("handle this transparently"),
                    "missing second para: {text:?}"
                );
                assert!(
                    text.contains("early in the installer"),
                    "missing third para: {text:?}"
                );
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // ── inspect raw marq elements for standalone multi-para rule ─────────────
    //
    // Always passes — read the output with `cargo test -- --nocapture`.

    // ── exact real-file content regression test ───────────────────────────────
    //
    // This is the verbatim section from beyondessential/linux-images
    // docs/spec/live-iso.md that the user reported as broken.  The blank
    // continuation lines inside the blockquote use bare `>` (no trailing space),
    // exactly as they appear in the source file.

    const REAL_CDROM_SECTION: &str = "\
## CD-ROM Partition Scanning

> r[iso.cdrom-partscan+4]
> When the ISO is booted as optical media (e.g. `/dev/sr0` in a VM), the
> Linux kernel does not parse the GPT appended partitions because the CD-ROM
> block device driver exposes the device as a single block device with an
> ISO 9660 filesystem. As a result, partition device nodes are never created
> and `/dev/disk/by-partuuid/` symlinks for the appended BESIMAGES and
> BESCONF partitions do not appear.
>
> The installer must handle this transparently. When the well-known
> PARTUUIDs are not present in `/dev/disk/by-partuuid/`, the installer
> must identify the boot device, create a loop device with partition
> scanning enabled, and trigger a udev settle so that the kernel creates
> partition device nodes and udev populates `/dev/disk/by-partuuid/` with
> the well-known PARTUUIDs. Boot device detection must work even when
> `toram` is active (where `/run/live/medium` is backed by tmpfs).
>
> This must happen early in the installer's startup, before any attempt to
> mount BESCONF or open the images partition. The installer must detach the
> loop device on exit. On USB boot, the PARTUUIDs are already visible and
> this step is a no-op.
";

    // Inspect what marq emits for the real content — always passes, read with --nocapture.
    #[test]
    fn test_inspect_real_cdrom_marq_elements() {
        use marq::{DocElement, RenderOptions, render};
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let doc = rt
            .block_on(render(REAL_CDROM_SECTION, &RenderOptions::new()))
            .expect("marq render failed");
        println!(
            "\n=== marq elements for REAL_CDROM_SECTION ({} total) ===",
            doc.elements.len()
        );
        for (i, el) in doc.elements.iter().enumerate() {
            match el {
                DocElement::Heading(h) => println!("  [{i}] Heading({:?})", h.title),
                DocElement::Req(r) => {
                    println!(
                        "  [{i}] Req(id={}, raw_lines={}, raw_bytes={})",
                        r.id,
                        r.raw.lines().count(),
                        r.raw.len()
                    );
                    println!("       raw = {:?}", &r.raw[..r.raw.len().min(200)]);
                }
                DocElement::Paragraph(p) => {
                    let start = p.offset.min(REAL_CDROM_SECTION.len());
                    let rest = &REAL_CDROM_SECTION[start..];
                    let end = rest.find("\n\n").unwrap_or(rest.len());
                    println!(
                        "  [{i}] Paragraph(offset={}) source={:?}",
                        p.offset,
                        &rest[..end.min(80)]
                    );
                }
            }
        }
        println!("=== end ===\n");
    }

    #[test]
    fn test_real_cdrom_produces_one_rule_block() {
        let blocks = run(parse_blocks_from_content(REAL_CDROM_SECTION));
        println!("\n=== parse_blocks_from_content for REAL_CDROM_SECTION ===");
        for (i, b) in blocks.iter().enumerate() {
            match &b.kind {
                SpecBlockKind::Heading { level, text, .. } => {
                    println!("  [{i}] Heading({level}, {text:?})")
                }
                SpecBlockKind::Rule { id, text } => println!(
                    "  [{i}] Rule({id:?}, {} lines, first 80: {:?})",
                    text.lines().count(),
                    &text[..text.len().min(80)]
                ),
                SpecBlockKind::Paragraph { text } => {
                    println!("  [{i}] Paragraph({:?})", &text[..text.len().min(80)])
                }
            }
        }
        println!("=== end ===\n");
        // Heading + one Rule only — no stray Paragraph blocks for the inner paragraphs
        assert_eq!(
            blocks.len(),
            2,
            "expected Heading + 1 Rule, got {} blocks:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_real_cdrom_rule_text_has_all_paragraphs() {
        let blocks = run(parse_blocks_from_content(REAL_CDROM_SECTION));
        let rule = blocks.iter().find(
            |b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "iso.cdrom-partscan+4")
        ).expect("iso.cdrom-partscan+4 not found");
        match &rule.kind {
            SpecBlockKind::Rule { text, .. } => {
                assert!(text.contains("optical media"), "missing para 1: {text:?}");
                assert!(
                    text.contains("handle this transparently"),
                    "missing para 2: {text:?}"
                );
                assert!(
                    text.contains("early in the installer"),
                    "missing para 3: {text:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_inspect_marq_elements_for_multi_para_rule() {
        use marq::{DocElement, RenderOptions, render};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let doc = rt
            .block_on(render(MULTI_PARA_RULE, &RenderOptions::new()))
            .expect("marq render failed");

        println!(
            "\n=== marq elements for MULTI_PARA_RULE ({} total) ===",
            doc.elements.len()
        );
        for (i, el) in doc.elements.iter().enumerate() {
            match el {
                DocElement::Heading(h) => {
                    println!("  [{i}] Heading(level={}, title={:?})", h.level, h.title);
                }
                DocElement::Req(r) => {
                    println!(
                        "  [{i}] Req(id={}, raw_lines={}, raw_bytes={})",
                        r.id,
                        r.raw.lines().count(),
                        r.raw.len(),
                    );
                    println!("       raw = {:?}", r.raw);
                }
                DocElement::Paragraph(p) => {
                    println!("  [{i}] Paragraph(offset={})", p.offset);
                    let start = p.offset.min(MULTI_PARA_RULE.len());
                    let rest = &MULTI_PARA_RULE[start..];
                    let end = rest.find("\n\n").unwrap_or(rest.len());
                    println!("       source_slice = {:?}", &rest[..end]);
                }
            }
        }
        println!("=== end ===\n");
    }
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── Provisional rule ID ───────────────────────────────────────────────────────

// r[impl ids.provisional]
pub fn next_provisional_id() -> String {
    #[cfg(feature = "hydrate")]
    let n = (js_sys::Math::random() * (u32::MAX as f64 + 1.0)) as u32;
    #[cfg(not(feature = "hydrate"))]
    let n = rand::random::<u32>();
    format!("new.{n:08x}+1")
}

// ── Per-session peer ID ───────────────────────────────────────────────────────

/// Return the persistent per-session Loro `PeerID`, creating and storing one in
/// `sessionStorage` on first call.  Uses a u32 range so the value round-trips
/// safely through JSON serialisation (JavaScript `Number.MAX_SAFE_INTEGER`).
#[cfg(feature = "hydrate")]
fn get_or_create_peer_id() -> u64 {
    const KEY: &str = "mcbean_peer_id";
    let storage = web_sys::window().and_then(|w| w.session_storage().ok().flatten());
    if let Some(ref s) = storage
        && let Ok(Some(val)) = s.get_item(KEY)
        && let Ok(id) = val.parse::<u64>()
    {
        return id;
    }
    let id = (js_sys::Math::random() * (u32::MAX as f64 + 1.0)) as u64;
    if let Some(s) = storage {
        s.set_item(KEY, &id.to_string()).ok();
    }
    id
}

// ── SpecBlockEditor component ─────────────────────────────────────────────────

/// Editor component for a proposal in the Drafting state.
///
/// Owns the Loro doc for this proposal.  On mount it fetches the full snapshot
/// from the server, derives the initial `Vec<SpecBlock>` and writes it to
/// `blocks_out`.  All subsequent mutations update the doc in place and push the
/// derived blocks back out.  A debounced delta-sync keeps the server in sync;
/// a background interval poll picks up updates from other peers.
// r[impl edit.rule-text]
// r[impl edit.add-rule]
// r[impl edit.add-section]
// r[impl edit.reorder]
// r[impl edit.delete]
#[component]
pub fn SpecBlockEditor(
    proposal_id: i32,
    /// Reactive output: updated after every local mutation and every successful
    /// server sync.  Initialised to an empty vec; the parent should render a
    /// loading fallback until it becomes non-empty.
    blocks_out: RwSignal<Vec<SpecBlock>>,
    /// Owned by the parent so it can be forwarded to the changelog sidebar.
    /// Set to Some(message) on sync failure, cleared on success.
    sync_error: RwSignal<Option<String>>,
) -> impl IntoView {
    // Key of the block whose textarea is currently open.
    let editing_key: RwSignal<Option<String>> = RwSignal::new(None);
    let edit_draft = RwSignal::new(String::new());

    // HTML5 drag state.
    let drag_key: RwSignal<Option<String>> = RwSignal::new(None);
    let drag_over_bar: RwSignal<Option<String>> = RwSignal::new(None);

    // Sync state.
    let loaded = RwSignal::new(false);
    let syncing = RwSignal::new(false);
    // Bytes of the last version vector we confirmed with the server.
    let synced_vv: RwSignal<Vec<u8>> = RwSignal::new(Vec::new());
    // Incremented on every local mutation; debounce tasks capture it at
    // spawn time and bail out if it has changed.
    let debounce_gen = RwSignal::new(0u32);
    // True when there are local changes not yet confirmed by the server.
    let dirty = RwSignal::new(false);

    // The Loro doc is browser-only state.  During SSR this StoredValue holds a
    // freshly constructed (empty) doc that is never read.
    let loro_doc = StoredValue::new(loro::LoroDoc::new());

    // Suppress unused-variable warnings on SSR where the hydrate-only signals
    // are never referenced.
    #[cfg(not(feature = "hydrate"))]
    let _ = (
        proposal_id,
        loaded,
        syncing,
        sync_error,
        synced_vv,
        debounce_gen,
        dirty,
        loro_doc,
    );
    // sync_error is a prop; keep it in the suppression list so the SSR build
    // does not warn about it being unused (all reads are hydrate-only).

    // ── Initial load ──────────────────────────────────────────────────────────

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            match get_proposal_doc(proposal_id).await {
                Ok(bytes) => {
                    loro_doc.with_value(|doc| {
                        doc.set_peer_id(get_or_create_peer_id()).ok();
                        doc.import(&bytes).ok();
                        let blocks = crate::components::loro_doc::loro_doc_to_blocks(doc);
                        blocks_out.set(blocks);
                        synced_vv.set(crate::components::loro_doc::encode_vv(doc));
                    });
                    loaded.set(true);

                    // Background pass: replace the inline HTML stubs with
                    // proper marq-rendered HTML (RFC 2119 highlighting etc.).
                    // Each block is rendered individually using the same path
                    // as commit_edit, so there is no key-reconciliation risk.
                    let snapshot = blocks_out.get_untracked();
                    for block in snapshot {
                        use marq::{RenderOptions, render};
                        let raw = match &block.kind {
                            SpecBlockKind::Rule { id, text } => {
                                crate::components::loro_doc::rule_to_markdown(id, text)
                            }
                            SpecBlockKind::Paragraph { text } if !text.is_empty() => {
                                format!("{}\n\n", text)
                            }
                            _ => continue,
                        };
                        let key = block.key.clone();
                        leptos::task::spawn_local(async move {
                            if let Ok(rdoc) = render(&raw, &RenderOptions::new()).await {
                                let html = rdoc
                                    .reqs
                                    .first()
                                    .map(|r| r.html.clone())
                                    .unwrap_or(rdoc.html);
                                blocks_out.update(|list| {
                                    if let Some(b) = list.iter_mut().find(|b| b.key == key) {
                                        b.html = html;
                                    }
                                });
                            }
                        });
                    }
                }
                Err(e) => {
                    sync_error.set(Some(format!("Failed to load spec: {e}")));
                }
            }
        });
    });

    // ── Sync helpers ──────────────────────────────────────────────────────────

    // Perform one delta-sync cycle: send our local changes, receive theirs.
    #[cfg(feature = "hydrate")]
    let do_sync = move || {
        if syncing.get_untracked() {
            return;
        }
        syncing.set(true);

        let (vv_snap, delta) = loro_doc.with_value(|doc| {
            let vv_bytes = synced_vv.get_untracked();
            let from_vv = crate::components::loro_doc::decode_vv_or_empty(&vv_bytes);
            let delta = doc
                .export(loro::ExportMode::updates(&from_vv))
                .unwrap_or_default();
            (vv_bytes, delta)
        });

        let peer_id = get_or_create_peer_id().to_string();

        leptos::task::spawn_local(async move {
            match sync_proposal(proposal_id, peer_id, vv_snap, delta).await {
                Ok(remote) => {
                    loro_doc.with_value(|doc| {
                        if !remote.is_empty() {
                            doc.import(&remote).ok();
                            // Rebuild blocks from the updated doc, but carry forward
                            // any HTML that was already marq-rendered so it is never
                            // clobbered by the inline stubs from loro_doc_to_blocks.
                            let mut new_blocks =
                                crate::components::loro_doc::loro_doc_to_blocks(doc);
                            let prev_html: std::collections::HashMap<String, String> = blocks_out
                                .with_untracked(|list| {
                                    list.iter()
                                        .filter(|b| !b.html.is_empty())
                                        .map(|b| (b.key.clone(), b.html.clone()))
                                        .collect()
                                });
                            for b in &mut new_blocks {
                                if let Some(html) = prev_html.get(&b.key) {
                                    b.html = html.clone();
                                }
                            }
                            blocks_out.set(new_blocks);
                        }
                        synced_vv.set(crate::components::loro_doc::encode_vv(doc));
                    });
                    sync_error.set(None);
                    dirty.set(false);
                }
                Err(e) => {
                    sync_error.set(Some(format!("Sync error: {e}")));
                }
            }
            syncing.set(false);
        });
    };

    // Trigger a debounced sync 1.5 s after the last local mutation.
    #[cfg(feature = "hydrate")]
    let trigger_debounced_sync = move || {
        dirty.set(true);
        let dgen = debounce_gen.get_untracked().wrapping_add(1);
        debounce_gen.set(dgen);
        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(1_500).await;
            if debounce_gen.get_untracked() != dgen {
                return;
            }
            do_sync();
        });
    };

    // Background poll: sync every 5 s to receive updates from other peers,
    // even when the current user is not actively editing.
    #[cfg(feature = "hydrate")]
    leptos::task::spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(5_000).await;
            if loaded.get_untracked()
                && !syncing.get_untracked()
                && sync_error.get_untracked().is_none()
            {
                do_sync();
            }
        }
    });

    // BeforeUnload guard: warn if the tab is closed while a sync is pending.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        use wasm_bindgen::JsCast as _;
        use wasm_bindgen::prelude::Closure;
        let is_dirty = dirty.get();
        let Some(window) = web_sys::window() else {
            return;
        };
        if is_dirty {
            let cb = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(
                |ev: web_sys::BeforeUnloadEvent| {
                    ev.prevent_default();
                    ev.set_return_value("Changes may not have been saved.");
                },
            );
            window.set_onbeforeunload(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        } else {
            window.set_onbeforeunload(None);
        }
    });

    // ── Mutation helpers ──────────────────────────────────────────────────────

    // Re-derive the flat block list from the Loro doc and push it out,
    // carrying forward any already-rendered HTML from the previous list so
    // that marq-rendered HTML is never discarded by a structural mutation.
    //
    // `exclude_key`: if Some, that block's HTML is NOT carried forward —
    // it is about to be replaced by a fresh marq render task.
    #[cfg(feature = "hydrate")]
    let rebuild_blocks = move |exclude_key: Option<String>| {
        loro_doc.with_value(|doc| {
            use std::collections::HashMap;
            let mut new_blocks = crate::components::loro_doc::loro_doc_to_blocks(doc);
            let prev_html: HashMap<String, String> = blocks_out.with_untracked(|list| {
                list.iter()
                    .filter(|b| {
                        !b.html.is_empty() && exclude_key.as_deref() != Some(b.key.as_str())
                    })
                    .map(|b| (b.key.clone(), b.html.clone()))
                    .collect()
            });
            for b in &mut new_blocks {
                if let Some(html) = prev_html.get(&b.key) {
                    b.html = html.clone();
                }
            }
            blocks_out.set(new_blocks);
        });
    };

    let open_edit = move |key: String, text: String| {
        edit_draft.set(text);
        editing_key.set(Some(key));
    };

    // Commit the textarea draft on blur: write the new text into the Loro doc
    // and trigger a re-render + debounced sync.
    let commit_edit = move || {
        let Some(key) = editing_key.get_untracked() else {
            return;
        };
        editing_key.set(None);
        let text = edit_draft.get_untracked();
        #[cfg(not(feature = "hydrate"))]
        let _ = (key, text);

        #[cfg(feature = "hydrate")]
        {
            use crate::components::loro_doc::{
                TREE_NAME, key_to_tree_id, loro_doc_to_blocks, set_text_content,
            };

            let changed = loro_doc.with_value(|doc| {
                let Some(node_id) = key_to_tree_id(&key) else {
                    return false;
                };
                let tree = doc.get_tree(TREE_NAME);
                let Ok(meta) = tree.get_meta(node_id) else {
                    return false;
                };
                let current = match meta.get("text") {
                    Some(loro::ValueOrContainer::Container(loro::Container::Text(t))) => {
                        t.to_string()
                    }
                    _ => String::new(),
                };
                current != text
            });

            if !changed {
                return;
            }

            // Write new text into the LoroText container for this node.
            loro_doc.with_value(|doc| {
                let Some(node_id) = key_to_tree_id(&key) else {
                    return;
                };
                let tree = doc.get_tree(TREE_NAME);
                let Ok(meta) = tree.get_meta(node_id) else {
                    return;
                };
                set_text_content(&meta, &text);
            });

            // Re-render this block's HTML client-side via marq (no round-trip).
            {
                use marq::{RenderOptions, render};

                // Use the outer `text` (from edit_draft) in all arms so that a
                // newly-inserted block with empty Loro text still renders the
                // content the user just typed.  Using `..` avoids the match
                // bindings shadowing the outer variable.
                let raw = blocks_out.with_untracked(|list| {
                    list.iter().find(|b| b.key == key).map(|b| match &b.kind {
                        SpecBlockKind::Rule { id, .. } => {
                            crate::components::loro_doc::rule_to_markdown(id, &text)
                        }
                        SpecBlockKind::Heading { level, .. } => {
                            format!("{} {}\n\n", "#".repeat(*level as usize), text)
                        }
                        SpecBlockKind::Paragraph { .. } => {
                            format!("{}\n\n", text)
                        }
                    })
                });

                if let Some(raw) = raw {
                    let key_for_task = key.clone();
                    leptos::task::spawn_local(async move {
                        if let Ok(doc) = render(&raw, &RenderOptions::new()).await {
                            let new_html =
                                doc.reqs.first().map(|r| r.html.clone()).unwrap_or(doc.html);
                            blocks_out.update(|list| {
                                if let Some(b) = list.iter_mut().find(|b| b.key == key_for_task) {
                                    b.html = new_html;
                                }
                            });
                        }
                    });
                }
            }

            // Rebuild blocks_out with updated text, preserving rendered HTML
            // for every block except this one (whose HTML the marq task above
            // will fill in momentarily).
            rebuild_blocks(Some(key.clone()));

            trigger_debounced_sync();
        }
    };

    let revert_edit = move || {
        editing_key.set(None);
    };

    // r[impl edit.delete]
    let delete_block = move |key: String| {
        if editing_key.get_untracked().as_deref() == Some(&key) {
            editing_key.set(None);
        }

        #[cfg(feature = "hydrate")]
        {
            use crate::components::loro_doc::{TREE_NAME, key_to_tree_id};

            loro_doc.with_value(|doc| {
                let Some(node_id) = key_to_tree_id(&key) else {
                    return;
                };
                let tree = doc.get_tree(TREE_NAME);
                tree.delete(node_id).ok();
            });
            rebuild_blocks(None);
            trigger_debounced_sync();
        }
    };

    // r[impl edit.reorder]
    let handle_drop = move |from_key: String, drop_key: String| {
        if from_key == drop_key {
            drag_key.set(None);
            drag_over_bar.set(None);
            return;
        }

        #[cfg(feature = "hydrate")]
        {
            use crate::components::loro_doc::{TREE_NAME, key_to_tree_id};
            use loro::TreeParentId;

            loro_doc.with_value(|doc| {
                let tree = doc.get_tree(TREE_NAME);

                let Some(from_id) = key_to_tree_id(&from_key) else {
                    return;
                };

                if drop_key == TOP_DROP_KEY {
                    // Move before the first block in the flat list.
                    let first_key =
                        blocks_out.with_untracked(|bs| bs.first().map(|b| b.key.clone()));
                    if let Some(first_key) = first_key
                        && let Some(first_id) = key_to_tree_id(&first_key)
                        && let Some(parent) = tree.parent(first_id)
                        && !matches!(parent, TreeParentId::Deleted | TreeParentId::Unexist)
                    {
                        tree.mov_to(from_id, parent, 0).ok();
                    }
                } else {
                    let Some(drop_id) = key_to_tree_id(&drop_key) else {
                        return;
                    };
                    let Some(parent) = tree.parent(drop_id) else {
                        return;
                    };
                    if matches!(parent, TreeParentId::Deleted | TreeParentId::Unexist) {
                        return;
                    }
                    let siblings = tree.children(parent).unwrap_or_default();
                    let idx = siblings
                        .iter()
                        .position(|s| *s == drop_id)
                        .map(|i| i + 1)
                        .unwrap_or(siblings.len());
                    tree.mov_to(from_id, parent, idx).ok();
                }
            });

            rebuild_blocks(None);
            trigger_debounced_sync();
        }

        drag_key.set(None);
        drag_over_bar.set(None);
    };

    // r[impl edit.add-rule]
    // r[impl edit.add-section]
    let insert_block = move |after_key: String, kind: SpecBlockKind| {
        #[cfg(not(feature = "hydrate"))]
        let _ = (after_key, kind);
        #[cfg(feature = "hydrate")]
        {
            use crate::components::loro_doc::{
                TREE_NAME, key_to_tree_id, set_text_content, tree_id_to_key,
            };
            use loro::TreeParentId;

            let new_key = loro_doc.with_value(|doc| {
                let tree = doc.get_tree(TREE_NAME);

                let (parent, idx) = if after_key == TOP_DROP_KEY {
                    // Insert at position 0 under the first block's parent.
                    let first_key =
                        blocks_out.with_untracked(|bs| bs.first().map(|b| b.key.clone()));
                    if let Some(fk) = first_key {
                        if let Some(fid) = key_to_tree_id(&fk) {
                            if let Some(p) = tree.parent(fid) {
                                if !matches!(p, TreeParentId::Deleted | TreeParentId::Unexist) {
                                    (p, 0usize)
                                } else {
                                    (TreeParentId::Root, 0)
                                }
                            } else {
                                (TreeParentId::Root, 0)
                            }
                        } else {
                            (TreeParentId::Root, 0)
                        }
                    } else {
                        (TreeParentId::Root, 0)
                    }
                } else {
                    let after_id = match key_to_tree_id(&after_key) {
                        Some(id) => id,
                        None => return None,
                    };
                    let parent = match tree.parent(after_id) {
                        Some(p) if !matches!(p, TreeParentId::Deleted | TreeParentId::Unexist) => p,
                        _ => TreeParentId::Root,
                    };
                    let siblings = tree.children(parent).unwrap_or_default();
                    let idx = siblings
                        .iter()
                        .position(|s| *s == after_id)
                        .map(|i| i + 1)
                        .unwrap_or(siblings.len());
                    (parent, idx)
                };

                let Ok(node_id) = tree.create_at(parent, idx) else {
                    return None;
                };
                let Ok(meta) = tree.get_meta(node_id) else {
                    return None;
                };

                match &kind {
                    SpecBlockKind::Heading { level, text, .. } => {
                        meta.insert("kind", "heading").ok();
                        meta.insert("level", *level as i64).ok();
                        set_text_content(&meta, text);
                    }
                    SpecBlockKind::Rule { id, text } => {
                        meta.insert("kind", "rule").ok();
                        meta.insert("rule_id", id.as_str()).ok();
                        set_text_content(&meta, text);
                    }
                    SpecBlockKind::Paragraph { text } => {
                        meta.insert("kind", "para").ok();
                        set_text_content(&meta, text);
                    }
                }

                Some(tree_id_to_key(node_id))
            });

            rebuild_blocks(None);
            trigger_debounced_sync();

            if let Some(k) = new_key {
                let init_text = match &kind {
                    SpecBlockKind::Heading { text, .. } => text.clone(),
                    SpecBlockKind::Rule { text, .. } => text.clone(),
                    SpecBlockKind::Paragraph { text } => text.clone(),
                };
                open_edit(k, init_text);
            }
        }
    };

    // ── View ──────────────────────────────────────────────────────────────────

    view! {
        <div class="spec-block-editor">

            // Insert bar above all blocks.
            <InsertBar
                drop_key=TOP_DROP_KEY.to_string()
                drag_key=drag_key
                drag_over_bar=drag_over_bar
                on_drop_block=Callback::new(move |(from, to)| handle_drop(from, to))
                on_insert_rule=Callback::new(move |_| {
                    insert_block(
                        TOP_DROP_KEY.to_string(),
                        SpecBlockKind::Rule {
                            id: next_provisional_id(),
                            text: String::new(),
                        },
                    )
                })
                on_insert_heading=Callback::new(move |lvl: u8| {
                    insert_block(
                        TOP_DROP_KEY.to_string(),
                        SpecBlockKind::Heading {
                            level: lvl,
                            text: String::new(),
                            anchor: String::new(),
                        },
                    )
                })
            />

            <For
                each=move || blocks_out.get()
                key=|b| b.key.clone()
                children=move |block| {
                    let key = StoredValue::new(block.key.clone());

                    let rule_id: StoredValue<Option<String>> =
                        StoredValue::new(if let SpecBlockKind::Rule { id, .. } = &block.kind {
                            Some(id.clone())
                        } else {
                            None
                        });
                    let heading_level: StoredValue<Option<u8>> =
                        StoredValue::new(if let SpecBlockKind::Heading { level, .. } = &block.kind {
                            Some(*level)
                        } else {
                            None
                        });
                    let heading_anchor: StoredValue<Option<String>> =
                        StoredValue::new(if let SpecBlockKind::Heading { anchor, .. } = &block.kind {
                            Some(anchor.clone())
                        } else {
                            None
                        });

                    let block_anchor = move || {
                        if heading_anchor.get_value().is_none() {
                            return String::new();
                        }
                        blocks_out.with(|list| {
                            list.iter()
                                .find(|b| b.key == key.get_value())
                                .and_then(|b| {
                                    if let SpecBlockKind::Heading { anchor, .. } = &b.kind {
                                        Some(anchor.clone())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_default()
                        })
                    };

                    let block_html = move || {
                        blocks_out.with(|list| {
                            list.iter()
                                .find(|b| b.key == key.get_value())
                                .map(|b| b.html.clone())
                                .unwrap_or_default()
                        })
                    };
                    let block_text = move || {
                        blocks_out.with(|list| {
                            list.iter()
                                .find(|b| b.key == key.get_value())
                                .map(|b| b.edit_text().to_owned())
                                .unwrap_or_default()
                        })
                    };

                    let is_editing =
                        move || editing_key.get().as_deref() == Some(&key.get_value());
                    let is_dragging =
                        move || drag_key.get().as_deref() == Some(&key.get_value());

                    let (ta_font_size, ta_font_weight) = match block.kind {
                        SpecBlockKind::Heading { level: 1, .. } => ("2rem", "bold"),
                        SpecBlockKind::Heading { level: 2, .. } => ("1.5rem", "bold"),
                        SpecBlockKind::Heading { level: 3, .. } => ("1.25rem", "600"),
                        SpecBlockKind::Heading { level: 4, .. } => ("1.1rem", "600"),
                        _ => ("1rem", "normal"),
                    };

                    view! {
                        <div
                            class="spec-block-wrapper"
                            class:spec-block--dragging=is_dragging
                            id=block_anchor
                            on:dragover=move |e| e.prevent_default()
                        >
                            <div class="spec-block-header">
                                <span
                                    class="spec-block-drag-handle"
                                    title="Drag to reorder"
                                    draggable="true"
                                    on:dragstart=move |ev| {
                                        ev.stop_propagation();
                                        drag_key.set(Some(key.get_value()));
                                    }
                                    on:dragend=move |_| {
                                        drag_key.set(None);
                                        drag_over_bar.set(None);
                                    }
                                >
                                    "⠿"
                                </span>

                                {move || {
                                    rule_id.with_value(|rid| {
                                        if let Some(id) = rid {
                                            view! {
                                                <span class="spec-block-badge spec-block-badge--rule">
                                                    {format!("r[{}]", id)}
                                                </span>
                                            }
                                            .into_any()
                                        } else if let Some(lvl) = heading_level.get_value() {
                                            view! {
                                                <span class="spec-block-badge spec-block-badge--heading">
                                                    {format!("H{}", lvl)}
                                                </span>
                                            }
                                            .into_any()
                                        } else {
                                            view! {
                                                <span class="spec-block-badge spec-block-badge--para">
                                                    "¶"
                                                </span>
                                            }
                                            .into_any()
                                        }
                                    })
                                }}

                                // r[impl edit.delete]
                                <button
                                    class="spec-action-btn spec-action-btn--delete"
                                    on:click=move |_| delete_block(key.get_value())
                                >
                                    "Delete"
                                </button>
                            </div>

                            <Show
                                when=is_editing
                                fallback=move || {
                                    let html = block_html();
                                    let is_empty = html.is_empty();
                                    view! {
                                        // r[impl edit.rule-text]
                                        <div
                                            class="spec-block-display"
                                            class:spec-block-display--empty=is_empty
                                            title="Click to edit"
                                            on:click=move |_| {
                                                open_edit(key.get_value(), block_text());
                                            }
                                        >
                                            {if !html.is_empty() {
                                                view! {
                                                    <div class="content" inner_html=html />
                                                }
                                                .into_any()
                                            } else {
                                                view! {
                                                    <span class="spec-block-placeholder">
                                                        "Click to add content\u{2026}"
                                                    </span>
                                                }
                                                .into_any()
                                            }}
                                        </div>
                                    }
                                }
                            >
                                // r[impl edit.rule-text]
                                <div
                                    class="grow-wrap"
                                    style:font-size=ta_font_size
                                    style:font-weight=ta_font_weight
                                >
                                    <div class="grow-wrap-mirror" aria-hidden="true">
                                        {move || format!("{} ", edit_draft.get())}
                                    </div>
                                    <textarea
                                        class="spec-block-textarea"
                                        autofocus=true
                                        prop:value=move || edit_draft.get()
                                        on:input=move |ev| {
                                            edit_draft.set(event_target_value(&ev));
                                        }
                                        on:blur=move |_| commit_edit()
                                        on:keydown=move |ev| {
                                            if ev.key() == "Escape" {
                                                revert_edit();
                                            }
                                        }
                                    />
                                </div>
                            </Show>

                            <InsertBar
                                drop_key=key.get_value()
                                drag_key=drag_key
                                drag_over_bar=drag_over_bar
                                on_drop_block=Callback::new(move |(from, to)| {
                                    handle_drop(from, to)
                                })
                                on_insert_rule=Callback::new(move |_| {
                                    insert_block(
                                        key.get_value(),
                                        SpecBlockKind::Rule {
                                            id: next_provisional_id(),
                                            text: String::new(),
                                        },
                                    )
                                })
                                on_insert_heading=Callback::new(move |lvl: u8| {
                                    insert_block(
                                        key.get_value(),
                                        SpecBlockKind::Heading {
                                            level: lvl,
                                            text: String::new(),
                                            anchor: String::new(),
                                        },
                                    )
                                })
                            />
                        </div>
                    }
                }
            />
        </div>
    }
}

// ── InsertBar ─────────────────────────────────────────────────────────────────

#[component]
fn InsertBar(
    drop_key: String,
    drag_key: RwSignal<Option<String>>,
    drag_over_bar: RwSignal<Option<String>>,
    on_drop_block: Callback<(String, String)>,
    on_insert_rule: Callback<()>,
    on_insert_heading: Callback<u8>,
) -> impl IntoView {
    let drop_key = StoredValue::new(drop_key);
    let menu_open = RwSignal::new(false);

    let is_drag_active = move || drag_key.get().is_some();
    let is_hovered = move || drag_over_bar.get().as_deref() == Some(&drop_key.get_value());

    Effect::new(move |_| {
        if drag_key.get().is_some() {
            menu_open.set(false);
        }
    });

    view! {
        <div
            class="insert-bar"
            class:insert-bar--drag-active=is_drag_active
            class:insert-bar--hovered=is_hovered
            on:dragover=move |e| {
                e.prevent_default();
                drag_over_bar.set(Some(drop_key.get_value()));
            }
            on:dragleave=move |_| {
                drag_over_bar.update(|k| {
                    if k.as_deref() == Some(&drop_key.get_value()) {
                        *k = None;
                    }
                });
            }
            on:drop=move |e| {
                e.prevent_default();
                if let Some(from) = drag_key.get_untracked() {
                    on_drop_block.run((from, drop_key.get_value()));
                }
            }
        >
            <div class="insert-bar-line" />

            <Show when=move || !is_drag_active()>
                <div class="insert-bar-controls">
                    <button
                        class="insert-bar-toggle"
                        class:is-active=move || menu_open.get()
                        on:click=move |_| menu_open.update(|v| *v = !*v)
                    >
                        "Add item"
                    </button>

                    <Show when=move || menu_open.get()>
                        <div class="insert-bar-menu">
                            // r[impl edit.add-rule]
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_rule.run(());
                                }
                            >
                                "Rule"
                            </button>
                            // r[impl edit.add-section]
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_heading.run(1);
                                }
                            >
                                "H1"
                            </button>
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_heading.run(2);
                                }
                            >
                                "H2"
                            </button>
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_heading.run(3);
                                }
                            >
                                "H3"
                            </button>
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_heading.run(4);
                                }
                            >
                                "H4"
                            </button>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    }
}
