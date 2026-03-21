use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// Sentinel used as the drop_key for the insert bar above all blocks.
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

// ── Block operations ──────────────────────────────────────────────────────────

/// An individual editing operation. The client queues these locally and ships
/// them to the server in batches; the server applies them in order to the
/// latest snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BlockOp {
    EditText { key: String, text: String },
    DeleteBlock { key: String },
    MoveBlock { key: String, after_key: String },
    InsertBlock { after_key: String, block: SpecBlock },
}

/// Apply a slice of ops to a block list in order.
/// Used by the server inside `apply_block_ops` and available for unit tests.
pub fn apply_ops_to_blocks(blocks: &mut Vec<SpecBlock>, ops: &[BlockOp]) {
    for op in ops {
        match op {
            BlockOp::EditText { key, text } => {
                if let Some(b) = blocks.iter_mut().find(|b| b.key == *key) {
                    match &mut b.kind {
                        SpecBlockKind::Heading {
                            text: t, anchor: a, ..
                        } => {
                            *t = text.clone();
                            *a = slugify(text);
                        }
                        SpecBlockKind::Rule { text: t, .. } => *t = text.clone(),
                        SpecBlockKind::Paragraph { text: t } => *t = text.clone(),
                    }
                    b.html.clear();
                }
            }
            BlockOp::DeleteBlock { key } => {
                blocks.retain(|b| b.key != *key);
            }
            BlockOp::MoveBlock { key, after_key } => {
                let Some(fi) = blocks.iter().position(|b| b.key == *key) else {
                    continue;
                };
                let item = blocks.remove(fi);
                let pos = if after_key == TOP_DROP_KEY {
                    0
                } else {
                    blocks
                        .iter()
                        .position(|b| b.key == *after_key)
                        .map(|i| i + 1)
                        .unwrap_or(blocks.len())
                };
                blocks.insert(pos, item);
            }
            BlockOp::InsertBlock { after_key, block } => {
                let pos = if after_key == TOP_DROP_KEY {
                    0
                } else {
                    blocks
                        .iter()
                        .position(|b| b.key == *after_key)
                        .map(|i| i + 1)
                        .unwrap_or(blocks.len())
                };
                blocks.insert(pos, block.clone());
            }
        }
    }
}

/// Push `new_op` onto the queue with coalescing:
///
/// - `EditText` for a key: update an existing `InsertBlock` or `EditText` for
///   that key in place — never send stale intermediate states.
/// - `DeleteBlock` for a key: purge all ops that touch that key. If one of
///   those was an `InsertBlock`, skip the delete too (the block never reached
///   the server).
/// - `MoveBlock` for a key: replace any prior `MoveBlock` for the same key.
/// - `InsertBlock`: always append (each new block has a unique key).
#[cfg(feature = "hydrate")]
fn push_op(ops: &mut Vec<BlockOp>, new_op: BlockOp) {
    match &new_op {
        BlockOp::EditText { key, text } => {
            for op in ops.iter_mut() {
                match op {
                    BlockOp::InsertBlock { block, .. } if block.key == *key => {
                        match &mut block.kind {
                            SpecBlockKind::Heading {
                                text: t, anchor: a, ..
                            } => {
                                *t = text.clone();
                                *a = slugify(text);
                            }
                            SpecBlockKind::Rule { text: t, .. } => *t = text.clone(),
                            SpecBlockKind::Paragraph { text: t } => *t = text.clone(),
                        }
                        return;
                    }
                    BlockOp::EditText { key: k, text: t } if k == key => {
                        *t = text.clone();
                        return;
                    }
                    _ => {}
                }
            }
            ops.push(new_op);
        }
        BlockOp::DeleteBlock { key } => {
            let had_pending_insert = ops
                .iter()
                .any(|op| matches!(op, BlockOp::InsertBlock { block, .. } if block.key == *key));
            ops.retain(|op| match op {
                BlockOp::EditText { key: k, .. } => k != key,
                BlockOp::InsertBlock { block, .. } => &block.key != key,
                BlockOp::MoveBlock { key: k, .. } => k != key,
                BlockOp::DeleteBlock { key: k } => k != key,
            });
            if !had_pending_insert {
                ops.push(new_op);
            }
        }
        BlockOp::MoveBlock { key, .. } => {
            ops.retain(|op| !matches!(op, BlockOp::MoveBlock { key: k, .. } if k == key));
            ops.push(new_op);
        }
        BlockOp::InsertBlock { .. } => {
            ops.push(new_op);
        }
    }
}

