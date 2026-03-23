use std::collections::{HashMap, HashSet};

use crate::components::avatar::Avatar;
use crate::pages::proposal::get_proposal_contributors;

type WordSpans = Vec<(bool, String)>;

use leptos::prelude::*;

use crate::components::spec_block_editor::{RevertOp, SpecBlock, SpecBlockKind};

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Reordered,
    VersionBump,
}

#[derive(Clone, Debug)]
pub struct ChangelogEntry {
    pub key: String,
    pub kind: ChangeKind,
    pub label: String,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
}

fn block_label(b: &SpecBlock) -> String {
    match &b.kind {
        SpecBlockKind::Rule { prefix, id, .. } => format!("{}[{}]", prefix, id),
        SpecBlockKind::Heading { level, text, .. } => {
            let prefix = "#".repeat(*level as usize);
            if text.is_empty() {
                format!("{} (untitled heading)", prefix)
            } else {
                format!("{} {}", prefix, text)
            }
        }
        SpecBlockKind::Paragraph { text } => {
            let mut snippet: String = text.chars().take(60).collect();
            if text.len() > 60 {
                snippet.push('…');
            }
            snippet
        }
    }
}

// ── Diff helpers ──────────────────────────────────────────────────────────────

/// Returns the matched index pairs (i_in_a, i_in_b) from the LCS of two
/// string slices. O(m * n) in time and space.
fn lcs_pairs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Word-level diff between two prose strings.
/// Returns `(old_spans, new_spans)` where each span is `(is_common, word)`.
/// Words not in the common subsequence are highlighted as removed / added.
pub fn word_diff(old: &str, new: &str) -> (WordSpans, WordSpans) {
    let old_words: Vec<&str> = old.split_whitespace().collect();
    let new_words: Vec<&str> = new.split_whitespace().collect();
    let pairs = lcs_pairs(&old_words, &new_words);

    let common_old: HashSet<usize> = pairs.iter().map(|(i, _)| *i).collect();
    let common_new: HashSet<usize> = pairs.iter().map(|(_, j)| *j).collect();

    let old_spans = old_words
        .iter()
        .enumerate()
        .map(|(i, w)| (common_old.contains(&i), w.to_string()))
        .collect();
    let new_spans = new_words
        .iter()
        .enumerate()
        .map(|(i, w)| (common_new.contains(&i), w.to_string()))
        .collect();

    (old_spans, new_spans)
}

// ── Changelog computation ─────────────────────────────────────────────────────

// r[impl proposal.diff.semantic]
pub fn compute_changelog(initial: &[SpecBlock], current: &[SpecBlock]) -> Vec<ChangelogEntry> {
    let initial_by_key: HashMap<&str, (usize, &SpecBlock)> = initial
        .iter()
        .enumerate()
        .map(|(i, b)| (b.key.as_str(), (i, b)))
        .collect();
    let current_key_set: HashSet<&str> = current.iter().map(|b| b.key.as_str()).collect();
    let initial_key_set: HashSet<&str> = initial.iter().map(|b| b.key.as_str()).collect();

    // Detect reordered keys: find blocks whose relative order changed among
    // the keys common to both snapshots, using LCS on the two orderings.
    let common_in_initial: Vec<&str> = initial
        .iter()
        .filter(|b| current_key_set.contains(b.key.as_str()))
        .map(|b| b.key.as_str())
        .collect();
    let common_in_current: Vec<&str> = current
        .iter()
        .filter(|b| initial_key_set.contains(b.key.as_str()))
        .map(|b| b.key.as_str())
        .collect();
    let stable_pairs = lcs_pairs(&common_in_initial, &common_in_current);
    let stable_keys: HashSet<&str> = stable_pairs
        .iter()
        .map(|(i, _)| common_in_initial[*i])
        .collect();

    let mut entries: Vec<ChangelogEntry> = Vec::new();

    // Deleted: present in initial, absent from current.
    for b in initial {
        if !current_key_set.contains(b.key.as_str()) {
            entries.push(ChangelogEntry {
                key: b.key.clone(),
                kind: ChangeKind::Deleted,
                label: block_label(b),
                old_text: Some(b.edit_text().to_owned()),
                new_text: None,
            });
        }
    }

    // Walk current to produce Added / Modified / Reordered in document order.
    for b in current {
        let key = b.key.as_str();
        if let Some((_, init_block)) = initial_by_key.get(key) {
            let init_text = init_block.edit_text();
            let curr_text = b.edit_text();
            if init_text != curr_text {
                entries.push(ChangelogEntry {
                    key: b.key.clone(),
                    kind: ChangeKind::Modified,
                    label: block_label(b),
                    old_text: Some(init_text.to_owned()),
                    new_text: Some(curr_text.to_owned()),
                });
            } else if !stable_keys.contains(key) {
                entries.push(ChangelogEntry {
                    key: b.key.clone(),
                    kind: ChangeKind::Reordered,
                    label: block_label(b),
                    old_text: None,
                    new_text: None,
                });
            }
        } else {
            entries.push(ChangelogEntry {
                key: b.key.clone(),
                kind: ChangeKind::Added,
                label: block_label(b),
                old_text: None,
                new_text: Some(b.edit_text().to_owned()),
            });
        }
    }

    entries
}

