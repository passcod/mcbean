use loro::{
    Container, LoroDoc, LoroMap, LoroText, LoroTree, LoroValue, PeerID, TreeID, TreeParentId,
    ValueOrContainer, VersionVector,
};
#[cfg(feature = "ssr")]
use tracing::{debug, info};

use crate::components::spec_block_editor::{SpecBlock, SpecBlockKind, slugify};

/// Serialise a rule back to Markdown in a form that marq will re-parse
/// correctly, preserving all paragraphs.
///
/// Single-paragraph rules use the compact inline form:
/// ```text
/// r[id]
/// The rule text.
/// ```
///
/// Multi-paragraph rules (text contains `\n\n`) use blockquote form so that
/// marq captures every paragraph as part of the same `Req` element:
/// ```text
/// > r[id]
/// > First paragraph line 1
/// > First paragraph line 2
/// >
/// > Second paragraph line 1
/// ```
pub fn rule_to_markdown(prefix: &str, rule_id: &str, text: &str) -> String {
    let trimmed = text.trim();
    let mut out = format!("> {prefix}[{rule_id}]\n");
    let paragraphs: Vec<&str> = trimmed.split("\n\n").collect();
    for (i, para) in paragraphs.iter().enumerate() {
        if i > 0 {
            out.push_str(">\n");
        }
        for line in para.lines() {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

/// Truncate `s` to at most `max_bytes` bytes while respecting UTF-8 char boundaries.
#[cfg(feature = "ssr")]
pub(crate) fn preview(s: &str, max_bytes: usize) -> &str {
    let end = max_bytes.min(s.len());
    let mut boundary = end;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

// ── Tree shape constants ──────────────────────────────────────────────────────

const KIND_SPEC: &str = "spec";
const KIND_FILE: &str = "file";
const KIND_HEADING: &str = "heading";
const KIND_RULE: &str = "rule";
const KIND_PARA: &str = "para";

/// Name of the single `LoroTree` container inside every doc.
pub const TREE_NAME: &str = "doc";

/// Loro `PeerID` used when the server constructs the initial base snapshot from
/// spec files.  Client sessions always use randomly-generated per-session peer
/// IDs that are distinct from this value.
pub const SERVER_PEER_ID: PeerID = 1;

// ── TreeID ↔ SpecBlock key ────────────────────────────────────────────────────

/// Encode a `TreeID` as a stable, URL-safe string key.
/// Format: `"<peer>:<counter>"` — both fields are stable identifiers within
/// a given Loro doc and never change once a node is created.
pub fn tree_id_to_key(id: TreeID) -> String {
    format!("{}:{}", id.peer, id.counter)
}

pub fn key_to_tree_id(key: &str) -> Option<TreeID> {
    let (peer_str, counter_str) = key.split_once(':')?;
    Some(TreeID::new(
        peer_str.parse().ok()?,
        counter_str.parse().ok()?,
    ))
}

// ── Flatten: Loro doc → Vec<SpecBlock> ───────────────────────────────────────

/// Flatten the Loro tree into an ordered `Vec<SpecBlock>` for the editor UI.
///
/// Spec and file nodes are structural containers and are not emitted as blocks.
/// Heading nodes are emitted and then recursed into, so their descendants
/// appear immediately after them in document order — the editor sees a flat
/// list while the CRDT retains the full hierarchy.
pub fn loro_doc_to_blocks(doc: &LoroDoc) -> Vec<SpecBlock> {
    let tree = doc.get_tree(TREE_NAME);
    let mut out = Vec::new();
    collect_blocks_under(&tree, TreeParentId::Root, &mut out);
    out
}

fn collect_blocks_under(tree: &LoroTree, parent: TreeParentId, out: &mut Vec<SpecBlock>) {
    let children = match tree.children(parent) {
        Some(c) => c,
        None => return,
    };
    for node_id in children {
        let meta = match tree.get_meta(node_id) {
            Ok(m) => m,
            Err(_) => continue,
        };
        match get_str(&meta, "kind").as_str() {
            KIND_SPEC | KIND_FILE => {
                collect_blocks_under(tree, TreeParentId::Node(node_id), out);
            }
            KIND_HEADING => {
                let text = get_text_str(&meta);
                let level = get_i64(&meta, "level").unwrap_or(1).clamp(1, 6) as u8;
                let anchor = slugify(&text);
                let html = format!(
                    "<h{level} id=\"{anchor}\">{}</h{level}>",
                    html_escape(&text),
                );
                out.push(SpecBlock {
                    key: tree_id_to_key(node_id),
                    kind: SpecBlockKind::Heading {
                        level,
                        text,
                        anchor,
                    },
                    html,
                });
                // Recurse so children appear immediately after in doc order.
                collect_blocks_under(tree, TreeParentId::Node(node_id), out);
            }
            KIND_RULE => {
                let text = get_text_str(&meta);
                let rule_id = get_str(&meta, "rule_id");
                let prefix = get_str(&meta, "prefix");
                let prefix = if prefix.is_empty() {
                    "r".to_string()
                } else {
                    prefix
                };
                // Generate HTML that matches the .req/.req-link CSS structure.
                // commit_edit will replace this with a full marq-rendered version
                // after the user edits the block; the background render pass in
                // SpecBlockEditor replaces it with marq HTML shortly after load.
                let html = format!(
                    "<div class=\"req\">\
                     <a class=\"req-link\" id=\"{id}\" href=\"#{id}\">\
                     <span>{prefix}[{id}]</span></a>\
                     <p>{text}</p>\
                     </div>",
                    prefix = html_escape(&prefix),
                    id = html_escape(&rule_id),
                    text = html_escape(&text),
                );
                out.push(SpecBlock {
                    key: tree_id_to_key(node_id),
                    kind: SpecBlockKind::Rule {
                        prefix,
                        id: rule_id,
                        text,
                    },
                    html,
                });
            }
            KIND_PARA => {
                let text = get_text_str(&meta);
                let html = format!("<p>{}</p>", html_escape(&text));
                out.push(SpecBlock {
                    key: tree_id_to_key(node_id),
                    kind: SpecBlockKind::Paragraph { text },
                    html,
                });
            }
            _ => {}
        }
    }
}

// ── Build: spec file content → LoroDoc ───────────────────────────────────────

/// Build a fresh `LoroDoc` from raw spec Markdown content.
///
/// `specs` is a list of `(spec_name, [(file_path, file_content)])`.
/// Each file's content is parsed via marq to obtain a flat block list, which
/// is then inserted into the tree with heading nodes nested by level.
///
/// The resulting doc uses [`SERVER_PEER_ID`] and is ready to be exported as a
/// base snapshot with `ExportMode::Snapshot`.
#[cfg(feature = "ssr")]
pub async fn build_doc_from_specs(specs: &[(String, Vec<(String, String)>)]) -> LoroDoc {
    use crate::components::spec_block_editor::parse_blocks_from_content;

    info!(
        spec_count = specs.len(),
        specs = ?specs.iter().map(|(n, files)| format!("{n} ({} files)", files.len())).collect::<Vec<_>>(),
        "build_doc_from_specs: entry"
    );

    let doc = LoroDoc::new();
    doc.set_peer_id(SERVER_PEER_ID).expect("set server peer id");
    let tree = doc.get_tree(TREE_NAME);

    for (spec_name, files) in specs {
        info!(spec = %spec_name, file_count = files.len(), "build_doc_from_specs: processing spec");

        let spec_node = tree.create(TreeParentId::Root).expect("create spec node");
        let spec_meta = tree.get_meta(spec_node).expect("spec meta");
        spec_meta.insert("kind", KIND_SPEC).unwrap();
        spec_meta.insert("name", spec_name.as_str()).unwrap();

        let mut sorted_files: Vec<&(String, String)> = files.iter().collect();
        sorted_files.sort_by(|a, b| a.0.cmp(&b.0));

        for (file_path, file_content) in sorted_files {
            info!(
                spec = %spec_name,
                path = %file_path,
                content_bytes = file_content.len(),
                "build_doc_from_specs: about to call parse_blocks_from_content"
            );

            let file_node = tree
                .create(TreeParentId::Node(spec_node))
                .expect("create file node");
            let file_meta = tree.get_meta(file_node).expect("file meta");
            file_meta.insert("kind", KIND_FILE).unwrap();
            file_meta.insert("path", file_path.as_str()).unwrap();

            let blocks = parse_blocks_from_content(file_content).await;

            info!(
                spec = %spec_name,
                path = %file_path,
                block_count = blocks.len(),
                "build_doc_from_specs: parse_blocks_from_content returned, about to insert_flat_blocks"
            );

            insert_flat_blocks(&tree, file_node, &blocks);

            info!(
                spec = %spec_name,
                path = %file_path,
                "build_doc_from_specs: insert_flat_blocks complete"
            );
        }

        info!(spec = %spec_name, "build_doc_from_specs: spec complete");
    }

    info!("build_doc_from_specs: all specs processed, returning doc");
    doc
}

/// Insert a flat `Vec<SpecBlock>` under `file_node`, nesting heading nodes so
/// that a heading at level N becomes a child of the nearest ancestor heading
/// with level < N, or of the file node if no such ancestor exists.
///
/// The stack tracks `(heading_level, node_id)` pairs; the file node is treated
/// as an implicit level-0 root for the stack.
#[cfg(feature = "ssr")]
fn insert_flat_blocks(tree: &LoroTree, file_node: TreeID, blocks: &[SpecBlock]) {
    debug!(block_count = blocks.len(), "insert_flat_blocks: entry");
    let mut stack: Vec<(u8, TreeID)> = vec![(0, file_node)];

    for (i, block) in blocks.iter().enumerate() {
        match &block.kind {
            SpecBlockKind::Heading { level, text, .. } => {
                debug!(index = i, %level, text = %text, "insert_flat_blocks: heading");
                while stack.len() > 1 && stack.last().map(|(l, _)| *l).unwrap_or(0) >= *level {
                    stack.pop();
                }
                let parent = stack.last().map(|(_, id)| *id).unwrap_or(file_node);
                let node = tree
                    .create(TreeParentId::Node(parent))
                    .expect("create heading node");
                let meta = tree.get_meta(node).expect("heading meta");
                meta.insert("kind", KIND_HEADING).unwrap();
                meta.insert("level", *level as i64).unwrap();
                debug!(
                    index = i,
                    "insert_flat_blocks: calling set_text_content for heading"
                );
                set_text_content(&meta, text);
                debug!(index = i, "insert_flat_blocks: heading done");
                stack.push((*level, node));
            }
            SpecBlockKind::Rule { prefix, id, text } => {
                debug!(
                    index = i,
                    rule_id = %id,
                    text_bytes = text.len(),
                    text_preview = %preview(text, 120),
                    "insert_flat_blocks: rule"
                );
                let parent = stack.last().map(|(_, id)| *id).unwrap_or(file_node);
                let node = tree
                    .create(TreeParentId::Node(parent))
                    .expect("create rule node");
                let meta = tree.get_meta(node).expect("rule meta");
                meta.insert("kind", KIND_RULE).unwrap();
                meta.insert("rule_id", id.as_str()).unwrap();
                meta.insert("prefix", prefix.as_str()).unwrap();
                debug!(index = i, rule_id = %id, "insert_flat_blocks: calling set_text_content for rule");
                set_text_content(&meta, text);
                debug!(index = i, rule_id = %id, "insert_flat_blocks: rule done");
            }
            SpecBlockKind::Paragraph { text } => {
                debug!(
                    index = i,
                    text_bytes = text.len(),
                    text_preview = %preview(text, 80),
                    "insert_flat_blocks: paragraph"
                );
                let parent = stack.last().map(|(_, id)| *id).unwrap_or(file_node);
                let node = tree
                    .create(TreeParentId::Node(parent))
                    .expect("create para node");
                let meta = tree.get_meta(node).expect("para meta");
                meta.insert("kind", KIND_PARA).unwrap();
                debug!(
                    index = i,
                    "insert_flat_blocks: calling set_text_content for para"
                );
                set_text_content(&meta, text);
                debug!(index = i, "insert_flat_blocks: para done");
            }
        }
    }

    debug!("insert_flat_blocks: complete");
}

// ── Reconstruct: base snapshot + stored updates → LoroDoc ────────────────────

/// Reconstruct the current state of a proposal by importing the base snapshot
/// and then replaying every stored update in ascending `id` order.
///
/// Empty slices in `update_rows` are silently skipped so callers do not need
/// to filter them out.
#[cfg(feature = "ssr")]
pub fn reconstruct_doc(base: &[u8], update_rows: &[Vec<u8>]) -> anyhow::Result<LoroDoc> {
    let doc = LoroDoc::new();
    doc.import(base)
        .map_err(|e| anyhow::anyhow!("import base snapshot: {e}"))?;
    for row in update_rows {
        if !row.is_empty() {
            doc.import(row)
                .map_err(|e| anyhow::anyhow!("import update: {e}"))?;
        }
    }
    Ok(doc)
}

// ── Rule ID manipulation ──────────────────────────────────────────────────────

/// Apply a set of rule ID renames to the Loro doc.
///
/// Each entry in `overrides` is `(tree_key, new_rule_id)` where `tree_key` is
/// the `"peer:counter"` key of a tree node whose `rule_id` metadata should be
/// replaced.  Only nodes of kind `"rule"` are touched.
#[cfg(feature = "ssr")]
pub fn rename_rule_ids(doc: &LoroDoc, overrides: &[(String, String)]) -> anyhow::Result<()> {
    if overrides.is_empty() {
        return Ok(());
    }
    let tree = doc.get_tree(TREE_NAME);
    for (key, new_id) in overrides {
        let tid = key_to_tree_id(key).ok_or_else(|| anyhow::anyhow!("invalid tree key: {key}"))?;
        let meta = tree
            .get_meta(tid)
            .map_err(|e| anyhow::anyhow!("get_meta for {key}: {e}"))?;
        let kind = get_str(&meta, "kind");
        if kind != KIND_RULE {
            return Err(anyhow::anyhow!(
                "node {key} is kind '{kind}', expected 'rule'"
            ));
        }
        meta.insert("rule_id", new_id.as_str())
            .map_err(|e| anyhow::anyhow!("set rule_id on {key}: {e}"))?;
    }
    Ok(())
}

/// Returns `true` if any rule node in the doc still carries a provisional ID
/// (one starting with `"new."`).
#[cfg(feature = "ssr")]
pub fn has_provisional_ids(doc: &LoroDoc) -> bool {
    let tree = doc.get_tree(TREE_NAME);
    has_provisional_ids_under(&tree, TreeParentId::Root)
}

#[cfg(feature = "ssr")]
fn has_provisional_ids_under(tree: &LoroTree, parent: TreeParentId) -> bool {
    for node_id in tree.children(parent).unwrap_or_default() {
        let meta = match tree.get_meta(node_id) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let kind = get_str(&meta, "kind");
        match kind.as_str() {
            KIND_RULE => {
                let rule_id = get_str(&meta, "rule_id");
                if rule_id.starts_with("new.") {
                    return true;
                }
            }
            KIND_SPEC | KIND_FILE | KIND_HEADING => {
                if has_provisional_ids_under(tree, TreeParentId::Node(node_id)) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

// ── Serialize: LoroDoc → per-file Markdown ────────────────────────────────────

/// Walk the entire tree and produce `(file_path, markdown_content)` pairs for
/// every file node, visiting specs and files in tree order.
///
/// Used when writing a proposal branch back to GitHub: each pair maps directly
/// to a file that must be created or updated in the repository.
pub fn doc_to_markdown_files(doc: &LoroDoc) -> Vec<(String, String)> {
    let tree = doc.get_tree(TREE_NAME);
    let mut out = Vec::new();

    for spec_id in tree.children(TreeParentId::Root).unwrap_or_default() {
        let spec_meta = match tree.get_meta(spec_id) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if get_str(&spec_meta, "kind") != KIND_SPEC {
            continue;
        }
        for file_id in tree
            .children(TreeParentId::Node(spec_id))
            .unwrap_or_default()
        {
            let file_meta = match tree.get_meta(file_id) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if get_str(&file_meta, "kind") != KIND_FILE {
                continue;
            }
            let path = get_str(&file_meta, "path");
            let mut content = String::new();
            write_markdown_under(&tree, TreeParentId::Node(file_id), &mut content);
            out.push((path, content));
        }
    }

    out
}

fn write_markdown_under(tree: &LoroTree, parent: TreeParentId, out: &mut String) {
    for node_id in tree.children(parent).unwrap_or_default() {
        let meta = match tree.get_meta(node_id) {
            Ok(m) => m,
            Err(_) => continue,
        };
        match get_str(&meta, "kind").as_str() {
            KIND_HEADING => {
                let level = get_i64(&meta, "level").unwrap_or(1).clamp(1, 6) as usize;
                let text = get_text_str(&meta);
                out.push_str(&"#".repeat(level));
                out.push(' ');
                out.push_str(text.trim());
                out.push_str("\n\n");
                write_markdown_under(tree, TreeParentId::Node(node_id), out);
            }
            KIND_RULE => {
                let rule_id = get_str(&meta, "rule_id");
                let text = get_text_str(&meta);
                let prefix = get_str(&meta, "prefix");
                let prefix = if prefix.is_empty() {
                    "r".to_string()
                } else {
                    prefix
                };
                out.push_str(&rule_to_markdown(&prefix, &rule_id, &text));
            }
            KIND_PARA => {
                let text = get_text_str(&meta);
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                }
            }
            _ => {}
        }
    }
}

// ── Version-vector helpers ────────────────────────────────────────────────────

/// Encode the doc's current oplog version vector for network transfer.
pub fn encode_vv(doc: &LoroDoc) -> Vec<u8> {
    doc.oplog_vv().encode()
}

/// Decode a version vector from network bytes, returning an empty `VersionVector`
/// on any failure.  An empty VV tells the server "I have nothing", so it will
/// respond with all known updates — the correct behaviour for a fresh client.
pub fn decode_vv_or_empty(bytes: &[u8]) -> VersionVector {
    if bytes.is_empty() {
        return VersionVector::default();
    }
    VersionVector::decode(bytes).unwrap_or_default()
}

// ── Text-container mutation ───────────────────────────────────────────────────

/// Set the `"text"` `LoroText` sub-container on a node's meta map to the given
/// content string, replacing any existing content.
///
/// If the container does not yet exist it is created.  This is the only correct
/// way to write text content — storing plain strings in the map would lose
/// fine-grained CRDT merge capability for concurrent edits.
pub fn set_text_content(meta: &LoroMap, content: &str) {
    #[cfg(feature = "ssr")]
    debug!(content_bytes = content.len(), "set_text_content: entry");
    match meta.get("text") {
        Some(ValueOrContainer::Container(Container::Text(t))) => {
            let len = t.len_unicode();
            #[cfg(feature = "ssr")]
            debug!(
                existing_len = len,
                "set_text_content: found existing LoroText, replacing"
            );
            if len > 0 {
                t.delete(0, len).ok();
            }
            if !content.is_empty() {
                t.insert(0, content).ok();
            }
        }
        _ => {
            #[cfg(feature = "ssr")]
            debug!("set_text_content: creating new LoroText container");
            let t = meta
                .insert_container("text", LoroText::new())
                .expect("insert text container");
            if !content.is_empty() {
                #[cfg(feature = "ssr")]
                debug!(
                    content_bytes = content.len(),
                    "set_text_content: inserting into new container"
                );
                t.insert(0, content).ok();
            }
        }
    }
    #[cfg(feature = "ssr")]
    debug!("set_text_content: done");
}

// ── Low-level meta accessors ──────────────────────────────────────────────────

fn get_str(meta: &LoroMap, key: &str) -> String {
    match meta.get(key) {
        Some(ValueOrContainer::Value(LoroValue::String(s))) => s.to_string(),
        _ => String::new(),
    }
}

fn get_i64(meta: &LoroMap, key: &str) -> Option<i64> {
    match meta.get(key) {
        Some(ValueOrContainer::Value(LoroValue::I64(n))) => Some(n),
        Some(ValueOrContainer::Value(LoroValue::Double(n))) => Some(n as i64),
        _ => None,
    }
}

fn get_text_str(meta: &LoroMap) -> String {
    match meta.get("text") {
        Some(ValueOrContainer::Container(Container::Text(t))) => t.to_string(),
        _ => String::new(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── Full-pipeline tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::spec_block_editor::SpecBlockKind;

    fn run<F: std::future::Future<Output = LoroDoc>>(f: F) -> LoroDoc {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    /// Build a doc from a single spec/file and flatten it back to blocks.
    fn pipeline(content: &str) -> Vec<crate::components::spec_block_editor::SpecBlock> {
        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = run(build_doc_from_specs(&specs));
        loro_doc_to_blocks(&doc)
    }

    // ── basic sanity ─────────────────────────────────────────────────────────

    // r[verify view.render]
    #[test]
    fn test_pipeline_single_rule() {
        let blocks = pipeline("r[foo.bar]\nThe rule text.\n");
        assert_eq!(blocks.len(), 1, "{blocks:#?}");
        match &blocks[0].kind {
            SpecBlockKind::Rule { id, text, .. } => {
                assert_eq!(id, "foo.bar");
                assert_eq!(text, "The rule text.");
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // r[verify view.render]
    #[test]
    fn test_pipeline_heading_and_rule() {
        let blocks = pipeline("# Section\n\nr[s.rule]\nSome text.\n");
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert!(matches!(
            &blocks[0].kind,
            SpecBlockKind::Heading { level: 1, .. }
        ));
        assert!(matches!(&blocks[1].kind, SpecBlockKind::Rule { .. }));
    }

    // ── multi-paragraph blockquote rule ──────────────────────────────────────

    const MULTI_PARA: &str = "\
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
> must identify the boot device and create a loop device.
>
> This must happen early in the installer's startup.
";

    const IN_CONTEXT: &str = "\
# Live ISO

r[iso.simple]
A simple rule before.

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
> must identify the boot device and create a loop device.
>
> This must happen early in the installer's startup.

r[iso.after]
A simple rule after.
";

    #[test]
    fn test_pipeline_multi_para_rule_count() {
        let blocks = pipeline(MULTI_PARA);
        assert_eq!(
            blocks.len(),
            1,
            "expected 1 Rule block, got {}:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_pipeline_multi_para_rule_kind() {
        let blocks = pipeline(MULTI_PARA);
        assert!(
            matches!(&blocks[0].kind, SpecBlockKind::Rule { .. }),
            "{:?}",
            blocks[0].kind
        );
    }

    #[test]
    fn test_pipeline_multi_para_rule_text_has_all_paras() {
        let blocks = pipeline(MULTI_PARA);
        match &blocks[0].kind {
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

    // r[verify view.render]
    #[test]
    fn test_pipeline_in_context_block_count() {
        let blocks = pipeline(IN_CONTEXT);
        println!("\n=== pipeline blocks ({}) ===", blocks.len());
        for (i, b) in blocks.iter().enumerate() {
            match &b.kind {
                SpecBlockKind::Heading { level, text, .. } => {
                    println!("  [{i}] Heading({level}, {text:?})")
                }
                SpecBlockKind::Rule { id, text, .. } => {
                    println!("  [{i}] Rule({id:?}, {} lines)", text.lines().count())
                }
                SpecBlockKind::Paragraph { text } => {
                    println!("  [{i}] Paragraph({:?})", &text[..text.len().min(60)])
                }
            }
        }
        println!("=== end ===\n");
        // Heading + iso.simple + iso.cdrom-partscan+4 + iso.after
        assert_eq!(
            blocks.len(),
            4,
            "expected 4 blocks, got {}:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_pipeline_in_context_cdrom_rule_text() {
        let blocks = pipeline(IN_CONTEXT);
        let rule = blocks.iter().find(
            |b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "iso.cdrom-partscan+4")
        ).expect("cdrom rule not found");
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

    // ── round-trip: build → markdown → same structure ────────────────────────

    /// Simulate the full round-trip that the read view performs:
    /// build_doc_from_specs → doc_to_markdown_files → parse_blocks_from_content.
    /// Multi-paragraph rules must survive this round-trip intact.
    fn round_trip_pipeline(content: &str) -> Vec<crate::components::spec_block_editor::SpecBlock> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = rt.block_on(build_doc_from_specs(&specs));

        // This is what list_rendered_specs and doc_to_markdown_files do.
        let markdown_files = doc_to_markdown_files(&doc);
        assert_eq!(markdown_files.len(), 1, "expected one file in round-trip");

        let (_, serialised_md) = &markdown_files[0];
        println!("\n=== round-trip serialised Markdown ===\n{serialised_md}\n=== end ===\n");

        rt.block_on(crate::components::spec_block_editor::parse_blocks_from_content(serialised_md))
    }

    // r[verify repo.multi-file]
    #[test]
    fn test_round_trip_single_para_rule_preserved() {
        let blocks = round_trip_pipeline("r[foo.bar]\nThe rule text.\n");
        assert_eq!(blocks.len(), 1, "{blocks:#?}");
        match &blocks[0].kind {
            SpecBlockKind::Rule { id, text, .. } => {
                assert_eq!(id, "foo.bar");
                assert_eq!(text.trim(), "The rule text.");
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // r[verify repo.multi-file]
    #[test]
    fn test_round_trip_multi_para_rule_count() {
        let blocks = round_trip_pipeline(IN_CONTEXT);
        println!("\n=== round-trip blocks ({}) ===", blocks.len());
        for (i, b) in blocks.iter().enumerate() {
            match &b.kind {
                SpecBlockKind::Heading { level, text, .. } => {
                    println!("  [{i}] Heading({level}, {text:?})")
                }
                SpecBlockKind::Rule { id, text, .. } => {
                    println!("  [{i}] Rule({id:?}, {} lines)", text.lines().count())
                }
                SpecBlockKind::Paragraph { text } => {
                    println!("  [{i}] Paragraph({:?})", &text[..text.len().min(60)])
                }
            }
        }
        println!("=== end ===\n");
        assert_eq!(
            blocks.len(),
            4,
            "round-trip: expected Heading + 3 Rules, got {}:\n{blocks:#?}",
            blocks.len()
        );
    }

    #[test]
    fn test_round_trip_multi_para_rule_text() {
        let blocks = round_trip_pipeline(IN_CONTEXT);
        let rule = blocks.iter().find(
            |b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "iso.cdrom-partscan+4"),
        ).expect("iso.cdrom-partscan+4 not found after round-trip");
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

    // r[verify ids.stable-on-reorder]
    #[test]
    fn test_pipeline_round_trip_rule_ids_preserved() {
        let blocks = pipeline(IN_CONTEXT);
        let ids: Vec<&str> = blocks
            .iter()
            .filter_map(|b| {
                if let SpecBlockKind::Rule { id, .. } = &b.kind {
                    Some(id.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            ids,
            vec!["iso.simple", "iso.cdrom-partscan+4", "iso.after"],
            "{ids:?}"
        );
    }
    // r[verify ids.stable-on-reorder]
    // r[verify edit.reorder]
    #[test]
    fn test_reorder_preserves_rule_ids() {
        let content = "# Section\n\nr[a.first]\nFirst rule.\n\nr[a.second]\nSecond rule.\n";
        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = run(build_doc_from_specs(&specs));

        // Collect the initial rule IDs and their tree node IDs.
        let blocks_before = loro_doc_to_blocks(&doc);
        let rule_ids_before: Vec<String> = blocks_before
            .iter()
            .filter_map(|b| {
                if let SpecBlockKind::Rule { id, .. } = &b.kind {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(rule_ids_before, vec!["a.first", "a.second"]);

        // Reorder: move the second rule before the first by finding their tree IDs
        // and using tree.mov_to.
        let tree = doc.get_tree(TREE_NAME);
        let second_block = blocks_before
            .iter()
            .find(|b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "a.second"))
            .unwrap();
        let first_block = blocks_before
            .iter()
            .find(|b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "a.first"))
            .unwrap();

        let second_id = key_to_tree_id(&second_block.key).unwrap();
        let first_id = key_to_tree_id(&first_block.key).unwrap();
        let parent = tree.parent(first_id).unwrap();
        // Move second before first (index 0 under parent).
        let siblings = tree.children(parent).unwrap_or_default();
        let first_idx = siblings.iter().position(|s| *s == first_id).unwrap();
        tree.mov_to(second_id, parent, first_idx).ok();

        // Rule IDs must be unchanged after reorder.
        let blocks_after = loro_doc_to_blocks(&doc);
        let rule_ids_after: Vec<String> = blocks_after
            .iter()
            .filter_map(|b| {
                if let SpecBlockKind::Rule { id, .. } = &b.kind {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        // Order flipped, but IDs preserved.
        assert_eq!(rule_ids_after, vec!["a.second", "a.first"]);
    }

    // r[verify edit.add-rule]
    #[test]
    fn test_add_rule_preserves_existing_ids() {
        let content = "# Section\n\nr[a.first]\nFirst rule.\n";
        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = run(build_doc_from_specs(&specs));

        let blocks_before = loro_doc_to_blocks(&doc);
        assert_eq!(blocks_before.len(), 2); // heading + rule

        // Add a new rule after the existing one.
        let tree = doc.get_tree(TREE_NAME);
        let last_block = &blocks_before[1];
        let last_id = key_to_tree_id(&last_block.key).unwrap();
        let parent = tree.parent(last_id).unwrap();
        let siblings = tree.children(parent).unwrap_or_default();
        let idx = siblings
            .iter()
            .position(|s| *s == last_id)
            .map(|i| i + 1)
            .unwrap();
        let new_node = tree.create_at(parent, idx).unwrap();
        let meta = tree.get_meta(new_node).unwrap();
        meta.insert("kind", "rule").unwrap();
        meta.insert("rule_id", "a.new-rule").unwrap();
        set_text_content(&meta, "New rule text.");

        let blocks_after = loro_doc_to_blocks(&doc);
        assert_eq!(blocks_after.len(), 3); // heading + 2 rules

        // Original rule ID preserved.
        let first_rule = blocks_after
            .iter()
            .find(|b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "a.first"));
        assert!(first_rule.is_some(), "original rule ID must still exist");

        // New rule present.
        let new_rule = blocks_after
            .iter()
            .find(|b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "a.new-rule"));
        assert!(new_rule.is_some(), "new rule must be present");
    }

    // r[verify edit.delete]
    #[test]
    fn test_delete_rule() {
        let content = "# Section\n\nr[a.first]\nFirst rule.\n\nr[a.second]\nSecond rule.\n";
        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = run(build_doc_from_specs(&specs));

        let blocks = loro_doc_to_blocks(&doc);
        let second_block = blocks
            .iter()
            .find(|b| matches!(&b.kind, SpecBlockKind::Rule { id, .. } if id == "a.second"))
            .unwrap();

        let tree = doc.get_tree(TREE_NAME);
        let second_id = key_to_tree_id(&second_block.key).unwrap();
        tree.delete(second_id).ok();

        let blocks_after = loro_doc_to_blocks(&doc);
        let rule_ids: Vec<&str> = blocks_after
            .iter()
            .filter_map(|b| {
                if let SpecBlockKind::Rule { id, .. } = &b.kind {
                    Some(id.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(rule_ids, vec!["a.first"]);
    }

    // r[verify edit.rule-text]
    #[test]
    fn test_edit_text_preserves_rule_id() {
        let content = "r[my.rule]\nOriginal text.\n";
        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = run(build_doc_from_specs(&specs));

        let blocks = loro_doc_to_blocks(&doc);
        assert_eq!(blocks.len(), 1);
        let rule_key = &blocks[0].key;
        let node_id = key_to_tree_id(rule_key).unwrap();

        // Modify the text via set_text_content.
        let tree = doc.get_tree(TREE_NAME);
        let meta = tree.get_meta(node_id).unwrap();
        set_text_content(&meta, "Updated text.");

        let blocks_after = loro_doc_to_blocks(&doc);
        assert_eq!(blocks_after.len(), 1);
        match &blocks_after[0].kind {
            SpecBlockKind::Rule { id, text, .. } => {
                assert_eq!(id, "my.rule", "rule ID must be preserved after text edit");
                assert_eq!(text, "Updated text.");
            }
            other => panic!("expected Rule, got {other:?}"),
        }
    }

    // r[verify edit.add-section]
    #[test]
    fn test_add_heading() {
        let content = "# Existing\n\nr[a.rule]\nSome rule.\n";
        let specs = vec![(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )];
        let doc = run(build_doc_from_specs(&specs));

        let tree = doc.get_tree(TREE_NAME);
        let blocks = loro_doc_to_blocks(&doc);
        let last = &blocks[blocks.len() - 1];
        let last_id = key_to_tree_id(&last.key).unwrap();
        let parent = tree.parent(last_id).unwrap();
        let siblings = tree.children(parent).unwrap_or_default();
        let idx = siblings.len();

        let new_node = tree.create_at(parent, idx).unwrap();
        let meta = tree.get_meta(new_node).unwrap();
        meta.insert("kind", "heading").unwrap();
        meta.insert("level", 2i64).unwrap();
        set_text_content(&meta, "New Section");

        let blocks_after = loro_doc_to_blocks(&doc);
        let headings: Vec<&str> = blocks_after
            .iter()
            .filter_map(|b| {
                if let SpecBlockKind::Heading { text, .. } = &b.kind {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(headings, vec!["Existing", "New Section"]);
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_rename_rule_ids() {
        let content = "# Section\n\n> r[new.aabb0011]\n> A provisional rule\n\n> r[existing.rule]\n> An existing rule\n";
        let doc = run(build_doc_from_specs(&[(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )]));

        // Find the provisional rule's key.
        let blocks = loro_doc_to_blocks(&doc);
        let prov_key = blocks
            .iter()
            .find_map(|b| match &b.kind {
                SpecBlockKind::Rule { id, .. } if id == "new.aabb0011" => Some(b.key.clone()),
                _ => None,
            })
            .expect("provisional rule should exist");

        assert!(has_provisional_ids(&doc));

        rename_rule_ids(&doc, &[(prov_key, "section.my-new-rule".to_string())]).unwrap();

        assert!(!has_provisional_ids(&doc));

        // Verify the markdown output uses the new ID.
        let files = doc_to_markdown_files(&doc);
        let (_, md) = &files[0];
        assert!(
            md.contains("r[section.my-new-rule]"),
            "markdown should contain renamed ID, got: {md}"
        );
        assert!(
            !md.contains("r[new.aabb0011]"),
            "markdown should not contain old provisional ID"
        );
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_has_provisional_ids_false_when_none() {
        let content = "# Section\n\n> r[repo.connect]\n> Connect a repo\n";
        let doc = run(build_doc_from_specs(&[(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )]));
        assert!(!has_provisional_ids(&doc));
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_rename_preserves_other_rules() {
        let content =
            "# Section\n\n> r[new.deadbeef]\n> New rule\n\n> r[keep.this]\n> Existing rule\n";
        let doc = run(build_doc_from_specs(&[(
            "test-spec".to_string(),
            vec![("test.md".to_string(), content.to_string())],
        )]));

        let blocks = loro_doc_to_blocks(&doc);
        let prov_key = blocks
            .iter()
            .find_map(|b| match &b.kind {
                SpecBlockKind::Rule { id, .. } if id.starts_with("new.") => Some(b.key.clone()),
                _ => None,
            })
            .unwrap();

        rename_rule_ids(&doc, &[(prov_key, "section.replaced".to_string())]).unwrap();

        let files = doc_to_markdown_files(&doc);
        let (_, md) = &files[0];
        assert!(md.contains("r[section.replaced]"));
        assert!(
            md.contains("r[keep.this]"),
            "existing rule ID should be preserved"
        );
    }
}