#[cfg(feature = "hydrate")]
fn load_queue(proposal_id: i32) -> Vec<BlockOp> {
    (|| -> Option<Vec<BlockOp>> {
        let storage = web_sys::window()?.local_storage().ok()??;
        let json = storage
            .get_item(&format!("mcbean_ops_{proposal_id}"))
            .ok()??;
        serde_json::from_str(&json).ok()
    })()
    .unwrap_or_default()
}

#[cfg(feature = "hydrate")]
fn persist_queue(proposal_id: i32, ops: &[BlockOp]) {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return;
    };
    let key = format!("mcbean_ops_{proposal_id}");
    if ops.is_empty() {
        let _ = storage.remove_item(&key);
    } else if let Ok(json) = serde_json::to_string(ops) {
        let _ = storage.set_item(&key, &json);
    }
}

/// Flush pending ops to the server with exponential backoff on failure.
/// Loops until the queue is empty or the retry limit is hit.
/// Backoff: 2^retry seconds, capped at 5 minutes. Maximum 8 retries.
#[cfg(feature = "hydrate")]
async fn do_flush(
    proposal_id: i32,
    pending_ops: RwSignal<Vec<BlockOp>>,
    flush_running: RwSignal<bool>,
    retry_count: RwSignal<u32>,
    save_error: RwSignal<Option<String>>,
) {
    use gloo_timers::future::TimeoutFuture;
    const MAX_RETRIES: u32 = 8;

    loop {
        let retry = retry_count.get_untracked();
        if retry > 0 {
            let delay_ms = (1_000u32 << retry.min(8)).min(300_000);
            TimeoutFuture::new(delay_ms).await;
        }

        let ops = pending_ops.get_untracked();
        if ops.is_empty() {
            break;
        }
        let n = ops.len();

        match apply_block_ops(proposal_id, ops).await {
            Ok(()) => {
                pending_ops.update(|q| {
                    q.drain(..n);
                });
                retry_count.set(0);
                save_error.set(None);
                if pending_ops.get_untracked().is_empty() {
                    break;
                }
                // More ops arrived while we were sending; loop immediately.
            }
            Err(e) => {
                let next = retry + 1;
                retry_count.set(next);
                if next >= MAX_RETRIES {
                    save_error.set(Some(format!(
                        "Failed to save after {MAX_RETRIES} attempts: {e}"
                    )));
                    break;
                }
                // Loop to apply backoff delay then retry.
            }
        }
    }

    flush_running.set(false);
}

/// Enqueue an op on the hydrate path; no-op during SSR.
#[cfg(feature = "hydrate")]
fn enqueue(pending_ops: RwSignal<Vec<BlockOp>>, op: BlockOp) {
    pending_ops.update(|ops| push_op(ops, op));
}

#[cfg(not(feature = "hydrate"))]
fn enqueue(_pending_ops: RwSignal<Vec<BlockOp>>, _op: BlockOp) {}

