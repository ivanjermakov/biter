use anyhow::{anyhow, ensure, Context, Error, Result};
use futures::future;
use std::path::Path;
use std::time::Instant;
use std::{fs, path::PathBuf, sync::Arc};
use tokio::{spawn, sync::Mutex};

use crate::abort::EnsureAbort;
use crate::config::Config;
use crate::{
    bencode::{parse_bencoded, BencodeValue},
    metainfo::{FileInfo, Metainfo, PathInfo},
    peer::peer_loop,
    sha1,
    state::{init_pieces, Peer, State, TorrentStatus},
    tracker::{tracker_loop, tracker_request, TrackerEvent, TrackerRequest, TrackerResponse},
    types::ByteString,
};

pub async fn download_torrent(path: &Path, peer_id: &ByteString, config: &Config) -> Result<()> {
    let started = Instant::now();
    debug!("reading torrent file: {:?}", path);
    let bencoded = fs::read(path).context("no metadata file")?;
    let metainfo_dict = match parse_bencoded(bencoded) {
        (Some(metadata), left) if left.is_empty() => metadata,
        _ => return Err(Error::msg("metadata file parsing error")),
    };
    debug!("metainfo dict: {metainfo_dict:?}");
    let metainfo = match Metainfo::try_from(metainfo_dict.clone()) {
        Ok(info) => info,
        Err(e) => return Err(Error::msg(e).context("metadata file structure error")),
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
            config.port,
            Some(TrackerEvent::Started),
            None,
        ),
    )
    .await
    .context("request failed")?;
    info!("tracker response: {tracker_response:?}");

    let resp = match tracker_response {
        TrackerResponse::Success(r) => r,
        TrackerResponse::Failure { failure_reason } => return Err(Error::msg(failure_reason)),
    };

    let state = Arc::new(Mutex::new(State {
        config: config.clone(),
        metainfo: metainfo.clone(),
        tracker_response: resp.clone(),
        info_hash,
        peer_id: peer_id.to_vec(),
        pieces: init_pieces(&metainfo.info),
        peers: resp
            .peers
            .into_iter()
            .map(|p| (p.peer_id.clone(), Peer::new(p)))
            .collect(),
        status: TorrentStatus::Started,
    }));
    trace!("init state: {:?}", state);

    let peer_loop_h = spawn(peer_loop(state.clone()));
    let tracker_loop_h = spawn(tracker_loop(state.clone()));
    debug!("connecting to peers");
    peer_loop_h.await??;
    trace!("aborting tracker loop");
    let _ = tracker_loop_h.ensure_abort().await;

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

    info!("writing files to disk");
    write_to_disk(state).await?;

    info!("done in {}s", started.elapsed().as_secs());
    Ok(())
}

async fn write_to_disk(mut state: State) -> Result<()> {
    debug!("partitioning pieces into files");
    let mut data: Vec<u8> = state
        .pieces
        .into_values()
        .flat_map(|p| p.blocks.into_values().flat_map(|b| b.0))
        .collect();

    let files = match state.metainfo.info.file_info {
        FileInfo::Single { length, md5_sum } => vec![PathInfo {
            length,
            path: PathBuf::from(&state.metainfo.info.name),
            md5_sum,
        }],
        FileInfo::Multi { files } => files,
    };

    // TODO: check files md5_sum
    info!("writing files");
    let mut write_handles = vec![];
    for file in files {
        let file_data = data.drain(0..file.length as usize).collect();
        let path = PathBuf::from("download")
            .join(&state.metainfo.info.name)
            .join(file.path);
        write_handles.push(spawn(write_file(path, file_data)))
    }
    let write_res = future::join_all(write_handles).await;
    if write_res.into_iter().filter(|r| r.is_err()).count() != 0 {
        return Err(Error::msg("file write errors"));
    }

    state.status = TorrentStatus::Saved;
    info!("torrent saved: {}", state.metainfo.info.name);
    Ok(())
}

async fn write_file(path: PathBuf, data: Vec<u8>) -> Result<()> {
    debug!("writing file ({} bytes): {:?}", data.len(), path);
    tokio::fs::create_dir_all(&path.parent().context("no parent")?).await?;
    let res = tokio::fs::write(&path, &data).await;
    if let Err(e) = res {
        error!("file write error: {e:#}");
        return Err(anyhow!(e));
    }
    info!("file written ({} bytes): {:?}", data.len(), path);
    Ok::<(), Error>(())
}
