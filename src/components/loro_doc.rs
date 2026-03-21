use loro::{
    Container, LoroDoc, LoroMap, LoroText, LoroTree, LoroValue, PeerID, TreeID, TreeParentId,
    ValueOrContainer, VersionVector,
};

use crate::components::spec_block_editor::{SpecBlock, SpecBlockKind, slugify};

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
                // Generate HTML that matches the .req/.req-link CSS structure.
                // commit_edit will replace this with a full marq-rendered version
                // after the user edits the block; the background render pass in
                // SpecBlockEditor replaces it with marq HTML shortly after load.
                let html = format!(
                    "<div class=\"req\">\
                     <a class=\"req-link\" id=\"{id}\" href=\"#{id}\">\
                     <span>r[{id}]</span></a>\
                     <p>{text}</p>\
                     </div>",
                    id = html_escape(&rule_id),
                    text = html_escape(&text),
                );
                out.push(SpecBlock {
                    key: tree_id_to_key(node_id),
                    kind: SpecBlockKind::Rule { id: rule_id, text },
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

    let doc = LoroDoc::new();
    doc.set_peer_id(SERVER_PEER_ID).expect("set server peer id");
    let tree = doc.get_tree(TREE_NAME);

    for (spec_name, files) in specs {
        let spec_node = tree.create(TreeParentId::Root).expect("create spec node");
        let spec_meta = tree.get_meta(spec_node).expect("spec meta");
        spec_meta.insert("kind", KIND_SPEC).unwrap();
        spec_meta.insert("name", spec_name.as_str()).unwrap();

        let mut sorted_files: Vec<&(String, String)> = files.iter().collect();
        sorted_files.sort_by(|a, b| a.0.cmp(&b.0));

        for (file_path, file_content) in sorted_files {
            let file_node = tree
                .create(TreeParentId::Node(spec_node))
                .expect("create file node");
            let file_meta = tree.get_meta(file_node).expect("file meta");
            file_meta.insert("kind", KIND_FILE).unwrap();
            file_meta.insert("path", file_path.as_str()).unwrap();

            let blocks = parse_blocks_from_content(file_content).await;
            insert_flat_blocks(&tree, file_node, &blocks);
        }
    }

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
    let mut stack: Vec<(u8, TreeID)> = vec![(0, file_node)];

    for block in blocks {
        match &block.kind {
            SpecBlockKind::Heading { level, text, .. } => {
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
                set_text_content(&meta, text);
                stack.push((*level, node));
            }
            SpecBlockKind::Rule { id, text } => {
                let parent = stack.last().map(|(_, id)| *id).unwrap_or(file_node);
                let node = tree
                    .create(TreeParentId::Node(parent))
                    .expect("create rule node");
                let meta = tree.get_meta(node).expect("rule meta");
                meta.insert("kind", KIND_RULE).unwrap();
                meta.insert("rule_id", id.as_str()).unwrap();
                set_text_content(&meta, text);
            }
            SpecBlockKind::Paragraph { text } => {
                let parent = stack.last().map(|(_, id)| *id).unwrap_or(file_node);
                let node = tree
                    .create(TreeParentId::Node(parent))
                    .expect("create para node");
                let meta = tree.get_meta(node).expect("para meta");
                meta.insert("kind", KIND_PARA).unwrap();
                set_text_content(&meta, text);
            }
        }
    }
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
                out.push_str("r[");
                out.push_str(&rule_id);
                out.push_str("]\n");
                out.push_str(text.trim());
                out.push_str("\n\n");
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
    match meta.get("text") {
        Some(ValueOrContainer::Container(Container::Text(t))) => {
            let len = t.len_unicode();
            if len > 0 {
                t.delete(0, len).ok();
            }
            if !content.is_empty() {
                t.insert(0, content).ok();
            }
        }
        _ => {
            let t = meta
                .insert_container("text", LoroText::new())
                .expect("insert text container");
            if !content.is_empty() {
                t.insert(0, content).ok();
            }
        }
    }
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