// r[impl edit.history]
#[server(input = server_fn::codec::Json)]
pub async fn apply_block_ops(proposal_id: i32, ops: Vec<BlockOp>) -> Result<(), ServerFnError> {
    use diesel::prelude::*;

    if ops.is_empty() {
        return Ok(());
    }

    let user_id = crate::auth::get_or_create_user_id().await?;
    let pool =
        use_context::<crate::db::DbPool>().ok_or_else(|| ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| ServerFnError::new(format!("{e}")))?;

    let (repository_id, latest_snapshot): (i32, Option<String>) = conn
        .interact(move |conn| {
            use crate::db::schema::{proposal_changes, proposals};

            let repo_id: i32 = proposals::table
                .find(proposal_id)
                .select(proposals::repository_id)
                .first(conn)?;

            let snapshot: Option<String> = proposal_changes::table
                .filter(proposal_changes::proposal_id.eq(proposal_id))
                .order(proposal_changes::id.desc())
                .select(proposal_changes::content_snapshot)
                .first(conn)
                .optional()?;

            Ok::<_, diesel::result::Error>((repo_id, snapshot))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?;

    let content = if let Some(s) = latest_snapshot {
        s
    } else {
        conn.interact(move |conn| {
            use crate::db::schema::{spec_files, specs};

            let contents: Vec<String> = spec_files::table
                .inner_join(specs::table)
                .filter(specs::repository_id.eq(repository_id))
                .order((specs::name.asc(), spec_files::path.asc()))
                .select(spec_files::content)
                .load(conn)?;

            Ok::<_, diesel::result::Error>(contents.join("\n\n"))
        })
        .await
        .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
        .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))?
    };

    let mut blocks = parse_blocks_from_content(&content).await;
    apply_ops_to_blocks(&mut blocks, &ops);
    let new_content = serialize_blocks(&blocks);

    conn.interact(move |conn| {
        use crate::db::schema::proposal_changes;

        let parent_id: Option<i32> = proposal_changes::table
            .filter(proposal_changes::proposal_id.eq(proposal_id))
            .order(proposal_changes::id.desc())
            .select(proposal_changes::id)
            .first(conn)
            .optional()?;

        diesel::insert_into(proposal_changes::table)
            .values((
                proposal_changes::proposal_id.eq(proposal_id),
                proposal_changes::parent_change_id.eq(parent_id),
                proposal_changes::user_id.eq(user_id),
                proposal_changes::change_type.eq(crate::db::models::ChangeType::UserEdit),
                proposal_changes::content_snapshot.eq(&new_content),
            ))
            .execute(conn)?;

        Ok(())
    })
    .await
    .map_err(|e| ServerFnError::new(format!("interact: {e}")))?
    .map_err(|e: diesel::result::Error| ServerFnError::new(format!("query: {e}")))
}

// ── Sidebar data ──────────────────────────────────────────────────────────────

/// Build sidebar outline and search entries from a block list.
/// The proposal is treated as a single spec named after `spec_name`.
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

    let outline = vec![SpecOutline {
        name: spec_name.to_string(),
        headings,
    }];

    (outline, search_entries)
}

// ── Helpers available on both SSR and WASM ────────────────────────────────────

