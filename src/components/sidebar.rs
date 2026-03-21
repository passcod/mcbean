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

#[component]
pub fn SpecSidebar(specs: Vec<SpecOutline>) -> impl IntoView {
    let collapsed = RwSignal::new(false);
    let width = RwSignal::new(260.0f64);
    let dragging = RwSignal::new(false);
    let drag_start_x = RwSignal::new(0.0f64);
    let drag_start_w = RwSignal::new(0.0f64);

    view! {
        // Full-screen overlay while dragging: captures mousemove/mouseup
        // even when the pointer moves faster than the resize handle.
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

        // Sidebar + handle in a sticky full-height flex column
        <div style="display: flex; flex-shrink: 0; position: sticky; top: 0; height: 100vh; z-index: 10;">

            // Panel — width 0 + overflow hidden when collapsed
            <aside
                style:width=move || {
                    if collapsed.get() {
                        "0px".to_string()
                    } else {
                        format!("{}px", width.get())
                    }
                }
                style="overflow: hidden; display: flex; flex-direction: column; \
                       background: #fafafa; border-right: 1px solid #e5e7eb; \
                       transition: width 0.15s ease;"
            >
                // Header row: label + collapse button
                <div style="display: flex; align-items: center; justify-content: space-between; \
                            padding: 0.5rem 0.75rem; border-bottom: 1px solid #e5e7eb; \
                            flex-shrink: 0; min-width: 0;">
                    <span style="font-size: 0.7rem; font-weight: 700; text-transform: uppercase; \
                                 letter-spacing: 0.05em; color: #9ca3af; white-space: nowrap;">
                        "Outline"
                    </span>
                    <button
                        title="Collapse sidebar"
                        style="border: none; background: none; cursor: pointer; \
                               padding: 0 4px; color: #9ca3af; font-size: 1rem; line-height: 1; \
                               flex-shrink: 0;"
                        on:click=move |_| collapsed.set(true)
                    >
                        "‹"
                    </button>
                </div>

                // Scrollable outline
                <nav style="overflow-y: auto; flex: 1; padding: 0.4rem 0;">
                    {specs.into_iter().map(|spec| view! {
                        <div style="margin-bottom: 0.75rem;">
                            // Spec name — top-level entry
                            <div style="padding: 0.3rem 0.75rem; font-size: 0.8rem; \
                                        font-weight: 600; color: #111827; \
                                        white-space: nowrap; overflow: hidden; \
                                        text-overflow: ellipsis;">
                                {spec.name}
                            </div>
                            // Headings
                            {spec.headings.into_iter().map(|h| {
                                let indent = (h.level as u32).saturating_sub(1) * 10 + 12;
                                let href = format!("#{}", h.anchor);
                                view! {
                                    <a
                                        href=href
                                        class="sidebar-nav-link"
                                        style:padding-left=move || format!("{}px", indent)
                                        style="display: block; padding-top: 0.2rem; \
                                               padding-bottom: 0.2rem; padding-right: 0.75rem; \
                                               font-size: 0.75rem; color: #4b5563; \
                                               text-decoration: none; white-space: nowrap; \
                                               overflow: hidden; text-overflow: ellipsis;"
                                    >
                                        {h.text}
                                    </a>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }).collect::<Vec<_>>()}
                </nav>
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
                // Arrow only visible when collapsed
                <Show when=move || collapsed.get()>
                    <span style="font-size: 0.9rem; color: #9ca3af; line-height: 1;">
                        "›"
                    </span>
                </Show>
            </div>

        </div>
    }
}