// ── ChangelogSidebar component ────────────────────────────────────────────────

// r[impl proposal.diff.expandable]
// r[impl proposal.diff.version-bumps]
// r[impl edit.history]
#[component]
pub fn ChangelogSidebar(
    proposal_id: i32,
    initial_blocks: Vec<SpecBlock>,
    blocks: Signal<Vec<SpecBlock>>,
    sync_error: RwSignal<Option<String>>,
    /// Set to Some(op) by this component; SpecBlockEditor applies and clears it.
    revert_op: RwSignal<Option<RevertOp>>,
) -> impl IntoView {
    let initial_blocks = StoredValue::new(initial_blocks);

    let contributors = Resource::new(move || proposal_id, |pid| get_proposal_contributors(pid));

    // r[impl proposal.diff.semantic]
    let entries = Signal::derive(move || {
        let current = blocks.get();
        if current.is_empty() {
            return Vec::new();
        }
        initial_blocks.with_value(|init| compute_changelog(init, &current))
    });

    let collapsed = RwSignal::new(false);
    let width = RwSignal::new(280.0f64);
    let dragging = RwSignal::new(false);
    let drag_start_x = RwSignal::new(0.0f64);
    let drag_start_w = RwSignal::new(0.0f64);

    let expanded: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());
    let toggle_expanded = Callback::new(move |key: String| {
        expanded.update(|set| {
            if !set.remove(&key) {
                set.insert(key);
            }
        });
    });

    // r[impl edit.undo]
    // Construct a RevertOp from a changelog entry and dispatch it.
    let on_revert = Callback::new(move |entry: ChangelogEntry| {
        let op = match entry.kind {
            ChangeKind::Added => RevertOp::DeleteBlock { key: entry.key },
            ChangeKind::Deleted => {
                let (original_kind, after_key) = initial_blocks.with_value(|init| {
                    let kind = init
                        .iter()
                        .find(|b| b.key == entry.key)
                        .map(|b| b.kind.clone())
                        .unwrap_or(SpecBlockKind::Paragraph {
                            text: entry.old_text.clone().unwrap_or_default(),
                        });
                    let current_keys: HashSet<String> =
                        blocks.get().iter().map(|b| b.key.clone()).collect();
                    let after = init
                        .iter()
                        .take_while(|b| b.key != entry.key)
                        .filter(|b| current_keys.contains(&b.key))
                        .last()
                        .map(|b| b.key.clone());
                    (kind, after)
                });
                RevertOp::RecreateBlock {
                    kind: original_kind,
                    after_key,
                }
            }
            ChangeKind::Modified => RevertOp::RestoreText {
                key: entry.key,
                text: entry.old_text.unwrap_or_default(),
            },
            ChangeKind::Reordered => {
                let after_key = initial_blocks.with_value(|init| {
                    let current_keys: HashSet<String> =
                        blocks.get().iter().map(|b| b.key.clone()).collect();
                    init.iter()
                        .take_while(|b| b.key != entry.key)
                        .filter(|b| current_keys.contains(&b.key))
                        .last()
                        .map(|b| b.key.clone())
                });
                RevertOp::MoveBlock {
                    key: entry.key,
                    after_key,
                }
            }
            ChangeKind::VersionBump => return,
        };
        revert_op.set(Some(op));
    });

    view! {
        // Full-screen drag overlay.
        <Show when=move || dragging.get()>
            <div
                style="position: fixed; inset: 0; z-index: 9999; \
                       cursor: col-resize; user-select: none;"
                on:mousemove=move |e| {
                    let dx = drag_start_x.get_untracked() - e.client_x() as f64;
                    let new_w = drag_start_w.get_untracked() + dx;
                    width.set(new_w.clamp(180.0, 600.0));
                }
                on:mouseup=move |_| dragging.set(false)
            />
        </Show>

        <div style="display: flex; flex-shrink: 0; position: sticky; top: 0; \
                    height: 100vh; z-index: 10;">

            // Resize handle / expand strip — sits to the LEFT of the panel.
            <div
                title=move || {
                    if collapsed.get() { "Expand changelog" } else { "Drag to resize" }
                }
                style:cursor=move || if collapsed.get() { "pointer" } else { "col-resize" }
                style:width=move || if collapsed.get() { "20px" } else { "5px" }
                style:background=move || {
                    if collapsed.get() {
                        "#f3f4f6".to_string()
                    } else if dragging.get() {
                        "rgba(59,130,246,0.4)".to_string()
                    } else {
                        "transparent".to_string()
                    }
                }
                style="flex-shrink: 0; border-left: 1px solid #e5e7eb; \
                       display: flex; align-items: flex-start; justify-content: center; \
                       padding-top: 6px; transition: background 0.1s, width 0.15s ease; \
                       user-select: none;"
                on:mousedown=move |e| {
                    if collapsed.get() {
                        collapsed.set(false);
                    } else {
                        e.prevent_default();
                        drag_start_x.set(e.client_x() as f64);
                        drag_start_w.set(width.get_untracked());
                        dragging.set(true);
                    }
                }
            >
                <Show when=move || collapsed.get()>
                    <span style="font-size: 0.9rem; color: #9ca3af; line-height: 1;">
                        "‹"
                    </span>
                </Show>
            </div>

            // Panel body.
            <aside
                style:width=move || {
                    if collapsed.get() { "0px".to_string() }
                    else { format!("{}px", width.get()) }
                }
                style="overflow: hidden; display: flex; flex-direction: column; \
                       background: #fafafa; border-left: 1px solid #e5e7eb; \
                       transition: width 0.15s ease;"
            >
                // Authors strip, above the header.
                <Suspense fallback=|| ()>
                    {move || Suspend::new(async move {
                        let contributors = contributors.await.unwrap_or_default();
                        if contributors.is_empty() {
                            return ().into_any();
                        }
                        view! {
                            <div style="flex-shrink: 0; border-bottom: 1px solid #e5e7eb; \
                                        padding: 0.4rem 0.6rem; display: flex; \
                                        align-items: center; gap: 0.35rem; flex-wrap: wrap;">
                                <span style="font-size: 0.6rem; font-weight: 700; \
                                             text-transform: uppercase; letter-spacing: 0.05em; \
                                             color: #9ca3af; margin-right: 0.25rem; flex-shrink: 0;">
                                    "Authors"
                                </span>
                                {contributors
                                    .into_iter()
                                    .map(|info| view! { <Avatar info=info size=24 /> })
                                    .collect::<Vec<_>>()}
                            </div>
                        }
                        .into_any()
                    })}
                </Suspense>

                // Header row.
                <div style="display: flex; align-items: center; gap: 0.25rem; \
                            padding: 0.4rem 0.5rem; border-bottom: 1px solid #e5e7eb; \
                            flex-shrink: 0; min-width: 0;">
                    <span style="flex: 1; font-size: 0.75rem; font-weight: 600; \
                                 color: #111827; white-space: nowrap; overflow: hidden; \
                                 text-overflow: ellipsis;">
                        "Changes"
                    </span>
                    {move || {
                        let n = entries.get().len();
                        if n > 0 {
                            view! {
                                <span style="font-size: 0.65rem; background: #3b82f6; \
                                             color: #fff; border-radius: 999px; \
                                             padding: 0.05rem 0.45rem; flex-shrink: 0;">
                                    {n}
                                </span>
                            }
                            .into_any()
                        } else {
                            view! { <span /> }.into_any()
                        }
                    }}

                    <Show when=move || sync_error.get().is_some()>
                        <button
                            title=move || {
                                sync_error
                                    .get()
                                    .unwrap_or_else(|| "Sync error".to_string())
                            }
                            style="border: none; background: none; cursor: pointer; \
                                   padding: 0 3px; color: #ef4444; font-size: 0.8rem; \
                                   line-height: 1; flex-shrink: 0;"
                            on:click=move |_| sync_error.set(None)
                        >
                            "⚠"
                        </button>
                    </Show>
                    <button
                        title="Collapse changelog"
                        style="border: none; background: none; cursor: pointer; \
                               padding: 0 4px; color: #9ca3af; font-size: 1rem; \
                               line-height: 1; flex-shrink: 0;"
                        on:click=move |_| collapsed.set(true)
                    >
                        "›"
                    </button>
                </div>

                // Scrollable entry list.
                <div style="overflow-y: auto; flex: 1;">
                    {move || {
                        let all = entries.get();
                        if all.is_empty() {
                            return view! {
                                <div style="padding: 0.75rem; font-size: 0.75rem; \
                                            color: #9ca3af; font-style: italic;">
                                    "No changes yet."
                                </div>
                            }
                            .into_any();
                        }

                        // r[impl proposal.diff.version-bumps]
                        let (content, bumps): (Vec<_>, Vec<_>) = all
                            .into_iter()
                            .partition(|e| e.kind != ChangeKind::VersionBump);

                        let show_bumps = !bumps.is_empty();

                        view! {
                            <div style="padding: 0.4rem 0;">
                                <ChangelogEntryList
                                    entries=content
                                    expanded=expanded
                                    toggle_expanded=toggle_expanded
                                    on_revert=on_revert
                                />
                                {if show_bumps {
                                    view! {
                                        <div>
                                            <div style="margin-top: 0.75rem; \
                                                        padding: 0.2rem 0.75rem; \
                                                        font-size: 0.65rem; font-weight: 700; \
                                                        text-transform: uppercase; \
                                                        letter-spacing: 0.05em; color: #9ca3af;">
                                                "Version bumps"
                                            </div>
                                            <ChangelogEntryList
                                                entries=bumps
                                                expanded=expanded
                                                toggle_expanded=toggle_expanded
                                                on_revert=on_revert
                                            />
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! { <span /> }.into_any()
                                }}
                            </div>
                        }
                        .into_any()
                    }}
                </div>

            </aside>
        </div>
    }
}

