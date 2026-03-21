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
                                format!("r[{}]\n{}\n\n", id, text)
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
                        }
                        let blocks = crate::components::loro_doc::loro_doc_to_blocks(doc);
                        blocks_out.set(blocks);
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

    // Re-derive the flat block list from the Loro doc and push it out.
    // Must be called after every mutation to keep the UI in sync.
    #[cfg(feature = "hydrate")]
    let refresh_blocks = move || {
        loro_doc.with_value(|doc| {
            let blocks = crate::components::loro_doc::loro_doc_to_blocks(doc);
            blocks_out.set(blocks);
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

                let raw = blocks_out.with_untracked(|list| {
                    list.iter().find(|b| b.key == key).map(|b| match &b.kind {
                        SpecBlockKind::Rule { id, text } => {
                            format!("r[{}]\n{}\n\n", id, text)
                        }
                        SpecBlockKind::Heading { level, text, .. } => {
                            format!("{} {}\n\n", "#".repeat(*level as usize), text)
                        }
                        SpecBlockKind::Paragraph { text } => {
                            format!("{}\n\n", text)
                        }
                    })
                });

                if let Some(raw) = raw {
                    leptos::task::spawn_local(async move {
                        if let Ok(doc) = render(&raw, &RenderOptions::new()).await {
                            let new_html =
                                doc.reqs.first().map(|r| r.html.clone()).unwrap_or(doc.html);
                            blocks_out.update(|list| {
                                if let Some(b) = list.iter_mut().find(|b| b.key == key) {
                                    b.html = new_html;
                                }
                            });
                        }
                    });
                }
            }

            // Push the updated text into blocks_out (HTML will be patched above).
            loro_doc.with_value(|doc| {
                let blocks = loro_doc_to_blocks(doc);
                blocks_out.set(blocks);
            });

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
            refresh_blocks();
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

            refresh_blocks();
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

            refresh_blocks();
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
