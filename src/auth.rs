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
const DEV_USER_EMAIL_ENV: &str = "DEV_USER_EMAIL";

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
            .or_else(|| std::env::var(DEV_USER_EMAIL_ENV).ok())
            .ok_or((
                axum::http::StatusCode::UNAUTHORIZED,
                "missing Tailscale-User-Login header and DEV_USER_EMAIL is not set",
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

/// Look up the current user in the database by email, creating a row if one
/// does not yet exist. Returns the user's database ID.
#[cfg(feature = "ssr")]
pub async fn get_or_create_user_id() -> Result<i32, leptos::prelude::ServerFnError> {
    use diesel::prelude::*;

    let user = get_current_user().await?;

    let pool = leptos::prelude::use_context::<crate::db::DbPool>()
        .ok_or_else(|| leptos::prelude::ServerFnError::new("No database pool"))?;
    let conn = pool
        .get()
        .await
        .map_err(|e| leptos::prelude::ServerFnError::new(format!("{e}")))?;

    conn.interact(move |conn| {
        use crate::db::schema::users;

        let existing: Option<i32> = users::table
            .filter(users::email.eq(&user.email))
            .select(users::id)
            .first(conn)
            .optional()?;

        if let Some(id) = existing {
            return Ok(id);
        }

        diesel::insert_into(users::table)
            .values((
                users::email.eq(&user.email),
                users::display_name.eq(user.name.as_deref()),
            ))
            .returning(users::id)
            .get_result::<i32>(conn)
    })
    .await
    .map_err(|e| leptos::prelude::ServerFnError::new(format!("interact error: {e}")))?
    .map_err(|e: diesel::result::Error| {
        leptos::prelude::ServerFnError::new(format!("query error: {e}"))
    })
}