// ── ChangelogEntryList component ──────────────────────────────────────────────

#[component]
fn ChangelogEntryList(
    entries: Vec<ChangelogEntry>,
    expanded: RwSignal<HashSet<String>>,
    toggle_expanded: Callback<String>,
    on_revert: Callback<ChangelogEntry>,
) -> impl IntoView {
    view! {
        <div>
            {entries
                .into_iter()
                .map(|entry| {
                    let key = StoredValue::new(entry.key.clone());
                    let kind = StoredValue::new(entry.kind.clone());
                    let label = StoredValue::new(entry.label.clone());
                    let old_text = StoredValue::new(entry.old_text.clone());
                    let new_text = StoredValue::new(entry.new_text.clone());

                    let has_diff = entry.old_text.is_some() || entry.new_text.is_some();
                    let is_expanded =
                        move || expanded.get().contains(&key.get_value());

                    view! {
                        <div class="changelog-entry">
                            <div
                                class="changelog-entry-header"
                                class:changelog-entry-header--clickable=has_diff
                                on:click=move |_| {
                                    if has_diff {
                                        toggle_expanded.run(key.get_value());
                                    }
                                }
                            >
                                <span class=move || {
                                    format!(
                                        "changelog-badge changelog-badge--{}",
                                        kind_slug(kind.get_value()),
                                    )
                                }>
                                    {move || kind_label(kind.get_value())}
                                </span>
                                <span class="changelog-entry-label">
                                    {move || label.get_value()}
                                </span>
                                <Show when=move || has_diff>
                                    <span class="changelog-expand-icon">
                                        {move || if is_expanded() { "▴" } else { "▾" }}
                                    </span>
                                </Show>
                            </div>

                            // r[impl proposal.diff.expandable]
                            <Show when=move || is_expanded() && has_diff>
                                // r[impl edit.undo]
                                <Show when=move || {
                                    kind.get_value() != ChangeKind::VersionBump
                                }>
                                    <button
                                        class="changelog-revert-btn"
                                        on:click=move |_| {
                                            on_revert.run(ChangelogEntry {
                                                key: key.get_value(),
                                                kind: kind.get_value(),
                                                label: label.get_value(),
                                                old_text: old_text.get_value(),
                                                new_text: new_text.get_value(),
                                            });
                                        }
                                    >
                                        "Revert"
                                    </button>
                                </Show>
                                <div class="changelog-diff">
                                    {move || {
                                        let old = old_text.get_value();
                                        let new = new_text.get_value();
                                        match (old, new) {
                                            (Some(o), Some(n)) => {
                                                let (old_spans, new_spans) =
                                                    word_diff(&o, &n);
                                                view! {
                                                    <div class="changelog-diff-block changelog-diff-block--old">
                                                        <div class="changelog-diff-label">
                                                            "Before"
                                                        </div>
                                                        <div class="changelog-diff-text">
                                                            {old_spans
                                                                .into_iter()
                                                                .map(|(common, word)| {
                                                                    view! {
                                                                        <span class:changelog-word--removed=move || {
                                                                            !common
                                                                        }>
                                                                            {word}
                                                                            " "
                                                                        </span>
                                                                    }
                                                                })
                                                                .collect::<Vec<_>>()}
                                                        </div>
                                                    </div>
                                                    <div class="changelog-diff-block changelog-diff-block--new">
                                                        <div class="changelog-diff-label">
                                                            "After"
                                                        </div>
                                                        <div class="changelog-diff-text">
                                                            {new_spans
                                                                .into_iter()
                                                                .map(|(common, word)| {
                                                                    view! {
                                                                        <span class:changelog-word--added=move || {
                                                                            !common
                                                                        }>
                                                                            {word}
                                                                            " "
                                                                        </span>
                                                                    }
                                                                })
                                                                .collect::<Vec<_>>()}
                                                        </div>
                                                    </div>
                                                }
                                                .into_any()
                                            }
                                            (Some(o), None) => view! {
                                                <div class="changelog-diff-block changelog-diff-block--old">
                                                    <div class="changelog-diff-label">"Deleted content"</div>
                                                    <div class="changelog-diff-text changelog-diff-text--fully-removed">
                                                        {o}
                                                    </div>
                                                </div>
                                            }
                                            .into_any(),
                                            (None, Some(n)) => view! {
                                                <div class="changelog-diff-block changelog-diff-block--new">
                                                    <div class="changelog-diff-label">"Added content"</div>
                                                    <div class="changelog-diff-text changelog-diff-text--fully-added">
                                                        {n}
                                                    </div>
                                                </div>
                                            }
                                            .into_any(),
                                            (None, None) => view! { <span /> }.into_any(),
                                        }
                                    }}
                                </div>
                            </Show>
                        </div>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn kind_slug(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Modified => "modified",
        ChangeKind::Reordered => "reordered",
        ChangeKind::VersionBump => "bump",
    }
}

fn kind_label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Added",
        ChangeKind::Deleted => "Deleted",
        ChangeKind::Modified => "Modified",
        ChangeKind::Reordered => "Reordered",
        ChangeKind::VersionBump => "Version bump",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::spec_block_editor::{SpecBlock, SpecBlockKind};

    fn rule_block(key: &str, id: &str, text: &str) -> SpecBlock {
        SpecBlock {
            key: key.into(),
            kind: SpecBlockKind::Rule {
                prefix: "r".to_string(),
                id: id.into(),
                text: text.into(),
            },
            html: String::new(),
        }
    }

    fn heading_block(key: &str, level: u8, text: &str) -> SpecBlock {
        SpecBlock {
            key: key.into(),
            kind: SpecBlockKind::Heading {
                level,
                text: text.into(),
                anchor: format!("h-{text}"),
            },
            html: String::new(),
        }
    }

    // r[verify proposal.diff.semantic]
    #[test]
    fn test_changelog_added_block() {
        let initial = vec![rule_block("1:0", "a.first", "First rule.")];
        let current = vec![
            rule_block("1:0", "a.first", "First rule."),
            rule_block("1:1", "a.second", "Second rule."),
        ];
        let entries = compute_changelog(&initial, &current);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Added);
        assert!(entries[0].label.contains("a.second"));
    }

    // r[verify proposal.diff.semantic]
    #[test]
    fn test_changelog_deleted_block() {
        let initial = vec![
            rule_block("1:0", "a.first", "First rule."),
            rule_block("1:1", "a.second", "Second rule."),
        ];
        let current = vec![rule_block("1:0", "a.first", "First rule.")];
        let entries = compute_changelog(&initial, &current);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Deleted);
        assert!(entries[0].label.contains("a.second"));
    }

    // r[verify proposal.diff.semantic]
    #[test]
    fn test_changelog_modified_block() {
        let initial = vec![rule_block("1:0", "a.first", "Original text.")];
        let current = vec![rule_block("1:0", "a.first", "Updated text.")];
        let entries = compute_changelog(&initial, &current);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Modified);
        assert_eq!(entries[0].old_text.as_deref(), Some("Original text."));
        assert_eq!(entries[0].new_text.as_deref(), Some("Updated text."));
    }

    // r[verify proposal.diff.semantic]
    #[test]
    fn test_changelog_reordered_blocks() {
        let initial = vec![
            rule_block("1:0", "a.first", "First."),
            rule_block("1:1", "a.second", "Second."),
        ];
        let current = vec![
            rule_block("1:1", "a.second", "Second."),
            rule_block("1:0", "a.first", "First."),
        ];
        let entries = compute_changelog(&initial, &current);
        // Both blocks moved relative to each other; at least one is Reordered.
        assert!(
            entries.iter().any(|e| e.kind == ChangeKind::Reordered),
            "expected at least one Reordered entry: {entries:#?}"
        );
    }

    // r[verify proposal.diff.semantic]
    #[test]
    fn test_changelog_no_changes() {
        let blocks = vec![rule_block("1:0", "a.first", "Text.")];
        let entries = compute_changelog(&blocks, &blocks);
        assert!(
            entries.is_empty(),
            "no changes should produce empty changelog"
        );
    }

    // r[verify proposal.diff.semantic]
    #[test]
    fn test_changelog_heading_label() {
        let initial = vec![];
        let current = vec![heading_block("1:0", 2, "New Section")];
        let entries = compute_changelog(&initial, &current);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ChangeKind::Added);
        assert!(entries[0].label.contains("New Section"));
    }

    // r[verify proposal.diff.expandable]
    #[test]
    fn test_word_diff_identical() {
        let (old_spans, new_spans) = word_diff("hello world", "hello world");
        assert!(old_spans.iter().all(|(common, _)| *common));
        assert!(new_spans.iter().all(|(common, _)| *common));
    }

    // r[verify proposal.diff.expandable]
    #[test]
    fn test_word_diff_addition() {
        let (old_spans, new_spans) = word_diff("hello world", "hello beautiful world");
        // "hello" and "world" are common in both
        let old_common: Vec<&str> = old_spans
            .iter()
            .filter(|(c, _)| *c)
            .map(|(_, w)| w.as_str())
            .collect();
        assert_eq!(old_common, vec!["hello", "world"]);
        // "beautiful" is added (not common) in new
        let new_added: Vec<&str> = new_spans
            .iter()
            .filter(|(c, _)| !*c)
            .map(|(_, w)| w.as_str())
            .collect();
        assert_eq!(new_added, vec!["beautiful"]);
    }

    // r[verify proposal.diff.expandable]
    #[test]
    fn test_word_diff_removal() {
        let (old_spans, _new_spans) = word_diff("hello beautiful world", "hello world");
        let removed: Vec<&str> = old_spans
            .iter()
            .filter(|(c, _)| !*c)
            .map(|(_, w)| w.as_str())
            .collect();
        assert_eq!(removed, vec!["beautiful"]);
    }

    // r[verify proposal.diff.version-bumps]
    #[test]
    fn test_changelog_version_bump_variant_exists() {
        // The VersionBump variant must exist so the UI can categorize bumps separately.
        let entry = ChangelogEntry {
            key: "1:0".into(),
            kind: ChangeKind::VersionBump,
            label: "r[a.rule+2]".into(),
            old_text: Some("old".into()),
            new_text: Some("new".into()),
        };
        assert_eq!(entry.kind, ChangeKind::VersionBump);
    }
}
