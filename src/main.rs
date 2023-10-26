#[macro_use]
extern crate log;

use anyhow::{Error, Result};
use std::{env, path::PathBuf, process, time::Duration};

use crate::{config::Config, peer::generate_peer_id, torrent::download_torrent};

mod abort;
mod bencode;
mod config;
mod hex;
mod metainfo;
mod peer;
mod sha1;
mod state;
mod torrent;
mod tracker;
mod types;

#[tokio::main]
async fn main() {
    if let Err(e) = try_main().await {
        eprintln!("{e:#}");
        process::exit(1);
    }
}

async fn try_main() -> Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let path = match env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        _ => return Err(Error::msg("no torrent file specified")),
    };

    let peer_id = generate_peer_id();
    info!("peer id {}", String::from_utf8_lossy(peer_id.as_slice()));

    let config = Config {
        port: 6881,
        respect_choke: true,
        choke_wait: Duration::from_millis(100),
        reconnect_wait: Duration::from_secs(10),
        downloaded_check_wait: Duration::from_secs(1),
        peer_connect_timeout: Duration::from_secs(4),
        piece_request_wait: Duration::from_millis(100),
    };

    download_torrent(&path, &peer_id, &config).await
}