pub async fn parse_blocks_from_content(content: &str) -> Vec<SpecBlock> {
    use marq::{DocElement, RenderOptions, render};

    let doc = match render(content, &RenderOptions::new()).await {
        Ok(d) => d,
        Err(_e) => {
            #[cfg(feature = "ssr")]
            tracing::warn!(error = %_e, "marq render failed in parse_blocks_from_content");
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
                let html = format!(
                    "<h{level} id=\"{id}\">{text}</h{level}>",
                    level = h.level,
                    id = h.id,
                    text = html_escape(&h.title),
                );
                blocks.push(SpecBlock {
                    key,
                    kind: SpecBlockKind::Heading {
                        level: h.level,
                        text: h.title.clone(),
                        anchor: h.id.clone(),
                    },
                    html,
                });
            }

            DocElement::Req(r) => {
                let prose = join_hard_wraps(strip_blockquote_prefixes(&r.raw).trim());
                blocks.push(SpecBlock {
                    key,
                    kind: SpecBlockKind::Rule {
                        id: r.id.to_string(),
                        text: prose,
                    },
                    html: r.html.clone(),
                });
            }

            DocElement::Paragraph(p) => {
                let start = p.offset.min(content.len());
                let rest = &content[start..];
                let end = rest.find("\n\n").unwrap_or(rest.len());
                let text = join_hard_wraps(rest[..end].trim());
                if text.is_empty() || text.starts_with("r[") {
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

    blocks
}

/// Serialize an ordered list of blocks back to Tracey-flavoured markdown.
#[cfg(feature = "ssr")]
pub fn serialize_blocks(blocks: &[SpecBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        match &block.kind {
            SpecBlockKind::Heading { level, text, .. } => {
                for _ in 0..*level {
                    out.push('#');
                }
                out.push(' ');
                out.push_str(text.trim());
                out.push_str("\n\n");
            }
            SpecBlockKind::Rule { id, text } => {
                // r[impl edit.rule-text]
                out.push_str("r[");
                out.push_str(id);
                out.push_str("]\n");
                out.push_str(&reflow_sentences(text));
                out.push_str("\n\n");
            }
            SpecBlockKind::Paragraph { text } => {
                out.push_str(&reflow_sentences(text));
                out.push_str("\n\n");
            }
        }
    }
    out
}

/// Collapse single newlines (hard wraps) into spaces so the textarea shows
/// soft-wrapped prose.
fn join_hard_wraps(text: &str) -> String {
    text.lines().collect::<Vec<_>>().join(" ")
}

/// Convert heading text to a URL-safe anchor id, matching marq's output.
/// Lowercases, maps spaces and punctuation to hyphens, collapses runs.
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

/// Reflow prose so each sentence starts on its own line. First joins any
/// remaining hard wraps, then inserts a newline after every `.`, `?`, or `!`
/// that is followed by a space and then an uppercase letter (classic sentence
/// boundary), or that ends the string.
#[cfg(feature = "ssr")]
fn reflow_sentences(text: &str) -> String {
    let flat = join_hard_wraps(text.trim());
    let chars: Vec<char> = flat.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(flat.len());
    let mut i = 0;
    while i < len {
        let c = chars[i];
        out.push(c);
        if matches!(c, '.' | '?' | '!') {
            let mut j = i + 1;
            while j < len && chars[j] == ' ' {
                j += 1;
            }
            if j == len || chars[j].is_uppercase() {
                if j < len {
                    out.push('\n');
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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

// ── Key / ID generation ───────────────────────────────────────────────────────

static BLOCK_SEQ: AtomicU64 = AtomicU64::new(1);

pub fn next_block_key() -> String {
    let n = BLOCK_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("b{n}")
}

/// Generate a provisional rule ID per r[ids.provisional].
pub fn next_provisional_id() -> String {
    // r[impl ids.provisional]
    #[cfg(feature = "hydrate")]
    let n = (js_sys::Math::random() * (u32::MAX as f64 + 1.0)) as u32;
    #[cfg(not(feature = "hydrate"))]
    let n = rand::random::<u32>();
    format!("{n:08x}+0")
}

// ── Component ─────────────────────────────────────────────────────────────────

#[component]
pub fn SpecBlockEditor(blocks_signal: RwSignal<Vec<SpecBlock>>, proposal_id: i32) -> impl IntoView {
    let blocks = blocks_signal;

    // Key of the block whose textarea is currently open.
    let editing_key: RwSignal<Option<String>> = RwSignal::new(None);
    // Live content of the open textarea.
    let edit_draft = RwSignal::new(String::new());

    // HTML5 drag-and-drop state.
    let drag_key: RwSignal<Option<String>> = RwSignal::new(None);
    let drag_over_bar: RwSignal<Option<String>> = RwSignal::new(None);

    // ── Save queue ────────────────────────────────────────────────────────────

    // Load any ops that survived a previous session from localStorage.
    let pending_ops: RwSignal<Vec<BlockOp>> = RwSignal::new({
        #[cfg(feature = "hydrate")]
        {
            load_queue(proposal_id)
        }
        #[cfg(not(feature = "hydrate"))]
        {
            Vec::new()
        }
    });
    let flush_running = RwSignal::new(false);
    let retry_count = RwSignal::new(0u32);
    let save_error: RwSignal<Option<String>> = RwSignal::new(None);

    // Monotonically increasing counter; the in-flight debounce task compares
    // its captured generation against this to detect supersession.
    let debounce_gen = RwSignal::new(0u32);

    // Both proposal_id and debounce_gen are only referenced inside
    // #[cfg(feature = "hydrate")] blocks; suppress the SSR-build warnings.
    #[cfg(not(feature = "hydrate"))]
    let _ = (proposal_id, debounce_gen);

    // Sync queue to localStorage on every change so offline edits survive a
    // page reload.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let ops = pending_ops.get();
        persist_queue(proposal_id, &ops);
    });

    // Spawn a flush task whenever ops are waiting and no flush is running.
    // save_error is a tracked dep so that clearing it (retry button) re-arms
    // the trigger without needing a new edit.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let ops = pending_ops.get();
        let error = save_error.get();
        if !ops.is_empty() && error.is_none() && !flush_running.get_untracked() {
            flush_running.set(true);
            leptos::task::spawn_local(async move {
                do_flush(
                    proposal_id,
                    pending_ops,
                    flush_running,
                    retry_count,
                    save_error,
                )
                .await;
            });
        }
    });

    // Register / clear the beforeunload guard so the browser warns when the
    // user tries to close the tab while changes have not reached the server.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        use wasm_bindgen::JsCast as _;
        use wasm_bindgen::prelude::Closure;

        let has_pending = !pending_ops.get().is_empty();
        let Some(window) = web_sys::window() else {
            return;
        };
        if has_pending {
            let cb = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(
                |ev: web_sys::BeforeUnloadEvent| {
                    ev.prevent_default();
                    ev.set_return_value("Changes not yet saved to server.");
                },
            );
            window.set_onbeforeunload(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        } else {
            window.set_onbeforeunload(None);
        }
    });

    // ── Editing helpers ───────────────────────────────────────────────────────

    let open_edit = move |key: String, text: String| {
        edit_draft.set(text);
        editing_key.set(Some(key));
    };

    // Commits the current textarea draft: cancels the debounce timer, updates
    // the local block signal for display, enqueues an EditText op, closes the
    // editor, and triggers a client-side HTML re-render. Called on blur.
    let commit_edit = move || {
        let Some(key) = editing_key.get_untracked() else {
            return;
        };

        // Cancel any in-flight debounce so it doesn't double-queue.
        #[cfg(feature = "hydrate")]
        debounce_gen.update(|g| *g = g.wrapping_add(1));

        editing_key.set(None);

        let text = edit_draft.get_untracked();

        let changed = blocks.with_untracked(|list| {
            list.iter()
                .find(|b| b.key == key)
                .is_some_and(|b| b.edit_text() != text)
        });

        if !changed {
            return;
        }

        blocks.update(|list| {
            if let Some(b) = list.iter_mut().find(|b| b.key == key) {
                match &mut b.kind {
                    SpecBlockKind::Heading {
                        text: t, anchor: a, ..
                    } => {
                        *t = text.clone();
                        *a = slugify(&text);
                    }
                    SpecBlockKind::Rule { text: t, .. } => *t = text.clone(),
                    SpecBlockKind::Paragraph { text: t } => *t = text.clone(),
                }
                b.html.clear();
            }
        });

        enqueue(
            pending_ops,
            BlockOp::EditText {
                key: key.clone(),
                text: text.clone(),
            },
        );

        // Re-render the edited block's HTML in the browser without a server
        // round-trip. blocks already has the updated text from above.
        #[cfg(feature = "hydrate")]
        {
            use marq::{RenderOptions, render};

            let raw = blocks.with_untracked(|list| {
                list.iter().find(|b| b.key == key).map(|b| match &b.kind {
                    SpecBlockKind::Rule { id, text } => {
                        format!("r[{}]\n{}\n\n", id, text)
                    }
                    SpecBlockKind::Heading { level, text, .. } => {
                        format!("{} {}\n\n", "#".repeat(*level as usize), text)
                    }
                    SpecBlockKind::Paragraph { text } => format!("{}\n\n", text),
                })
            });

            if let Some(raw) = raw {
                leptos::task::spawn_local(async move {
                    if let Ok(doc) = render(&raw, &RenderOptions::new()).await {
                        let new_html = if let Some(req) = doc.reqs.first() {
                            req.html.clone()
                        } else {
                            doc.html.clone()
                        };
                        blocks.update(|list| {
                            if let Some(b) = list.iter_mut().find(|b| b.key == key) {
                                b.html = new_html;
                            }
                        });
                    }
                });
            }
        }
    };

    // Closes the editor without saving; also cancels any pending debounce.
    let revert_edit = move || {
        #[cfg(feature = "hydrate")]
        debounce_gen.update(|g| *g = g.wrapping_add(1));
        editing_key.set(None);
    };

    // r[impl edit.delete]
    let delete_block = move |key: String| {
        // Cancel the debounce if we're deleting the block currently being
        // edited so the timer doesn't later queue a stale EditText for a
        // block that no longer exists on the server.
        #[cfg(feature = "hydrate")]
        if editing_key.get_untracked().as_deref() == Some(&key) {
            debounce_gen.update(|g| *g = g.wrapping_add(1));
        }
        if editing_key.get_untracked().as_deref() == Some(&key) {
            editing_key.set(None);
        }
        blocks.update(|list| list.retain(|b| b.key != key));
        enqueue(pending_ops, BlockOp::DeleteBlock { key });
    };

    // r[impl edit.reorder]
    let handle_drop = move |from_key: String, drop_key: String| {
        if from_key == drop_key {
            drag_key.set(None);
            drag_over_bar.set(None);
            return;
        }
        blocks.update(|list| {
            let Some(fi) = list.iter().position(|b| b.key == from_key) else {
                return;
            };
            let item = list.remove(fi);
            let pos = if drop_key == TOP_DROP_KEY {
                0
            } else {
                list.iter()
                    .position(|b| b.key == drop_key)
                    .map(|i| i + 1)
                    .unwrap_or(list.len())
            };
            list.insert(pos, item);
        });
        drag_key.set(None);
        drag_over_bar.set(None);
        enqueue(
            pending_ops,
            BlockOp::MoveBlock {
                key: from_key,
                after_key: drop_key,
            },
        );
    };

    // r[impl edit.add-rule]
    // r[impl edit.add-section]
    let insert_block = move |after_key: String, kind: SpecBlockKind| {
        let new_key = next_block_key();
        let new_block = SpecBlock {
            key: new_key.clone(),
            kind,
            html: String::new(),
        };
        let block_for_queue = new_block.clone();
        blocks.update(|list| {
            let pos = if after_key == TOP_DROP_KEY {
                0
            } else {
                list.iter()
                    .position(|b| b.key == after_key)
                    .map(|i| i + 1)
                    .unwrap_or(list.len())
            };
            list.insert(pos, new_block);
        });
        enqueue(
            pending_ops,
            BlockOp::InsertBlock {
                after_key,
                block: block_for_queue,
            },
        );
        open_edit(new_key, String::new());
    };

    view! {
        <div class="spec-block-editor">

            // ── Save status indicator ─────────────────────────────────────────
            {move || {
                let error = save_error.get();
                let pending = !pending_ops.get().is_empty();
                let flushing = flush_running.get();
                if let Some(err) = error {
                    view! {
                        <div class="notification is-danger is-light mb-3">
                            <strong>"Save error: "</strong>
                            {err}
                            <button
                                class="button is-small is-danger is-outlined ml-3"
                                on:click=move |_| {
                                    save_error.set(None);
                                    retry_count.set(0);
                                }
                            >
                                "Retry"
                            </button>
                        </div>
                    }
                    .into_any()
                } else if pending || flushing {
                    view! { <p class="help is-info mb-2">"Saving\u{2026}"</p> }.into_any()
                } else {
                    view! { <span /> }.into_any()
                }
            }}

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
                        SpecBlockKind::Heading { level: lvl, text: String::new(), anchor: String::new() },
                    )
                })
            />

            <For
                each=move || blocks.get()
                key=|b| b.key.clone()
                children=move |block| {
                    let key = StoredValue::new(block.key.clone());

                    // These are stable for the lifetime of an existing block.
                    let rule_id: StoredValue<Option<String>> =
                        StoredValue::new(if let SpecBlockKind::Rule { id, .. } = &block.kind {
                            Some(id.clone())
                        } else {
                            None
                        });
                    let heading_level: StoredValue<Option<u8>> =
                        StoredValue::new(
                            if let SpecBlockKind::Heading { level, .. } = &block.kind {
                                Some(*level)
                            } else {
                                None
                            },
                        );
                    let heading_anchor: StoredValue<Option<String>> =
                        StoredValue::new(
                            if let SpecBlockKind::Heading { anchor, .. } = &block.kind {
                                Some(anchor.clone())
                            } else {
                                None
                            },
                        );

                    // Reactive anchor for this block — recomputed from the block
                    // signal so it stays current after the user edits the heading.
                    let block_anchor = move || {
                        if heading_anchor.get_value().is_none() {
                            return String::new();
                        }
                        blocks.with(|list| {
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

                    // Reactive reads of this block's current text and HTML from
                    // the shared signal, so the display stays accurate after edits.
                    let block_html = move || {
                        blocks.with(|list| {
                            list.iter()
                                .find(|b| b.key == key.get_value())
                                .map(|b| b.html.clone())
                                .unwrap_or_default()
                        })
                    };
                    let block_text = move || {
                        blocks.with(|list| {
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

                    // Font styling for the textarea so headings fill roughly the
                    // same vertical space as their rendered counterpart.
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
                            // Allow dragging over the block body without showing
                            // the "forbidden" cursor — the InsertBars are the targets.
                            on:dragover=move |e| e.prevent_default()
                        >
                            // ── Header row ───────────────────────────────────
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

                            // ── Block body ────────────────────────────────────
                            // Show either the textarea (editing) or the rendered
                            // display (reading). Clicking anywhere on the display
                            // opens the textarea for this block.
                            <Show
                                when=is_editing
                                fallback=move || {
                                    let html = block_html();
                                    let text = block_text();
                                    let is_empty = html.is_empty() && text.is_empty();
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
                                            } else if !text.is_empty() {
                                                view! { <span>{text}</span> }.into_any()
                                            } else {
                                                view! {
                                                    <span class="spec-block-placeholder">
                                                        "Click to add content…"
                                                    </span>
                                                }
                                                .into_any()
                                            }}
                                        </div>
                                    }
                                }
                            >
                                // r[impl edit.rule-text]
                                // grow-wrap uses the CSS grid shadow-twin trick so the textarea
                                // auto-sizes in browsers without field-sizing:content (Firefox).
                                // The mirror div drives the container height; the textarea
                                // occupies the same grid cell. Font metrics are shared so both
                                // size identically. Trailing space prevents last-line collapse.
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

                                            // Debounce: push an EditText op 1.5 s after the
                                            // last keystroke while the textarea is still open,
                                            // so long editing sessions are saved incrementally
                                            // without requiring the user to blur. Blur still
                                            // pushes a final op when the editor closes.
                                            #[cfg(feature = "hydrate")]
                                            {
                                                use gloo_timers::future::TimeoutFuture;

                                                let dgen = debounce_gen
                                                    .get_untracked()
                                                    .wrapping_add(1);
                                                debounce_gen.set(dgen);
                                                let key_snap = editing_key.get_untracked();
                                                leptos::task::spawn_local(async move {
                                                    TimeoutFuture::new(1_500).await;
                                                    if debounce_gen.get_untracked() != dgen {
                                                        return;
                                                    }
                                                    let Some(key) = key_snap else { return };
                                                    let text = edit_draft.get_untracked();
                                                    let changed =
                                                        blocks.with_untracked(|list| {
                                                            list.iter()
                                                                .find(|b| b.key == key)
                                                                .is_some_and(|b| {
                                                                    b.edit_text() != text
                                                                })
                                                        });
                                                    if !changed {
                                                        return;
                                                    }
                                                    blocks.update(|list| {
                                                        if let Some(b) = list
                                                            .iter_mut()
                                                            .find(|b| b.key == key)
                                                        {
                                                            match &mut b.kind {
                                                                SpecBlockKind::Heading {
                                                                    text: t,
                                                                    anchor: a,
                                                                    ..
                                                                } => {
                                                                    *t = text.clone();
                                                                    *a = slugify(&text);
                                                                }
                                                                SpecBlockKind::Rule {
                                                                    text: t,
                                                                    ..
                                                                } => *t = text.clone(),
                                                                SpecBlockKind::Paragraph {
                                                                    text: t,
                                                                } => *t = text.clone(),
                                                            }
                                                            b.html.clear();
                                                        }
                                                    });
                                                    pending_ops.update(|ops| {
                                                        push_op(
                                                            ops,
                                                            BlockOp::EditText { key, text },
                                                        )
                                                    });
                                                });
                                            }
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

                            // ── Insert bar below this block ───────────────────
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

/// A thin strip rendered between every pair of blocks (and above the first block).
///
/// During drag-and-drop it becomes the drop target: the strip expands and glows
/// blue when the dragged block is hovering over it, giving an unambiguous
/// insertion-point indicator. The "+" menu for manual insertion is hidden during
/// drag so it doesn't interfere.
#[component]
fn InsertBar(
    /// Position identifier. `TOP_DROP_KEY` means "before all blocks"; a block key
    /// means "after that block".
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

    // Close the insert menu when a drag starts so it doesn't persist into drag mode.
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
            // The visible line — invisible at rest, a coloured bar during drag.
            <div class="insert-bar-line" />

            // Manual insert controls — hidden while a drag is in progress.
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
