use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::pages::proposal::create_proposal;

// Feather-style "edit" pencil icon rendered via inner_html so SVG attributes
// don't need to go through the view! macro attribute parser.
const EDIT_ICON: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" "#,
    r#"viewBox="0 0 24 24" fill="none" stroke="currentColor" "#,
    r#"stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">"#,
    r#"<path d="M12 20h9"/>"#,
    r#"<path d="M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z"/>"#,
    r#"</svg>"#,
);

#[component]
pub fn ProposalFab(repo_id: i32) -> impl IntoView {
    let panel_open = RwSignal::new(false);
    let hovered = RwSignal::new(false);
    let title = RwSignal::new(String::new());
    let navigate = use_navigate();

    // r[impl users.collaboration]
    let create_action = Action::new(move |_: &()| {
        let t = title.get();
        async move { create_proposal(repo_id, t).await }
    });

    Effect::new(move |_| {
        if let Some(Ok(new_id)) = create_action.value().get() {
            navigate(
                &format!("/repo/{}/proposal/{}", repo_id, new_id),
                Default::default(),
            );
        }
    });

    view! {
        // Panel — always in DOM so the CSS transition plays on open/close.
        <div
            style:opacity=move || if panel_open.get() { "1" } else { "0" }
            style:pointer-events=move || if panel_open.get() { "auto" } else { "none" }
            style:transform=move || {
                if panel_open.get() { "translateY(0)" } else { "translateY(10px)" }
            }
            style="position: fixed; bottom: 1.5rem; right: 1.5rem; \
                   width: 320px; background: #fff; border-radius: 12px; \
                   border: 1px solid #e5e7eb; \
                   box-shadow: 0 8px 32px rgba(0,0,0,0.12); \
                   z-index: 1000; overflow: hidden; \
                   transition: opacity 0.2s ease, transform 0.2s ease;"
        >
            <div style="display: flex; align-items: center; \
                        justify-content: space-between; \
                        padding: 0.875rem 1rem 0.75rem; \
                        border-bottom: 1px solid #f3f4f6;">
                <span style="font-size: 1rem; font-weight: 600; color: #111827;">
                    "Change Proposal"
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
                // r[impl proposal.create.dismiss]
                <input
                    class="input"
                    type="text"
                    placeholder="Title (optional)"
                    prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))
                />
                {move || {
                    create_action
                        .value()
                        .get()
                        .and_then(|r| r.err())
                        .map(|e| {
                            view! { <p class="help is-danger mt-1">{format!("{e}")}</p> }
                        })
                }}
                <button
                    class="button is-primary is-fullwidth"
                    style="margin-top: 0.625rem;"
                    disabled=move || create_action.pending().get()
                    on:click=move |_| { create_action.dispatch(()); }
                >
                    {move || {
                        if create_action.pending().get() {
                            "Creating…"
                        } else {
                            "Create Proposal"
                        }
                    }}
                </button>
            </div>
        </div>

        // FAB — hidden while the panel is open.
        <button
            style:opacity=move || if panel_open.get() { "0" } else { "1" }
            style:pointer-events=move || if panel_open.get() { "none" } else { "auto" }
            style:background=move || if hovered.get() { "#3254d4" } else { "#485fc7" }
            style="position: fixed; bottom: 1.5rem; right: 1.5rem; \
                   height: 56px; min-width: 56px; border-radius: 28px; \
                   border: none; color: #fff; cursor: pointer; \
                   display: flex; align-items: center; \
                   box-shadow: 0 4px 16px rgba(72,95,199,0.45); \
                   z-index: 1001; overflow: hidden; \
                   transition: background 0.15s ease, box-shadow 0.15s ease, \
                               opacity 0.15s ease;"
            on:mouseenter=move |_| hovered.set(true)
            on:mouseleave=move |_| hovered.set(false)
            on:click=move |_| panel_open.update(|v| *v = !*v)
        >
            // Icon occupies a fixed 56×56 square so it stays centred in the
            // collapsed circle and becomes the left end of the expanded pill.
            <span
                inner_html=EDIT_ICON
                style="display: flex; align-items: center; justify-content: center; \
                       width: 56px; height: 56px; flex-shrink: 0;"
            />
            // Label expands via max-width transition.
            <span
                style:max-width=move || if hovered.get() { "180px" } else { "0px" }
                style:opacity=move || if hovered.get() { "1" } else { "0" }
                style:padding-right=move || if hovered.get() { "1.25rem" } else { "0" }
                style="overflow: hidden; white-space: nowrap; \
                       font-size: 0.875rem; font-weight: 600; \
                       transition: max-width 0.2s ease, opacity 0.15s ease, \
                                   padding-right 0.2s ease;"
            >
                "Propose a Change"
            </span>
        </button>
    }
}
