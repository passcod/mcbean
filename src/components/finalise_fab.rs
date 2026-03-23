use leptos::prelude::*;

// Feather "check" icon.
const CHECK_ICON: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" "#,
    r#"viewBox="0 0 24 24" fill="none" stroke="currentColor" "#,
    r#"stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">"#,
    r#"<polyline points="20 6 9 17 4 12"/>"#,
    r#"</svg>"#,
);

/// Floating action button shown on a drafting proposal, allowing the user to
/// begin the finalisation flow (ID review + submission).
///
/// Clicking the FAB fires `on_finalise` immediately — the finalising view
/// itself has a button to return to editing, so no confirmation gate is needed.
#[component]
pub fn FinaliseFab(
    /// Called when the user clicks the FAB.
    on_finalise: Callback<()>,
    /// Whether the action is currently in progress (shows a spinner).
    #[prop(default = false)]
    pending: bool,
) -> impl IntoView {
    let hovered = RwSignal::new(false);

    view! {
        <button
            style:background=move || {
                if pending {
                    "#94a3b8"
                } else if hovered.get() {
                    "#15803d"
                } else {
                    "#16a34a"
                }
            }
            style="position: fixed; bottom: 1.5rem; right: 1.5rem; \
                   height: 56px; min-width: 56px; border-radius: 28px; \
                   border: none; color: #fff; cursor: pointer; \
                   display: flex; align-items: center; \
                   box-shadow: 0 4px 16px rgba(22,163,74,0.45); \
                   z-index: 1001; overflow: hidden; \
                   transition: background 0.15s ease, box-shadow 0.15s ease;"
            disabled=pending
            on:mouseenter=move |_| hovered.set(true)
            on:mouseleave=move |_| hovered.set(false)
            on:click=move |_| on_finalise.run(())
        >
            <span
                inner_html=CHECK_ICON
                style="display: flex; align-items: center; justify-content: center; \
                       width: 56px; height: 56px; flex-shrink: 0;"
            />
            <span
                style:max-width=move || if hovered.get() { "140px" } else { "0px" }
                style:opacity=move || if hovered.get() { "1" } else { "0" }
                style:padding-right=move || if hovered.get() { "1.25rem" } else { "0" }
                style="overflow: hidden; white-space: nowrap; \
                       font-size: 0.875rem; font-weight: 600; \
                       transition: max-width 0.2s ease, opacity 0.15s ease, \
                                   padding-right 0.2s ease;"
            >
                {move || if pending { "Finalising…" } else { "Review & Finalise" }}
            </span>
        </button>
    }
}
