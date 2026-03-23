use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Contributor information for display in avatar lists and change attribution.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AvatarInfo {
    pub email: String,
    pub name: Option<String>,
    /// Pre-computed avatar URL (Tailscale profile pic or Gravatar with d=404).
    pub avatar_url: String,
    /// Loro peer IDs used by this user within the relevant proposal.
    pub peer_ids: Vec<u64>,
}

impl AvatarInfo {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.email)
    }
}

/// Compute the canonical avatar URL for a user. Prefers the Tailscale profile
/// pic when present; falls back to a Gravatar URL with `d=404` so the client
/// can detect a missing gravatar and render initials instead.
#[cfg(feature = "ssr")]
pub fn compute_avatar_url(email: &str, profile_pic_url: Option<&str>) -> String {
    if let Some(url) = profile_pic_url {
        if !url.is_empty() {
            return url.to_string();
        }
    }
    gravatar_url(email, 80)
}

#[cfg(feature = "ssr")]
pub fn gravatar_url(email: &str, size: u32) -> String {
    let hash = md5::compute(email.trim().to_lowercase().as_bytes());
    format!("https://www.gravatar.com/avatar/{hash:x}?s={size}&d=404")
}

/// Deterministic background colour derived from the email string.
fn avatar_bg(email: &str) -> &'static str {
    const COLOURS: &[&str] = &[
        "#6366f1", "#8b5cf6", "#ec4899", "#f43f5e", "#f97316", "#eab308", "#22c55e", "#14b8a6",
        "#06b6d4", "#3b82f6",
    ];
    let hash = email.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    COLOURS[hash % COLOURS.len()]
}

fn initials(display: &str) -> String {
    display
        .split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

/// A fully-rounded avatar that shows the user's profile picture with a tooltip.
/// Falls back to coloured initials when the image URL yields a 404 or fails.
#[component]
pub fn Avatar(
    info: AvatarInfo,
    /// Diameter in pixels. Defaults to 32.
    #[prop(default = 32u32)]
    size: u32,
) -> impl IntoView {
    let show_img = RwSignal::new(true);

    let display = info.display_name().to_string();
    let bg = avatar_bg(&info.email);
    let initials_str = initials(&display);
    let avatar_url = info.avatar_url.clone();
    let font_size = ((size as f32) * 0.38).round() as u32;

    let outer_style = format!(
        "width: {size}px; height: {size}px; border-radius: 50%; \
         display: inline-flex; align-items: center; justify-content: center; \
         flex-shrink: 0; overflow: hidden; background: {bg}; \
         font-size: {font_size}px; font-weight: 600; color: #fff; \
         cursor: default; vertical-align: middle;"
    );
    let img_style = format!(
        "width: {size}px; height: {size}px; border-radius: 50%; \
         object-fit: cover; display: block;"
    );

    view! {
        <span title=display style=outer_style>
            <Show
                when=move || show_img.get()
                fallback=move || {
                    view! {
                        <span style="line-height: 1; user-select: none;">
                            {initials_str.clone()}
                        </span>
                    }
                }
            >
                <img
                    src=avatar_url.clone()
                    alt=""
                    style=img_style.clone()
                    on:error=move |_| show_img.set(false)
                />
            </Show>
        </span>
    }
}
