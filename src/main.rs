#[macro_use]
extern crate log;

use anyhow::{anyhow, ensure, Context, Error, Result};
use futures::future::join_all;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};
use tokio::{spawn, sync::Mutex};

use bencode::parse_bencoded;
use types::ByteString;

use crate::{
    metainfo::{FileInfo, Metainfo, PathInfo},
    peer::handle_peer,
    state::{init_pieces, State},
    tracker::{tracker_request, TrackerRequest, TrackerResponse},
};

mod bencode;
mod hex;
mod metainfo;
mod peer;
mod sha1;
mod state;
mod tracker;
mod types;

#[tokio::main]
async fn main() -> Result<()> {
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
        metainfo.announce.clone(),
        TrackerRequest::new(
            info_hash.clone(),
            peer_id.clone(),
            tracker::TrackerEvent::Started,
            None,
        ),
    )
    .await
    .context("request failed")?;
    info!("tracker response: {tracker_response:?}");

    let state = Arc::new(Mutex::new(State {
        metainfo: metainfo.clone(),
        info_hash,
        peer_id,
        pieces: init_pieces(&metainfo.info),
        peers: BTreeMap::new(),
    }));

    if let TrackerResponse::Success(resp) = tracker_response {
        let handles = resp
            .peers
            .into_iter()
            .map(|p| {
                spawn({
                    let state = state.clone();
                    handle_peer(p, state)
                })
            })
            .collect::<Vec<_>>();
        join_all(handles).await;
    }

    debug!("verifying downloaded pieces");
    let state = Arc::try_unwrap(state)
        .map_err(|_| Error::msg("dangling state reference"))?
        .into_inner();
    ensure!(
        state.pieces.len() == state.metainfo.info.pieces.len(),
        "pieces length mismatch"
    );
    ensure!(
        state.pieces.values().all(|p| p.completed),
        "incomplete pieces"
    );

    info!("partitioning pieces into files");
    let mut data: Vec<u8> = state
        .pieces
        .into_values()
        .flat_map(|p| p.blocks.into_values().flat_map(|b| b.0))
        .collect();

    let files = match metainfo.info.file_info {
        FileInfo::Single { length, md5_sum } => vec![PathInfo {
            length,
            path: PathBuf::from(&metainfo.info.name),
            md5_sum,
        }],
        FileInfo::Multi { files } => files,
    };

    info!("writing files");
    let mut write_handles = vec![];
    for file in files {
        let file_data = data.drain(0..file.length as usize).collect();
        let path = PathBuf::from("download")
            .join(&metainfo.info.name)
            .join(file.path.clone());
        write_handles.push(spawn(write_file(path, file_data)))
    }

    let write_res = join_all(write_handles).await;
    if write_res.into_iter().filter(|r| r.is_err()).count() != 0 {
        return Err(Error::msg("file write errors"));
    }

    info!("torrent downloaded: {}", metainfo.info.name);
    Ok(())
}

async fn write_file(path: PathBuf, data: Vec<u8>) -> Result<()> {
    debug!("writing file ({} bytes): {:?}", data.len(), path);
    tokio::fs::create_dir_all(&path.parent().context("no parent")?).await?;
    let res = tokio::fs::write(&path, &data).await;
    if let Err(e) = res {
        error!("file write error: {e}");
        return Err(anyhow!(e));
    }
    info!("file written ({} bytes): {:?}", data.len(), path);
    Ok::<(), Error>(())
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
