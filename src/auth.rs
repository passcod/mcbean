use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TailscaleUser {
    // r[impl users.identity]
    pub email: String,
    pub name: Option<String>,
}

#[cfg(feature = "ssr")]
const TAILSCALE_USER_LOGIN: &str = "Tailscale-User-Login";
#[cfg(feature = "ssr")]
const TAILSCALE_USER_NAME: &str = "Tailscale-User-Name";

#[cfg(feature = "ssr")]
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for TailscaleUser {
    type Rejection = (axum::http::StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        // r[impl users.identity]
        let email = parts
            .headers
            .get(TAILSCALE_USER_LOGIN)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or((
                axum::http::StatusCode::UNAUTHORIZED,
                "missing Tailscale-User-Login header",
            ))?;

        let name = parts
            .headers
            .get(TAILSCALE_USER_NAME)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        Ok(TailscaleUser { email, name })
    }
}

#[cfg(feature = "ssr")]
pub async fn get_current_user() -> Result<TailscaleUser, leptos::prelude::ServerFnError> {
    leptos_axum::extract::<TailscaleUser>()
        .await
        .map_err(|e| leptos::prelude::ServerFnError::new(e.to_string()))
}
