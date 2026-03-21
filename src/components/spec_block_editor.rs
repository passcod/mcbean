use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

// Sentinel used as the drop_key for the insert bar above all blocks.
const TOP_DROP_KEY: &str = "^^top";

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecBlockKind {
    Heading { level: u8, text: String },
    Rule { id: String, text: String },
    Paragraph { text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpecBlock {
    /// Stable identity key — used for Leptos list keying only, not persisted.
    pub key: String,
    pub kind: SpecBlockKind,
    /// Pre-rendered HTML for display mode. Empty for newly created blocks.
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

// ── SSR-only helpers ──────────────────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub async fn parse_blocks_from_content(content: &str) -> Vec<SpecBlock> {
    use marq::{DocElement, RenderOptions, render};

    let doc = match render(content, &RenderOptions::new()).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "marq render failed in parse_blocks_from_content");
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
                    },
                    html,
                });
            }

            DocElement::Req(r) => {
                let prose = strip_blockquote_prefixes(&r.raw).trim().to_string();
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
                let text = rest[..end].trim().to_string();
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
            SpecBlockKind::Heading { level, text } => {
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
                out.push_str(text.trim());
                out.push_str("\n\n");
            }
            SpecBlockKind::Paragraph { text } => {
                out.push_str(text.trim());
                out.push_str("\n\n");
            }
        }
    }
    out
}

#[cfg(feature = "ssr")]
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(feature = "ssr")]
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
    let n = BLOCK_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{:08x}+0", n)
}

// ── Component ─────────────────────────────────────────────────────────────────

#[component]
pub fn SpecBlockEditor(blocks: Vec<SpecBlock>, on_save: Callback<Vec<SpecBlock>>) -> impl IntoView {
    let blocks = RwSignal::new(blocks);

    // Key of the block whose textarea is currently open.
    let editing_key: RwSignal<Option<String>> = RwSignal::new(None);
    // Live content of the open textarea.
    let edit_draft = RwSignal::new(String::new());

    // HTML5 drag-and-drop: which block is being dragged, which insert bar is the drop target.
    let drag_key: RwSignal<Option<String>> = RwSignal::new(None);
    let drag_over_bar: RwSignal<Option<String>> = RwSignal::new(None);

    // Opens the textarea for a block, seeding the draft from its current text.
    let open_edit = move |key: String, text: String| {
        edit_draft.set(text);
        editing_key.set(Some(key));
    };

    // Writes the textarea draft back into the block list and closes the editor.
    // Called on textarea blur — no explicit "Done" button needed.
    let commit_edit = move || {
        let Some(key) = editing_key.get_untracked() else {
            return;
        };
        let text = edit_draft.get_untracked();
        blocks.update(|list| {
            if let Some(b) = list.iter_mut().find(|b| b.key == key) {
                match &mut b.kind {
                    SpecBlockKind::Heading { text: t, .. } => *t = text.clone(),
                    SpecBlockKind::Rule { text: t, .. } => *t = text.clone(),
                    SpecBlockKind::Paragraph { text: t } => *t = text.clone(),
                }
                // Clear stale rendered HTML; display falls back to plain text
                // until the user saves and the server re-renders.
                b.html.clear();
            }
        });
        editing_key.set(None);
    };

    // Closes the editor without writing the draft, reverting the visible text.
    let revert_edit = move || editing_key.set(None);

    // r[impl edit.delete]
    let delete_block = move |key: String| {
        if editing_key.get_untracked().as_deref() == Some(&key) {
            editing_key.set(None);
        }
        blocks.update(|list| list.retain(|b| b.key != key));
    };

    // r[impl edit.reorder]
    // Moves the dragged block to the position indicated by drop_key.
    // drop_key is either TOP_DROP_KEY (insert at position 0) or a block key
    // meaning "insert after this block".
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
    };

    // Inserts a new block at the position given by after_key (same convention as drop_key).
    // r[impl edit.add-rule]
    // r[impl edit.add-section]
    let insert_block = move |after_key: String, kind: SpecBlockKind| {
        let new_key = next_block_key();
        let new_block = SpecBlock {
            key: new_key.clone(),
            kind,
            html: String::new(),
        };
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
        open_edit(new_key, String::new());
    };

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
                        SpecBlockKind::Heading { level: lvl, text: String::new() },
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

                    // Reactive read of this block's current text and HTML from
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

                    // Font styling for the textarea — headings get a matching size/weight
                    // so the textarea fills approximately the same vertical space.
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
                            // Allow dragging over the block body without showing
                            // the "forbidden" cursor — the InsertBars are the real targets.
                            on:dragover=move |e| e.prevent_default()
                        >
                            // ── Header row ───────────────────────────────────
                            <div class="spec-block-header">
                                // Only the handle is draggable; clicking the body opens the editor.
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

                                // Type badge.
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
                                    title="Delete block"
                                    on:click=move |_| delete_block(key.get_value())
                                >
                                    "✕"
                                </button>
                            </div>

                            // ── Block body ────────────────────────────────────
                            // Show either the textarea (editing) or the rendered display (reading).
                            // Clicking anywhere on the display opens the textarea for this block.
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
                                // The ::after pseudo-element mirrors the content via
                                // data-replicated-value and occupies the same grid cell, driving
                                // the container height. field-sizing:content is kept as a
                                // @supports enhancement for browsers that have it.
                                // Font metrics live on the wrapper so both the textarea and the
                                // ::after inherit identical sizing.
                                <div
                                    class="grow-wrap"
                                    style:font-size=ta_font_size
                                    style:font-weight=ta_font_weight
                                >
                                    // Mirror div: sits in the same grid cell as the textarea
                                    // and drives the row height through its text content.
                                    // Wired directly to edit_draft — no attr()/::after needed,
                                    // so this works in Firefox and every other browser.
                                    // Trailing space prevents the last line collapsing when
                                    // the content ends with a newline.
                                    <div
                                        class="grow-wrap-mirror"
                                        aria-hidden="true"
                                    >
                                        {move || format!("{} ", edit_draft.get())}
                                    </div>
                                    <textarea
                                        class="spec-block-textarea"
                                        autofocus=true
                                        prop:value=move || edit_draft.get()
                                        on:input=move |ev| edit_draft.set(event_target_value(&ev))
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
                                        SpecBlockKind::Heading { level: lvl, text: String::new() },
                                    )
                                })
                            />
                        </div>
                    }
                }
            />

            // ── Footer ────────────────────────────────────────────────────────
            <div class="spec-editor-footer">
                <button
                    class="button is-primary"
                    on:click=move |_| on_save.run(blocks.get_untracked())
                >
                    "Save changes"
                </button>
            </div>
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
                        title="Insert a block here"
                        on:click=move |_| menu_open.update(|v| *v = !*v)
                    >
                        "+"
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
                                    on_insert_heading.run(2);
                                }
                            >
                                "Section (H2)"
                            </button>
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_heading.run(3);
                                }
                            >
                                "Subsection (H3)"
                            </button>
                            <button
                                class="insert-bar-option"
                                on:click=move |_| {
                                    menu_open.set(false);
                                    on_insert_heading.run(4);
                                }
                            >
                                "Sub-subsection (H4)"
                            </button>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    }
}
