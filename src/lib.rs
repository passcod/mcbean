#![recursion_limit = "256"]

pub mod app;
pub mod auth;
pub mod components;
pub mod db;
#[cfg(feature = "ssr")]
pub mod github;
pub mod pages;
#[cfg(feature = "ssr")]
pub mod server;
#[cfg(feature = "ssr")]
pub mod sync;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
