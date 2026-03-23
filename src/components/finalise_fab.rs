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
/// begin the finalisation flow (ID assignment + PR creation).
///
/// `on_finalise` is called when the user confirms. The parent is responsible
/// for the actual server action so it can track pending/error state.
#[component]
pub fn FinaliseFab(
    /// Called when the user clicks "Finalise & Submit" in the confirmation panel.
    on_finalise: Callback<()>,
    /// Whether the action is currently in progress (shows a spinner).
    #[prop(default = false)]
    pending: bool,
) -> impl IntoView {
    let panel_open = RwSignal::new(false);
    let hovered = RwSignal::new(false);

    view! {
        // ── Confirmation panel ────────────────────────────────────────────────
        // Always in DOM so the CSS opacity/transform transition plays.
        <div
            style:opacity=move || if panel_open.get() { "1" } else { "0" }
            style:pointer-events=move || if panel_open.get() { "auto" } else { "none" }
            style:transform=move || {
                if panel_open.get() {
                    "translateY(0)"
                } else {
                    "translateY(10px)"
                }
            }
            style="position: fixed; bottom: 5rem; right: 1.5rem; \
                   width: 300px; background: #fff; border-radius: 12px; \
                   border: 1px solid #e5e7eb; \
                   box-shadow: 0 8px 32px rgba(0,0,0,0.12); \
                   z-index: 1000; overflow: hidden; \
                   transition: opacity 0.2s ease, transform 0.2s ease;"
        >
            <div
                style="display: flex; align-items: center; \
                       justify-content: space-between; \
                       padding: 0.875rem 1rem 0.75rem; \
                       border-bottom: 1px solid #f3f4f6;"
            >
                <span style="font-size: 1rem; font-weight: 600; color: #111827;">
                    "Finalise Proposal"
                </span>
                <button
                    style="border: none; background: none; cursor: pointer; \
                           color: #9ca3af; font-size: 1.2rem; line-height: 1; \
                           padding: 0 4px; display: flex; letter-spacing: -0.05em;"
                    on:click=move |_| panel_open.set(false)
                >
                    "—"
                </button>
            </div>
            <div style="padding: 0.875rem 1rem 1rem;">
                <p class="is-size-7 has-text-grey mb-4">
                    "You'll review all changes and assign final rule IDs \
                     before submitting."
                </p>
                <button
                    class="button is-success is-fullwidth"
                    disabled=pending
                    on:click=move |_| {
                        panel_open.set(false);
                        on_finalise.run(());
                    }
                >
                    {move || if pending { "Finalising…" } else { "Review & Finalise" }}
                </button>
            </div>
        </div>

        // ── FAB ───────────────────────────────────────────────────────────────
        <button
            style:opacity=move || if panel_open.get() { "0" } else { "1" }
            style:pointer-events=move || if panel_open.get() { "none" } else { "auto" }
            style:background=move || {
                if hovered.get() { "#15803d" } else { "#16a34a" }
            }
            style="position: fixed; bottom: 1.5rem; right: 1.5rem; \
                   height: 56px; min-width: 56px; border-radius: 28px; \
                   border: none; color: #fff; cursor: pointer; \
                   display: flex; align-items: center; \
                   box-shadow: 0 4px 16px rgba(22,163,74,0.45); \
                   z-index: 1001; overflow: hidden; \
                   transition: background 0.15s ease, box-shadow 0.15s ease, \
                               opacity 0.15s ease;"
            on:mouseenter=move |_| hovered.set(true)
            on:mouseleave=move |_| hovered.set(false)
            on:click=move |_| panel_open.update(|v| *v = !*v)
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
                "Finalise"
            </span>
        </button>
    }
}
