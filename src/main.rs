#[macro_use]
extern crate log;

use anyhow::Result;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::path::PathBuf;
use torrent::download_torrent;

use types::ByteString;

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

/// Generate random 20 byte string, starting with -<2 byte client name><4 byte client version>-
fn generate_peer_id() -> ByteString {
    let rand = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .collect::<Vec<_>>();
    ["-ER0000-".as_bytes(), &rand].concat()
}
