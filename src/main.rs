#[macro_use]
extern crate log;

use anyhow::{Error, Result};
use expanduser::expanduser;
use std::{collections::BTreeSet, env, path::PathBuf, process, sync::Arc, time::Duration};
use tokio::sync::Mutex;

use crate::{
    config::Config, peer::generate_peer_id, persist::PersistState, torrent::download_torrent,
};

mod abort;
mod bencode;
mod config;
mod dht;
mod hex;
mod metainfo;
mod peer;
mod persist;
mod sha1;
mod state;
mod torrent;
mod tracker;
mod tracker_udp;
mod types;
mod udp;

#[tokio::main]
async fn main() {
    if let Err(e) = try_main().await {
        error!("{e:#}");
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

    let config = Config {
        port: 6881,
        respect_choke: false,
        choke_wait: Duration::from_millis(100),
        reconnect_wait: Duration::from_secs(10),
        downloaded_check_wait: Duration::from_secs(1),
        peer_connect_timeout: Duration::from_secs(4),
        piece_request_wait: Duration::from_millis(100),
    };

    let state_path = expanduser("~/.local/state/biter")?;
    let p_state = PersistState::load(&state_path)
        .ok()
        .unwrap_or_else(|| PersistState {
            path: state_path,
            peer_id: generate_peer_id(),
            dht_peers: BTreeSet::new(),
        });
    debug!("read persist state from file: {:?}", p_state);
    let p_state = Arc::new(Mutex::new(p_state));

    download_torrent(&path, &config, p_state).await?;

    Ok(())
}
