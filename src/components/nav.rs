use leptos::prelude::*;

use crate::components::avatar::{Avatar, AvatarInfo};

#[server]
pub async fn get_user_info() -> Result<AvatarInfo, ServerFnError> {
    use crate::auth::get_current_user;
    use crate::components::avatar::compute_avatar_url;

    let user = get_current_user().await?;
    let avatar_url = compute_avatar_url(&user.email, user.profile_pic_url.as_deref());

    Ok(AvatarInfo {
        email: user.email,
        name: user.name,
        avatar_url,
        peer_ids: vec![],
    })
}

#[component]
pub fn Nav() -> impl IntoView {
    let user_info = Resource::new(|| (), |_| get_user_info());

    view! {
        <nav class="navbar is-dark" role="navigation" aria-label="main navigation">
            <div class="navbar-brand">
                <a class="navbar-item" href="/">
                    <img
                        src="/logo.png"
                        alt="McBean"
                        height="28"
                        style="height: 28px; width: 28px; margin-right: 0.5rem; border-radius: 4px;"
                    />
                    <span class="has-text-weight-bold">"McBean"</span>
                </a>
            </div>
            <div class="navbar-menu">
                <div class="navbar-start">
                    <a class="navbar-item" href="/">"Repositories"</a>
                </div>
                <div class="navbar-end">
                    <div class="navbar-item">
                        <Suspense fallback=move || view! { <span>"Loading..."</span> }>
                            {move || Suspend::new(async move {
                                match user_info.await {
                                    // r[impl users.identity]
                                    Ok(info) => {
                                        let email = info.email.clone();
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 0.5rem;">
                                                <Avatar info=info size=28 />
                                                <span class="has-text-light">{email}</span>
                                            </div>
                                        }
                                        .into_any()
                                    }
                                    Err(_) => view! {
                                        <span class="has-text-grey-light">"Not signed in"</span>
                                    }
                                    .into_any(),
                                }
                            })}
                        </Suspense>
                    </div>
                </div>
            </div>
        </nav>
    }
}
