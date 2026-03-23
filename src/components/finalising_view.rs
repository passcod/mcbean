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
    let provisionals = provisional_rules(&blocks);
    let has_provisionals = !provisionals.is_empty();

    // Editable ID overrides: key -> new slug typed by the user.
    // Starts empty — the inputs are blank for rules that still need assignment.
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
                                            let key = key.clone();
                                            view! {
                                                <tr>
                                                    <td>
                                                        <code class="has-text-warning-dark">
                                                            {old_id}
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
                                                            on:input={
                                                                let key = key.clone();
                                                                move |ev| {
                                                                    let val = event_target_value(&ev);
                                                                    if let Some((_, sig)) =
                                                                        overrides.get().get(i)
                                                                    {
                                                                        sig.set(val.clone());
                                                                    }
                                                                    let trimmed = val.trim().to_string();
                                                                    if !trimmed.is_empty() {
                                                                        on_id_change.run((key.clone(), trimmed));
                                                                    }
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

#[component]
fn ChangelogRow(entry: ChangelogEntry) -> impl IntoView {
    let kind_class = match entry.kind {
        ChangeKind::Added => "has-text-success",
        ChangeKind::Deleted => "has-text-danger",
        ChangeKind::Modified | ChangeKind::VersionBump => "has-text-warning-dark",
        ChangeKind::Reordered => "has-text-info",
    };
    let kind_label = match &entry.kind {
        ChangeKind::Added => "Added".to_string(),
        ChangeKind::Deleted => "Deleted".to_string(),
        ChangeKind::Modified => "Modified".to_string(),
        ChangeKind::Reordered => "Reordered".to_string(),
        ChangeKind::VersionBump => "Version bump".to_string(),
    };
    let label = entry.label.clone();

    // Word-level diff for modified blocks.
    let diff_view = match entry.kind {
        ChangeKind::Modified => {
            if let (Some(old), Some(new)) = (&entry.old_text, &entry.new_text) {
                let (old_spans, new_spans) = word_diff(old, new);
                Some(view! {
                    <div class="mt-2" style="font-size: 0.85em;">
                        <div class="mb-1">
                            <span class="has-text-weight-semibold has-text-danger-dark">"- "</span>
                            {old_spans
                                .into_iter()
                                .map(|(is_common, word)| {
                                    if is_common {
                                        view! { <span>{word}</span> }.into_any()
                                    } else {
                                        view! {
                                            <span
                                                style="background-color: #fdd; text-decoration: line-through;"
                                            >
                                                {word}
                                            </span>
                                        }
                                        .into_any()
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                        <div>
                            <span class="has-text-weight-semibold has-text-success-dark">"+ "</span>
                            {new_spans
                                .into_iter()
                                .map(|(is_common, word)| {
                                    if is_common {
                                        view! { <span>{word}</span> }.into_any()
                                    } else {
                                        view! {
                                            <span style="background-color: #dfd;">
                                                {word}
                                            </span>
                                        }
                                        .into_any()
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    </div>
                })
            } else {
                None
            }
        }
        _ => None,
    };

    view! {
        <div class="mb-3 p-3" style="border-left: 3px solid #dbdbdb; background: #fafafa;">
            <div class="is-flex is-align-items-center">
                <span class={format!("tag is-light mr-2 {kind_class}")}>{kind_label}</span>
                <span class="has-text-weight-medium">{label}</span>
            </div>
            {diff_view}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_block(key: &str, id: &str, text: &str) -> SpecBlock {
        SpecBlock {
            key: key.to_string(),
            kind: SpecBlockKind::Rule {
                prefix: "r".to_string(),
                id: id.into(),
                text: text.into(),
            },
            html: format!("<p>{text}</p>"),
        }
    }

    fn heading_block(key: &str, level: u8, text: &str) -> SpecBlock {
        SpecBlock {
            key: key.to_string(),
            kind: SpecBlockKind::Heading {
                level,
                text: text.into(),
                anchor: text.to_lowercase().replace(' ', "-"),
            },
            html: format!("<h{level}>{text}</h{level}>"),
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
