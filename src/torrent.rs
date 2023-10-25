use anyhow::{anyhow, ensure, Context, Error, Result};
use futures::future;
use std::path::Path;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};
use tokio::{spawn, sync::Mutex};

use crate::{
    bencode::{parse_bencoded, BencodeValue},
    metainfo::{FileInfo, Metainfo, PathInfo},
    peer::handle_peer,
    sha1,
    state::{init_pieces, State},
    tracker::{tracker_request, TrackerEvent, TrackerRequest, TrackerResponse},
    types::ByteString,
};

pub async fn download_torrent(path: &Path, peer_id: &ByteString) -> Result<()> {
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
        BencodeValue::Dict(d) => d.get("info").context("no 'info' key")?.encode(),
        _ => unreachable!(),
    };
    let info_hash = sha1::encode(info_dict_str);
    let tracker_response = tracker_request(
        metainfo.announce.clone(),
        TrackerRequest::new(
            info_hash.clone(),
            peer_id.to_vec(),
            TrackerEvent::Started,
            None,
        ),
    )
    .await
    .context("request failed")?;
    info!("tracker response: {tracker_response:?}");

    let state = Arc::new(Mutex::new(State {
        metainfo: metainfo.clone(),
        info_hash,
        peer_id: peer_id.to_vec(),
        pieces: init_pieces(&metainfo.info),
        peers: BTreeMap::new(),
    }));

    let resp = match tracker_response {
        TrackerResponse::Success(r) => r,
        TrackerResponse::Failure { failure_reason } => return Err(Error::msg(failure_reason)),
    };
    future::join_all(
        resp.peers
            .into_iter()
            .map(|p| spawn(handle_peer(p, state.clone())))
            .collect::<Vec<_>>(),
    )
    .await;

    trace!("unwrapping state");
    let state = Arc::try_unwrap(state)
        .map_err(|_| Error::msg("dangling state reference"))?
        .into_inner();

    debug!("verifying downloaded pieces");
    ensure!(
        state.pieces.len() == state.metainfo.info.pieces.len(),
        "pieces length mismatch"
    );
    ensure!(
        state.pieces.values().all(|p| p.completed),
        "incomplete pieces"
    );

    write_to_disk(state, metainfo).await?;
    Ok(())
}

async fn write_to_disk(state: State, metainfo: Metainfo) -> Result<()> {
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
    let write_res = future::join_all(write_handles).await;
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
