use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadingEntry {
    pub level: u8,
    pub text: String,
    pub anchor: String,
}

#[derive(Clone, Debug)]
pub struct SpecOutline {
    pub name: String,
    pub headings: Vec<HeadingEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchEntry {
    pub spec_name: String,
    pub text: String,
    pub anchor: String,
}

#[component]
pub fn SpecSidebar(outline: Vec<SpecOutline>, search_entries: Vec<SearchEntry>) -> impl IntoView {
    let collapsed = RwSignal::new(false);
    let width = RwSignal::new(260.0f64);
    let dragging = RwSignal::new(false);
    let drag_start_x = RwSignal::new(0.0f64);
    let drag_start_w = RwSignal::new(0.0f64);
    let query = RwSignal::new(String::new());

    let outline = StoredValue::new(outline);
    let search_entries = StoredValue::new(search_entries);

    view! {
        // Full-screen overlay while dragging.
        <Show when=move || dragging.get()>
            <div
                class="sidebar-dragging"
                style="position: fixed; inset: 0; z-index: 9999;"
                on:mousemove=move |e| {
                    let dx = e.client_x() as f64 - drag_start_x.get_untracked();
                    let new_w = drag_start_w.get_untracked() + dx;
                    width.set(new_w.clamp(150.0, 600.0));
                }
                on:mouseup=move |_| dragging.set(false)
            />
        </Show>

        <div style="display: flex; flex-shrink: 0; position: sticky; top: 0; height: 100vh; z-index: 10;">

            <aside
                style:width=move || {
                    if collapsed.get() { "0px".to_string() }
                    else { format!("{}px", width.get()) }
                }
                style="overflow: hidden; display: flex; flex-direction: column; \
                       background: #fafafa; border-right: 1px solid #e5e7eb; \
                       transition: width 0.15s ease;"
            >
                // Header: search input + collapse button
                <div style="display: flex; align-items: center; gap: 0.25rem; \
                            padding: 0.4rem 0.5rem; border-bottom: 1px solid #e5e7eb; \
                            flex-shrink: 0; min-width: 0;">
                    // r[impl view.search]
                    <input
                        type="search"
                        placeholder="Search…"
                        prop:value=move || query.get()
                        on:input=move |ev| query.set(event_target_value(&ev))
                        style="flex: 1; min-width: 0; font-size: 0.75rem; \
                               border: 1px solid #d1d5db; border-radius: 4px; \
                               padding: 0.2rem 0.4rem; background: #fff; \
                               outline: none;"
                    />
                    <button
                        title="Collapse sidebar"
                        style="border: none; background: none; cursor: pointer; \
                               padding: 0 4px; color: #9ca3af; font-size: 1rem; \
                               line-height: 1; flex-shrink: 0;"
                        on:click=move |_| collapsed.set(true)
                    >
                        "‹"
                    </button>
                </div>

                // Body: outline or search results
                {move || {
                    let q = query.get();
                    if q.trim().is_empty() {
                        // r[impl view.nav]
                        outline.with_value(|specs| view! {
                            <nav style="overflow-y: auto; flex: 1; padding: 0.4rem 0;">
                                {specs.iter().map(|spec| {
                                    let name = spec.name.clone();
                                    let headings = spec.headings.clone();
                                    view! {
                                        <div style="margin-bottom: 0.75rem;">
                                            <div style="padding: 0.3rem 0.75rem; \
                                                        font-size: 0.8rem; font-weight: 600; \
                                                        color: #111827; white-space: nowrap; \
                                                        overflow: hidden; text-overflow: ellipsis;">
                                                {name}
                                            </div>
                                            {headings.into_iter().map(|h| {
                                                let indent = (h.level as u32).saturating_sub(1) * 10 + 12;
                                                let href = format!("#{}", h.anchor);
                                                view! {
                                                    <a
                                                        href=href
                                                        class="sidebar-nav-link"
                                                        style:padding-left=move || format!("{}px", indent)
                                                        style="display: block; padding-top: 0.2rem; \
                                                               padding-bottom: 0.2rem; \
                                                               padding-right: 0.75rem; \
                                                               font-size: 0.75rem; color: #4b5563; \
                                                               text-decoration: none; \
                                                               white-space: nowrap; overflow: hidden; \
                                                               text-overflow: ellipsis;"
                                                    >
                                                        {h.text}
                                                    </a>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </nav>
                        }).into_any()
                    } else {
                        // r[impl view.search]
                        let words: Vec<String> = q
                            .split_whitespace()
                            .map(|w| w.to_lowercase())
                            .collect();
                        let mut scored = search_entries.with_value(|entries| {
                            entries
                                .iter()
                                .filter_map(|e| {
                                    let text_lower = e.text.to_lowercase();
                                    let matches = words
                                        .iter()
                                        .filter(|w| text_lower.contains(w.as_str()))
                                        .count();
                                    if matches > 0 {
                                        Some((matches, e.clone()))
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                        });
                        // Stable sort: higher match count first, preserving
                        // original document order within the same score.
                        scored.sort_by(|a, b| b.0.cmp(&a.0));
                        let results: Vec<SearchEntry> =
                            scored.into_iter().map(|(_, e)| e).collect();

                        if results.is_empty() {
                            view! {
                                <div style="padding: 0.75rem; font-size: 0.75rem; color: #9ca3af;">
                                    "No results"
                                </div>
                            }.into_any()
                        } else {
                            // Group consecutive results by spec_name.
                            let mut groups: Vec<(String, Vec<SearchEntry>)> = Vec::new();
                            for entry in results {
                                if groups.last().map(|(n, _)| n == &entry.spec_name).unwrap_or(false) {
                                    groups.last_mut().unwrap().1.push(entry);
                                } else {
                                    groups.push((entry.spec_name.clone(), vec![entry]));
                                }
                            }

                            view! {
                                <div style="overflow-y: auto; flex: 1;">
                                    {groups.into_iter().map(|(spec_name, entries)| view! {
                                        <div style="margin-bottom: 0.5rem;">
                                            <div style="padding: 0.25rem 0.75rem; \
                                                        font-size: 0.65rem; font-weight: 700; \
                                                        text-transform: uppercase; \
                                                        letter-spacing: 0.05em; color: #9ca3af;">
                                                {spec_name}
                                            </div>
                                            {entries.into_iter().map(|e| {
                                                let snippet = if e.text.len() > 120 {
                                                    format!("{}…", &e.text[..120])
                                                } else {
                                                    e.text.clone()
                                                };
                                                let href = format!("#{}", e.anchor);
                                                view! {
                                                    <a
                                                        href=href
                                                        class="sidebar-nav-link"
                                                        style="display: block; \
                                                               padding: 0.25rem 0.75rem; \
                                                               font-size: 0.75rem; color: #4b5563; \
                                                               text-decoration: none; \
                                                               white-space: pre-wrap; \
                                                               word-break: break-word;"
                                                    >
                                                        {snippet}
                                                    </a>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }
                }}
            </aside>

            // Handle strip — col-resize when expanded, expand button when collapsed
            <div
                title=move || {
                    if collapsed.get() { "Expand sidebar" } else { "Drag to resize" }
                }
                style:cursor=move || {
                    if collapsed.get() { "pointer" } else { "col-resize" }
                }
                style:width=move || {
                    if collapsed.get() { "20px" } else { "5px" }
                }
                style:background=move || {
                    if collapsed.get() {
                        "#f3f4f6".to_string()
                    } else if dragging.get() {
                        "rgba(59,130,246,0.4)".to_string()
                    } else {
                        "transparent".to_string()
                    }
                }
                style="flex-shrink: 0; border-right: 1px solid #e5e7eb; \
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
                        "›"
                    </span>
                </Show>
            </div>

        </div>
    }
}
