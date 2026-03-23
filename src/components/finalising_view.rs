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
            SpecBlockKind::Rule { id, text } if is_provisional_id(id) => {
                Some((b.key.clone(), id.clone()))
            }
            _ => None,
        })
        .collect()
}

// r[impl lifecycle.finalising]
// r[impl lifecycle.finalising.ids]
#[component]
pub fn FinalisingView(
    /// Current proposal blocks (with any edits applied).
    blocks: Vec<SpecBlock>,
    /// Base snapshot blocks for changelog comparison.
    base_blocks: Vec<SpecBlock>,
    /// Called when the user wants to go back to drafting.
    on_back: Callback<()>,
    /// Called when the user confirms submission, passing `(tree_key, new_id)` pairs.
    on_submit: Callback<Vec<(String, String)>>,
    /// Whether submission is currently in progress.
    #[prop(into)]
    submitting: Signal<bool>,
    /// Optional error message from a failed submission attempt.
    #[prop(into)]
    submit_error: Signal<Option<String>>,
) -> impl IntoView {
    let provisionals = provisional_rules(&blocks);
    let has_provisionals = !provisionals.is_empty();

    // Editable ID overrides: key -> new slug. Populated as users type replacements.
    let id_overrides: RwSignal<Vec<(String, RwSignal<String>)>> = RwSignal::new(
        provisionals
            .iter()
            .map(|(key, _old_id)| (key.clone(), RwSignal::new(String::new())))
            .collect(),
    );

    // Check whether all provisionals have been given a non-empty replacement.
    let all_resolved = Memo::new(move |_| {
        if !has_provisionals {
            return true;
        }
        id_overrides
            .get()
            .iter()
            .all(|(_, sig)| !sig.get().trim().is_empty())
    });

    let provisional_keys: Vec<String> = provisionals.iter().map(|(k, _)| k.clone()).collect();

    let changelog = compute_changelog(&base_blocks, &blocks);

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

            // Provisional ID resolution section.
            {if has_provisionals {
                let prov_list = provisionals.clone();
                let overrides = id_overrides;
                view! {
                    <div class="notification is-warning is-light mb-5">
                        <p class="mb-3">
                            <strong>{format!("{} rule(s) still have provisional IDs:", prov_list.len())}</strong>
                        </p>
                        <div class="table-container">
                            <table class="table is-fullwidth is-narrow">
                                <thead>
                                    <tr>
                                        <th>"Provisional ID"</th>
                                        <th>"New ID"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {prov_list
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (key, old_id))| {
                                            let old_id = old_id.clone();
                                            let _key = key.clone();
                                            view! {
                                                <tr>
                                                    <td>
                                                        <code class="has-text-warning-dark">
                                                            {format!("r[{}]", old_id)}
                                                        </code>
                                                    </td>
                                                    <td>
                                                        <input
                                                            class="input is-small"
                                                            type="text"
                                                            placeholder="e.g. security.auth-tokens"
                                                            prop:value=move || {
                                                                overrides
                                                                    .get()
                                                                    .get(i)
                                                                    .map(|(_, s)| s.get())
                                                                    .unwrap_or_default()
                                                            }
                                                            on:input=move |ev| {
                                                                let val = event_target_value(&ev);
                                                                if let Some((_, sig)) =
                                                                    overrides.get().get(i)
                                                                {
                                                                    sig.set(val);
                                                                }
                                                            }
                                                        />
                                                    </td>
                                                </tr>
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        </div>
                    </div>
                }
                .into_any()
            } else {
                view! {
                    <div class="notification is-success is-light mb-5">
                        <p>"All rule IDs are finalised."</p>
                    </div>
                }
                .into_any()
            }}

            // Changelog.
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
                                view! { <ChangelogRow entry=entry /> }
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
                    on:click={
                        let provisional_keys = provisional_keys.clone();
                        move |_| {
                            let overrides: Vec<(String, String)> = provisional_keys
                                .iter()
                                .enumerate()
                                .filter_map(|(i, key)| {
                                    let new_id = id_overrides
                                        .get()
                                        .get(i)
                                        .map(|(_, sig)| sig.get())
                                        .unwrap_or_default();
                                    let trimmed = new_id.trim().to_string();
                                    if trimmed.is_empty() {
                                        None
                                    } else {
                                        Some((key.clone(), trimmed))
                                    }
                                })
                                .collect();
                            on_submit.run(overrides);
                        }
                    }
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

#[component]
fn ChangelogRow(entry: ChangelogEntry) -> impl IntoView {
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
                <div>
                    <span class=kind_class style="font-weight: 600; margin-right: 0.5rem;">
                        {kind_label}
                    </span>
                    <span>{entry.label.clone()}</span>
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
                            let (_old_spans, new_spans) = word_diff(o, n);
                            view! {
                                <div
                                    class="content"
                                    style="background: #f5f5f5; padding: 0.75rem; border-radius: 4px; white-space: pre-wrap;"
                                >
                                    {new_spans
                                        .into_iter()
                                        .map(|(is_common, text)| {
                                            if !is_common {
                                                view! {
                                                    <span style="background: #fde68a; padding: 0 2px;">
                                                        {text}
                                                    </span>
                                                }
                                                .into_any()
                                            } else {
                                                view! { <span>{text}" "</span> }.into_any()
                                            }
                                        })
                                        .collect::<Vec<_>>()}
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
