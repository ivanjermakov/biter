#[macro_use]
extern crate log;

use anyhow::Result;
use std::path::PathBuf;

use crate::{peer::generate_peer_id, torrent::download_torrent};

mod bencode;
mod hex;
mod metainfo;
mod peer;
mod sha1;
mod state;
mod torrent;
mod tracker;
mod types;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let peer_id = generate_peer_id();
    info!("peer id {}", String::from_utf8_lossy(peer_id.as_slice()));

    let path = PathBuf::from("data/academic_test.torrent");
    download_torrent(&path, &peer_id).await
}
