use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, env, default_value = "[::]:10800")]
    listen_address: std::net::SocketAddr,

    #[arg(long, env, default_value = "amqp://localhost")]
    amqp_addr: String,

    #[arg(long, env)]
    nat64_prefix: Option<ipnetwork::Ipv6Network>,

    #[arg(long, env)]
    db_url: String,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    openssl_probe::init_ssl_cert_env_vars();
    let args = Args::parse();

    if !tokio::task::block_in_place(|| {
        kosmos::run_migrations(&args.db_url)
    }) {
        return
    }

    let db_config = diesel_async::pooled_connection::AsyncDieselConnectionManager::<diesel_async::AsyncPgConnection>::new(args.db_url);
    let db_pool = std::sync::Arc::new(mobc::Pool::new(db_config));

    kosmos::mo::receive_mt(args.listen_address, args.amqp_addr, args.nat64_prefix, db_pool).await;
}