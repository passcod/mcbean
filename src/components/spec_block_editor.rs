use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

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
                    text = html_escape_attr(&h.title),
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
                // r.raw is the content after the r[id] marker line, possibly
                // with `> ` blockquote prefixes.
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
                    html: format!("<p>{}</p>", html_escape_attr(&text)),
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
fn html_escape_attr(s: &str) -> String {
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

// ── Key generation ────────────────────────────────────────────────────────────

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

/// Structured WYSIWYG-style editor for a list of spec blocks.
///
/// `on_save` is called with the full updated block list when the user
/// confirms their changes. The parent is responsible for serialising and
/// persisting the result.
#[component]
pub fn SpecBlockEditor(blocks: Vec<SpecBlock>, on_save: Callback<Vec<SpecBlock>>) -> impl IntoView {
    let blocks = RwSignal::new(blocks);

    // Key of the block currently open in the textarea editor.
    let editing_key: RwSignal<Option<String>> = RwSignal::new(None);
    // Live text in the textarea.
    let edit_draft = RwSignal::new(String::new());

    // HTML5 drag-and-drop state.
    let drag_key: RwSignal<Option<String>> = RwSignal::new(None);
    let drag_over_key: RwSignal<Option<String>> = RwSignal::new(None);

    // Commit the textarea text back into the block list and close the editor.
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
                // Invalidate stale HTML; display will fall back to plain text
                // until the parent re-renders after a full save.
                b.html.clear();
            }
        });
        editing_key.set(None);
    };

    let cancel_edit = move || editing_key.set(None);

    let open_edit = move |key: String, text: String| {
        edit_draft.set(text);
        editing_key.set(Some(key));
    };

    // r[impl edit.delete]
    let delete_block = move |key: String| {
        if editing_key.get_untracked().as_deref() == Some(&key) {
            editing_key.set(None);
        }
        blocks.update(|list| list.retain(|b| b.key != key));
    };

    // Insert a new rule immediately after `after_key`, or at the end.
    // r[impl edit.add-rule]
    let insert_rule_after = move |after_key: Option<String>| {
        let new_key = next_block_key();
        let new_block = SpecBlock {
            key: new_key.clone(),
            // r[impl ids.provisional]
            kind: SpecBlockKind::Rule {
                id: next_provisional_id(),
                text: String::new(),
            },
            html: String::new(),
        };
        blocks.update(|list| {
            let pos = insertion_pos(list, after_key.as_deref());
            list.insert(pos, new_block);
        });
        open_edit(new_key, String::new());
    };

    // r[impl edit.add-section]
    let insert_heading_after = move |after_key: Option<String>, level: u8| {
        let new_key = next_block_key();
        let new_block = SpecBlock {
            key: new_key.clone(),
            kind: SpecBlockKind::Heading {
                level,
                text: String::new(),
            },
            html: String::new(),
        };
        blocks.update(|list| {
            let pos = insertion_pos(list, after_key.as_deref());
            list.insert(pos, new_block);
        });
        open_edit(new_key, String::new());
    };

    view! {
        <div class="spec-block-editor">
            // Insert bar at the very top of the list.
            <InsertBar
                on_insert_rule=Callback::new(move |_| insert_rule_after(None))
                on_insert_heading=Callback::new(move |level| insert_heading_after(None, level))
            />

            <For
                each=move || blocks.get()
                key=|b| b.key.clone()
                children=move |block| {
                    // Stable, Copy handles for per-block identity and initial values.
                    let key = StoredValue::new(block.key.clone());
                    let init_text = StoredValue::new(block.edit_text().to_owned());
                    let init_html = StoredValue::new(block.html.clone());

                    // Per-block metadata (fixed for the lifetime of this render).
                    let rule_id: StoredValue<Option<String>> = StoredValue::new(
                        if let SpecBlockKind::Rule { id, .. } = &block.kind {
                            Some(id.clone())
                        } else {
                            None
                        },
                    );
                    let heading_level: Option<u8> =
                        if let SpecBlockKind::Heading { level, .. } = &block.kind {
                            Some(*level)
                        } else {
                            None
                        };

                    let placeholder = rule_id.with_value(|r| {
                        if r.is_some() {
                            "Rule prose…"
                        } else if heading_level.is_some() {
                            "Heading text…"
                        } else {
                            "Paragraph text…"
                        }
                    });

                    // Reactive predicates for this block's state.
                    let is_editing =
                        move || editing_key.get().as_deref() == Some(&key.get_value());
                    let is_drag_over =
                        move || drag_over_key.get().as_deref() == Some(&key.get_value());
                    let is_dragging =
                        move || drag_key.get().as_deref() == Some(&key.get_value());

                    view! {
                        <div
                            class="spec-block-wrapper"
                            class:spec-block--drag-over=is_drag_over
                            class:spec-block--dragging=is_dragging
                            draggable="true"
                            on:dragstart=move |e| {
                                e.stop_propagation();
                                drag_key.set(Some(key.get_value()));
                            }
                            on:dragend=move |_| {
                                drag_key.set(None);
                                drag_over_key.set(None);
                            }
                            on:dragover=move |e| {
                                e.prevent_default();
                                drag_over_key.set(Some(key.get_value()));
                            }
                            on:dragleave=move |_| {
                                drag_over_key.update(|k| {
                                    if k.as_deref() == Some(&key.get_value()) {
                                        *k = None;
                                    }
                                });
                            }
                            on:drop=move |e| {
                                e.prevent_default();
                                drag_over_key.set(None);
                                let to = key.get_value();
                                if let Some(from) = drag_key.get_untracked() {
                                    drag_key.set(None);
                                    if from != to {
                                        // r[impl edit.reorder]
                                        blocks.update(|list| {
                                            if let Some(fi) =
                                                list.iter().position(|b| b.key == from)
                                            {
                                                let item = list.remove(fi);
                                                // Recalculate index after the removal.
                                                let ti = list
                                                    .iter()
                                                    .position(|b| b.key == to)
                                                    .unwrap_or(list.len());
                                                list.insert(ti, item);
                                            }
                                        });
                                    }
                                }
                            }
                        >
                            // ── Block header ──────────────────────────────────
                            <div class="spec-block-header">
                                <span
                                    class="spec-block-drag-handle"
                                    title="Drag to reorder"
                                >
                                    "⠿"
                                </span>

                                // Rule ID badge / heading level / paragraph marker.
                                {move || {
                                    rule_id.with_value(|rid| {
                                        if let Some(id) = rid {
                                            view! {
                                                <span class="spec-block-badge spec-block-badge--rule">
                                                    {format!("r[{}]", id)}
                                                </span>
                                            }
                                            .into_any()
                                        } else if let Some(lvl) = heading_level {
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

                                <div class="spec-block-actions">
                                    <Show when=move || !is_editing()>
                                        <button
                                            class="spec-action-btn"
                                            title="Edit this block"
                                            on:click=move |_| {
                                                open_edit(
                                                    key.get_value(),
                                                    init_text.get_value(),
                                                );
                                            }
                                        >
                                            "Edit"
                                        </button>
                                    </Show>
                                    // r[impl edit.delete]
                                    <button
                                        class="spec-action-btn spec-action-btn--delete"
                                        title="Delete this block"
                                        on:click=move |_| delete_block(key.get_value())
                                    >
                                        "✕"
                                    </button>
                                </div>
                            </div>

                            // ── Block body ────────────────────────────────────
                            <Show
                                when=is_editing
                                fallback=move || {
                                    let html = init_html.get_value();
                                    let text = init_text.get_value();
                                    let html_empty = html.is_empty();
                                    view! {
                                        <div
                                            class="spec-block-display"
                                            class:spec-block-display--unsaved=html_empty
                                        >
                                            {if html_empty {
                                                view! {
                                                    <span class="spec-block-unsaved-text">
                                                        {text}
                                                    </span>
                                                }
                                                .into_any()
                                            } else {
                                                view! {
                                                    <div class="content" inner_html=html />
                                                }
                                                .into_any()
                                            }}
                                        </div>
                                    }
                                }
                            >
                                // r[impl edit.rule-text]
                                <div class="spec-block-edit">
                                    <textarea
                                        class="textarea spec-block-textarea"
                                        placeholder=placeholder
                                        prop:value=move || edit_draft.get()
                                        rows=5
                                        on:input=move |ev| {
                                            edit_draft.set(event_target_value(&ev));
                                        }
                                        on:keydown=move |ev| {
                                            let key_name = ev.key();
                                            if key_name == "Escape" {
                                                cancel_edit();
                                            } else if key_name == "Enter"
                                                && (ev.ctrl_key() || ev.meta_key())
                                            {
                                                commit_edit();
                                            }
                                        }
                                    />
                                    <div class="spec-block-edit-btns">
                                        <button
                                            class="button is-small is-success"
                                            on:click=move |_| commit_edit()
                                        >
                                            "Done"
                                        </button>
                                        <button
                                            class="button is-small is-light"
                                            on:click=move |_| cancel_edit()
                                        >
                                            "Cancel"
                                        </button>
                                        <span class="spec-block-edit-hint">
                                            "Ctrl+Enter to save, Esc to cancel"
                                        </span>
                                    </div>
                                </div>
                            </Show>

                            // ── Insert bar below each block ───────────────────
                            // r[impl edit.add-rule]
                            // r[impl edit.add-section]
                            <InsertBar
                                on_insert_rule=Callback::new(move |_| {
                                    insert_rule_after(Some(key.get_value()))
                                })
                                on_insert_heading=Callback::new(move |level| {
                                    insert_heading_after(Some(key.get_value()), level)
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

// ── Insert bar ────────────────────────────────────────────────────────────────

#[component]
fn InsertBar(on_insert_rule: Callback<()>, on_insert_heading: Callback<u8>) -> impl IntoView {
    let open = RwSignal::new(false);

    view! {
        <div class="insert-bar">
            <button
                class="insert-bar-toggle"
                class:is-active=move || open.get()
                title="Insert a block here"
                on:click=move |_| open.update(|v| *v = !*v)
            >
                "+"
            </button>

            <Show when=move || open.get()>
                <div class="insert-bar-menu">
                    // r[impl edit.add-rule]
                    <button
                        class="insert-bar-option"
                        on:click=move |_| {
                            open.set(false);
                            on_insert_rule.run(());
                        }
                    >
                        "Rule"
                    </button>
                    // r[impl edit.add-section]
                    <button
                        class="insert-bar-option"
                        on:click=move |_| {
                            open.set(false);
                            on_insert_heading.run(2);
                        }
                    >
                        "Section (H2)"
                    </button>
                    <button
                        class="insert-bar-option"
                        on:click=move |_| {
                            open.set(false);
                            on_insert_heading.run(3);
                        }
                    >
                        "Subsection (H3)"
                    </button>
                    <button
                        class="insert-bar-option"
                        on:click=move |_| {
                            open.set(false);
                            on_insert_heading.run(4);
                        }
                    >
                        "Sub-subsection (H4)"
                    </button>
                </div>
            </Show>
        </div>
    }
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn insertion_pos(list: &[SpecBlock], after_key: Option<&str>) -> usize {
    after_key
        .and_then(|k| list.iter().position(|b| b.key == k))
        .map(|i| i + 1)
        .unwrap_or(list.len())
}
