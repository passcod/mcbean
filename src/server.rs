use std::net::SocketAddr;

use axum::Router;
use leptos::config::get_configuration;
use leptos::prelude::*;
use leptos_axum::{LeptosRoutes, generate_route_list};
use leptos_meta::*;

use crate::app::App;
use crate::db::{DbPool, create_pool};

#[derive(Clone, Debug)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub pool: DbPool,
}

impl axum::extract::FromRef<AppState> for LeptosOptions {
    fn from_ref(state: &AppState) -> Self {
        state.leptos_options.clone()
    }
}

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <Stylesheet href="/pkg/mcbean.css" />
                <Stylesheet href="https://cdn.jsdelivr.net/npm/bulma@1.0.4/css/bulma.min.css" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

#[tokio::main]
pub async fn run() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = create_pool(&database_url);

    let conf = get_configuration(None).unwrap();
    let leptos_options = conf.leptos_options;
    let addr: SocketAddr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    let state = AppState {
        leptos_options: leptos_options.clone(),
        pool: pool.clone(),
    };

    let app = Router::new()
        .leptos_routes_with_context(
            &state,
            routes,
            {
                let pool = pool.clone();
                move || {
                    provide_context(pool.clone());
                }
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler::<AppState, _>(shell))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
