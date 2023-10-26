#[macro_use]
extern crate log;

use anyhow::{Error, Result};
use std::{env, path::PathBuf, process};

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
mod abort;

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

    download_torrent(&path, &peer_id).await
}
