use leptos::prelude::*;

use crate::components::changelog_sidebar::{
    ChangeKind, ChangelogEntry, compute_changelog, word_diff,
};
use crate::components::spec_block_editor::{SpecBlock, SpecBlockKind};

/// Returns true if the rule ID looks provisional (assigned by next_provisional_id).
pub fn is_provisional_id(id: &str) -> bool {
    id.starts_with("new.")
}

/// Collect all rule blocks that still carry a provisional ID.
fn provisional_rules(blocks: &[SpecBlock]) -> Vec<(String, String)> {
    blocks
        .iter()
        .filter_map(|b| match &b.kind {
            SpecBlockKind::Rule { id, .. } if is_provisional_id(id) => {
                Some((b.key.clone(), id.clone()))
            }
            _ => None,
        })
        .collect()
}

/// For a given tree key, return the rule ID if the block is a rule.
fn rule_id_for_key<'a>(blocks: &'a [SpecBlock], key: &str) -> Option<&'a str> {
    blocks
        .iter()
        .find(|b| b.key == key)
        .and_then(|b| match &b.kind {
            SpecBlockKind::Rule { id, .. } => Some(id.as_str()),
            _ => None,
        })
}

// r[impl lifecycle.finalising]
// r[impl lifecycle.finalising.ids]
// r[impl ids.persist-overrides]
#[component]
pub fn FinalisingView(
    /// Current proposal blocks (with any edits applied).
    blocks: Vec<SpecBlock>,
    /// Base snapshot blocks for changelog comparison.
    base_blocks: Vec<SpecBlock>,
    /// Called when the user wants to go back to drafting.
    on_back: Callback<()>,
    /// Called when the user confirms submission. ID renames have already been
    /// persisted as Loro CRDT operations via `on_id_change`.
    on_submit: Callback<()>,
    /// Called when the user changes a provisional rule ID. The tuple is
    /// `(tree_key, new_id)`. The parent is responsible for dispatching this
    /// to the `set_rule_id` server function so the rename is recorded as a
    /// Loro CRDT operation.
    on_id_change: Callback<(String, String)>,
    /// Whether submission is currently in progress.
    #[prop(into)]
    submitting: Signal<bool>,
    /// Optional error message from a failed submission attempt.
    #[prop(into)]
    submit_error: Signal<Option<String>>,
) -> impl IntoView {
    let provisional_keys: std::collections::HashSet<String> = provisional_rules(&blocks)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let has_provisionals = !provisional_keys.is_empty();

    let changelog = compute_changelog(&base_blocks, &blocks);

    // Build per-rule editable signals for every rule that appears in the changelog.
    // Provisional rules start empty (user must fill them in); final rules start
    // with their current ID (user may optionally change them).
    let rule_overrides: Vec<(String, RwSignal<String>)> = changelog
        .iter()
        .filter_map(|entry| {
            rule_id_for_key(&blocks, &entry.key).map(|current_id| {
                let initial = if provisional_keys.contains(&entry.key) {
                    String::new()
                } else {
                    current_id.to_owned()
                };
                (entry.key.clone(), RwSignal::new(initial))
            })
        })
        .collect();
    let rule_overrides = StoredValue::new(rule_overrides);

    // Submission is blocked while any provisional rule still has an empty input.
    let provisional_keys_for_memo = provisional_keys.clone();
    let all_resolved = Memo::new(move |_| {
        if !has_provisionals {
            return true;
        }
        rule_overrides.with_value(|overrides| {
            overrides
                .iter()
                .filter(|(key, _)| provisional_keys_for_memo.contains(key))
                .all(|(_, sig)| !sig.get().trim().is_empty())
        })
    });

    view! {
        <div class="box" style="margin-bottom: 1.5rem;">
            <h2 class="title is-4 mb-3">"Review & Submit"</h2>
            <p class="subtitle is-6 has-text-grey">
                "Review the changes below. All provisional rule IDs must be replaced before submission."
            </p>

            // Show submission error if any.
            {move || {
                submit_error.get().map(|e| {
                    view! {
                        <div class="notification is-danger is-light mb-4">
                            {format!("Submission failed: {e}")}
                        </div>
                    }
                })
            }}

            // Provisional ID warning (no inputs — those are inline in the changelog now).
            {if has_provisionals {
                view! {
                    <div class="notification is-warning is-light mb-5">
                        <p>
                            <strong>"Some rules still have provisional IDs."</strong>
                            " Fill in replacements below before submitting."
                        </p>
                    </div>
                }
                .into_any()
            } else {
                ().into_any()
            }}

            // Changelog with inline-editable rule IDs.
            <h3 class="title is-5 mb-3">"Changes"</h3>
            {if changelog.is_empty() {
                view! {
                    <p class="has-text-grey">"No changes detected."</p>
                }
                .into_any()
            } else {
                view! {
                    <div>
                        {changelog
                            .into_iter()
                            .map(|entry| {
                                let key = entry.key.clone();
                                let override_sig = rule_overrides.with_value(|overrides| {
                                    overrides.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
                                });
                                let is_provisional = provisional_keys.contains(&key);
                                view! {
                                    <ChangelogRow
                                        entry=entry
                                        rule_id_signal=override_sig
                                        is_provisional=is_provisional
                                        on_id_change=on_id_change
                                    />
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                }
                .into_any()
            }}

            // Action buttons.
            <div class="buttons mt-5">
                <button
                    class="button is-light"
                    on:click=move |_| on_back.run(())
                    disabled=move || submitting.get()
                >
                    "Back to Editing"
                </button>
                <button
                    class="button is-success"
                    disabled=move || !all_resolved.get() || submitting.get()
                    on:click=move |_| on_submit.run(())
                >
                    {move || {
                        if submitting.get() {
                            "Submitting…"
                        } else {
                            "Submit Proposal"
                        }
                    }}
                </button>
            </div>
        </div>
    }
}

/// Commit the current value of an override signal via the callback.
fn commit_id_override(sig: RwSignal<String>, key: &str, on_id_change: Callback<(String, String)>) {
    let trimmed = sig.get_untracked().trim().to_string();
    if !trimmed.is_empty() {
        on_id_change.run((key.to_owned(), trimmed));
    }
}

#[component]
fn ChangelogRow(
    entry: ChangelogEntry,
    /// If this entry is a rule, the editable signal holding the (possibly overridden) ID.
    rule_id_signal: Option<RwSignal<String>>,
    /// Whether the rule's current ID is provisional (must be replaced).
    is_provisional: bool,
    /// Callback to persist a rule ID rename.
    on_id_change: Callback<(String, String)>,
) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let has_diff = entry.old_text.is_some() || entry.new_text.is_some();

    let (kind_class, kind_label) = match entry.kind {
        ChangeKind::Added => ("has-text-success", "Added"),
        ChangeKind::Deleted => ("has-text-danger", "Deleted"),
        ChangeKind::Modified => ("has-text-info", "Modified"),
        ChangeKind::Reordered => ("has-text-grey", "Reordered"),
        ChangeKind::VersionBump => ("has-text-grey-light", "Version bump"),
    };

    let old = entry.old_text.clone();
    let new = entry.new_text.clone();
    let entry_key = entry.key.clone();

    // Build the label portion: either an editable ID input or a plain text label.
    let label_view = match rule_id_signal {
        Some(sig) => {
            let key_for_blur = entry_key.clone();
            let key_for_enter = entry_key.clone();
            let input_class = if is_provisional {
                "input is-small is-warning"
            } else {
                "input is-small"
            };
            view! {
                <span
                    class="is-inline-flex is-align-items-center"
                    style="gap: 0.35rem;"
                    on:click=move |ev| ev.stop_propagation()
                >
                    <span class="has-text-grey-light" style="font-size: 0.85em;">
                        {if is_provisional { "new rule →" } else { "ID:" }}
                    </span>
                    <input
                        class=input_class
                        type="text"
                        placeholder="e.g. security.auth-tokens"
                        style="width: 16em;"
                        prop:value=move || sig.get()
                        on:input=move |ev| sig.set(event_target_value(&ev))
                        on:blur={
                            let key = key_for_blur;
                            move |_| commit_id_override(sig, &key, on_id_change)
                        }
                        on:keydown={
                            let key = key_for_enter;
                            move |ev: leptos::ev::KeyboardEvent| {
                                if ev.key() == "Enter" {
                                    ev.prevent_default();
                                    commit_id_override(sig, &key, on_id_change);
                                }
                            }
                        }
                    />
                </span>
            }
            .into_any()
        }
        None => view! { <span>{entry.label.clone()}</span> }.into_any(),
    };

    view! {
        <div
            class="box py-3 px-4 mb-2"
            style="cursor: pointer;"
            on:click=move |_| {
                if has_diff {
                    expanded.update(|v| *v = !*v);
                }
            }
        >
            <div class="is-flex is-align-items-center is-justify-content-space-between">
                <div class="is-flex is-align-items-center" style="gap: 0.5rem;">
                    <span class=kind_class style="font-weight: 600;">
                        {kind_label}
                    </span>
                    {label_view}
                </div>
                {if has_diff {
                    view! {
                        <span class="icon is-small has-text-grey">
                            {move || if expanded.get() { "▼" } else { "▶" }}
                        </span>
                    }
                    .into_any()
                } else {
                    ().into_any()
                }}
            </div>
            <Show when=move || expanded.get()>
                <div class="mt-3" style="font-size: 0.9rem;">
                    {match (&old, &new) {
                        (Some(o), Some(n)) => {
                            let (old_spans, new_spans) = word_diff(o, n);
                            view! {
                                <div style="font-family: monospace; font-size: 0.85em;">
                                    <div style="background: #fef2f2; padding: 0.5rem 0.75rem; border-radius: 4px 4px 0 0; white-space: pre-wrap;">
                                        <span style="color: #b91c1c; font-weight: 600; margin-right: 0.4rem;">"−"</span>
                                        {old_spans
                                            .into_iter()
                                            .map(|(is_common, text)| {
                                                if !is_common {
                                                    view! {
                                                        <span style="background: #fecaca; text-decoration: line-through; padding: 0 2px;">
                                                            {text}
                                                        </span>
                                                        " "
                                                    }
                                                    .into_any()
                                                } else {
                                                    view! { <span>{text}" "</span> }.into_any()
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </div>
                                    <div style="background: #f0fdf4; padding: 0.5rem 0.75rem; border-radius: 0 0 4px 4px; white-space: pre-wrap;">
                                        <span style="color: #15803d; font-weight: 600; margin-right: 0.4rem;">"+"</span>
                                        {new_spans
                                            .into_iter()
                                            .map(|(is_common, text)| {
                                                if !is_common {
                                                    view! {
                                                        <span style="background: #bbf7d0; padding: 0 2px;">
                                                            {text}
                                                        </span>
                                                        " "
                                                    }
                                                    .into_any()
                                                } else {
                                                    view! { <span>{text}" "</span> }.into_any()
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </div>
                                </div>
                            }
                            .into_any()
                        }
                        (None, Some(n)) => {
                            view! {
                                <div style="background: #ecfdf5; padding: 0.75rem; border-radius: 4px; white-space: pre-wrap;">
                                    {n.clone()}
                                </div>
                            }
                            .into_any()
                        }
                        (Some(o), None) => {
                            view! {
                                <div style="background: #fef2f2; padding: 0.75rem; border-radius: 4px; white-space: pre-wrap; text-decoration: line-through;">
                                    {o.clone()}
                                </div>
                            }
                            .into_any()
                        }
                        _ => ().into_any(),
                    }}
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::spec_block_editor::html_escape;

    fn rule_block(key: &str, id: &str, text: &str) -> SpecBlock {
        SpecBlock {
            key: key.to_string(),
            kind: SpecBlockKind::Rule {
                prefix: "r".to_string(),
                id: id.to_string(),
                text: text.to_string(),
            },
            html: format!("<p>{}</p>", html_escape(text)),
        }
    }

    fn heading_block(key: &str, level: u8, text: &str) -> SpecBlock {
        SpecBlock {
            key: key.to_string(),
            kind: SpecBlockKind::Heading {
                level,
                text: text.to_string(),
                anchor: text.to_lowercase().replace(' ', "-"),
            },
            html: format!("<h{level}>{}</h{level}>", html_escape(text)),
        }
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_provisional_id_detected() {
        assert!(is_provisional_id("new.abcd1234+1"));
        assert!(is_provisional_id("new.00000000+1"));
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_final_id_not_provisional() {
        assert!(!is_provisional_id("security.auth-tokens"));
        assert!(!is_provisional_id("repo.connect+2"));
        assert!(!is_provisional_id(""));
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_provisional_rules_collects_only_provisional() {
        let blocks = vec![
            heading_block("1:0", 1, "Section"),
            rule_block("1:1", "new.aabbccdd+1", "A new rule"),
            rule_block("1:2", "repo.connect", "An existing rule"),
            rule_block("1:3", "new.11223344+1", "Another new rule"),
        ];
        let provs = provisional_rules(&blocks);
        assert_eq!(provs.len(), 2);
        assert_eq!(provs[0].1, "new.aabbccdd+1");
        assert_eq!(provs[1].1, "new.11223344+1");
    }

    // trc[verify lifecycle.finalising.ids]
    #[test]
    fn test_provisional_rules_empty_when_all_final() {
        let blocks = vec![
            rule_block("1:0", "repo.connect", "Connect a repo"),
            rule_block("1:1", "repo.multi-spec", "Multiple specs"),
        ];
        assert!(provisional_rules(&blocks).is_empty());
    }

    // trc[verify lifecycle.finalising]
    #[test]
    fn test_provisional_rules_ignores_headings_and_paragraphs() {
        let blocks = vec![
            heading_block("1:0", 1, "Section"),
            SpecBlock {
                key: "1:1".to_string(),
                kind: SpecBlockKind::Paragraph {
                    text: "Some prose".to_string(),
                },
                html: "<p>Some prose</p>".to_string(),
            },
            rule_block("1:2", "new.deadbeef+1", "A provisional rule"),
        ];
        let provs = provisional_rules(&blocks);
        assert_eq!(provs.len(), 1);
        assert_eq!(provs[0].0, "1:2");
    }
}
