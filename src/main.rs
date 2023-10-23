#[macro_use]
extern crate log;

use anyhow::{Context, Result};
use peer::handshake;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::{fs, path::PathBuf};

use bencode::parse_bencoded;
use types::ByteString;

use crate::{
    metainfo::Metainfo,
    peer::read_message,
    tracker::{tracker_request, TrackerRequest, TrackerResponse},
};

mod bencode;
mod hex;
mod metainfo;
mod peer;
mod sha1;
mod tracker;
mod types;

fn main() -> Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let path = PathBuf::from("data/knoppix.torrent");
    let bencoded = fs::read(path).context("no metadata file")?;
    let metainfo_dict = match parse_bencoded(bencoded) {
        (Some(metadata), left) if left.is_empty() => metadata,
        _ => panic!("metadata file parsing error"),
    };
    debug!("metainfo dict: {metainfo_dict:?}");
    let metainfo = match Metainfo::try_from(metainfo_dict.clone()) {
        Ok(info) => info,
        Err(e) => panic!("metadata file structure error: {e}"),
    };
    info!("metainfo: {metainfo:?}");
    let info_dict_str = match metainfo_dict {
        bencode::BencodeValue::Dict(d) => d.get("info").context("no 'info' key")?.encode(),
        _ => unreachable!(),
    };
    let info_hash = sha1::encode(info_dict_str);
    let peer_id = generate_peer_id();
    info!("peer id {}", String::from_utf8_lossy(peer_id.as_slice()));
    let tracker_response = tracker_request(
        metainfo.announce,
        TrackerRequest::new(
            info_hash.clone(),
            peer_id.clone(),
            tracker::TrackerEvent::Started,
            None,
        ),
    )
    .context("request failed")?;
    info!("tracker response: {tracker_response:?}");

    if let TrackerResponse::Success(resp) = tracker_response {
        for p in resp.peers {
            match handshake(&p, &info_hash, &peer_id) {
                Ok(stream) => {
                    info!("successfull handshake with peer {:?}", p);
                    loop {
                        match read_message(&stream) {
                            Ok(msg) => {
                                debug!("message from peer {:?}: {:?}", p, msg);
                            }
                            Err(e) => {
                                warn!("{}", e);
                                break;
                            }
                        };
                    }
                }
                Err(e) => warn!("handshake error: {}", e),
            }
        }
    }
    Ok(())
}

/// Generate random 20 byte string, starting with -<2 byte client name><4 byte client version>-
fn generate_peer_id() -> ByteString {
    let rand = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .collect::<Vec<_>>();
    vec!["-ER0000-".as_bytes(), &rand]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}
