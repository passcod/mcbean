#[cfg(feature = "ssr")]
pub mod models;
#[cfg(feature = "ssr")]
pub mod schema;

#[cfg(feature = "ssr")]
pub type DbPool = deadpool_diesel::postgres::Pool;

#[cfg(feature = "ssr")]
pub fn create_pool(database_url: &str) -> DbPool {
    let manager =
        deadpool_diesel::postgres::Manager::new(database_url, deadpool_diesel::Runtime::Tokio1);
    deadpool_diesel::postgres::Pool::builder(manager)
        .build()
        .expect("failed to create database pool")
}
