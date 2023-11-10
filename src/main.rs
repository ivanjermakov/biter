#![allow(clippy::format_collect)]

#[macro_use]
extern crate log;

use anyhow::{Context, Error, Result};
use expanduser::expanduser;
use reqwest::Url;
use std::{collections::BTreeSet, env, path::PathBuf, process, sync::Arc, time::Duration};
use tokio::sync::Mutex;

use crate::{
    config::Config,
    hex::from_hex,
    peer::generate_peer_id,
    persist::PersistState,
    torrent::{download_torrent, metainfo_from_path},
};

mod abort;
mod bencode;
mod config;
mod dht;
mod extension;
mod feature;
mod hex;
mod message;
mod metainfo;
mod peer;
mod peer_metainfo;
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
    env_logger::init_from_env(env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"));

    let arg = match env::args().nth(1) {
        Some(arg) => arg,
        _ => return Err(Error::msg("no torrent file/magnet specified")),
    };

    let config = Config {
        port: 6881,
        respect_choke: false,
        choke_wait: Duration::from_secs(10),
        reconnect_wait: Duration::from_secs(20),
        downloaded_check_wait: Duration::from_secs(1),
        peer_connect_timeout: Duration::from_secs(4),
        piece_request_wait: Duration::from_millis(100),
        dht_chunk: 200,
        dht_min_peers: 50,
    };

    let state_path = expanduser("~/.local/state/biter")?;
    let p_state = PersistState::load(&state_path).ok().unwrap_or_else(|| PersistState {
        path: state_path,
        peer_id: generate_peer_id(),
        dht_peers: BTreeSet::new(),
    });
    debug!("read persist state from file: {:?}", p_state);
    let p_state = Arc::new(Mutex::new(p_state));

    if arg.starts_with("magnet:") {
        debug!("parsing magnet: {}", arg);
        let uri = Url::parse(&arg).context("magnet uri parsing error")?;
        let xt = uri
            .query_pairs()
            .find(|(k, _)| k == "xt")
            .context("no `info_hash` query param")?
            .1
            .to_string();
        trace!("xt: {}", xt);
        let info_hash = xt.split("urn:btih:").last().context("invalid magnet")?.to_lowercase();
        info!("magnet info hash: {}", info_hash);
        download_torrent(from_hex(&info_hash), None, &config, p_state).await?;
    } else {
        let (info_hash, metainfo) = metainfo_from_path(&PathBuf::from(arg))?;
        download_torrent(info_hash, Some(metainfo), &config, p_state).await?;
    }

    Ok(())
}
