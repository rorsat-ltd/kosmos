#[macro_use]
extern crate log;
extern crate core;

use diesel_migrations::MigrationHarness;

mod ie;
mod schema;
mod models;
pub mod mo;
pub mod worker;
mod types;
pub mod http;

pub const IRIDUM_MT_ADDR: &'static str = "directip.sbd.iridium.com:10800";
pub const IRIDIUM_SOURCE_IP: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::new(12, 47, 179, 11));
pub const MIGRATIONS: diesel_migrations::EmbeddedMigrations = diesel_migrations::embed_migrations!("./migrations");

type DBPool = std::sync::Arc<diesel_async::pooled_connection::mobc::Pool<diesel_async::AsyncPgConnection>>;
type DBConn = mobc::Connection<diesel_async::pooled_connection::AsyncDieselConnectionManager<diesel_async::AsyncPgConnection>>;

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

pub fn run_migrations(db_url: &str) -> bool {
    use diesel::prelude::Connection;

    let mut db_conn = match diesel_async::async_connection_wrapper::AsyncConnectionWrapper::<
        diesel_async::pg::AsyncPgConnection
    >::establish(db_url) {
        Ok(c) => c,
        Err(err) => {
            error!("Failed to connect to database: {}", err);
            return false
        }
    };

    if let Err(err) = db_conn.run_pending_migrations(MIGRATIONS) {
        error!("Failed to run migrations: {}", err);
        return false;
    }

    true
}



