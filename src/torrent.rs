use anyhow::{anyhow, ensure, Context, Error, Result};
use futures::future;
use futures::stream::FuturesUnordered;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;
use std::{fs, path::PathBuf, sync::Arc};
use tokio::{spawn, sync::Mutex};

use crate::{
    abort::EnsureAbort,
    bencode::{parse_bencoded, BencodeValue},
    config::Config,
    dht::find_peers,
    metainfo::{FileInfo, Metainfo},
    peer::peer_loop,
    persist::PersistState,
    sha1,
    state::{init_pieces, Peer, PeerInfo, State, TorrentStatus},
    tracker::{
        tracker_loop, tracker_request, TrackerEvent, TrackerRequest, TrackerResponse,
        TrackerResponseSuccess,
    },
};

pub async fn download_torrent(
    path: &Path,
    config: &Config,
    p_state: Arc<Mutex<PersistState>>,
) -> Result<()> {
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

    let peers = discover_peers(p_state.clone(), &info_hash).await?;

    let tracker_response = tracker_request(
        metainfo.announce.clone(),
        TrackerRequest::new(
            info_hash.clone(),
            p_state.lock().await.peer_id.to_vec(),
            config.port,
            Some(TrackerEvent::Started),
            None,
        ),
    )
    .await
    .context("request failed");
    info!("tracker response: {tracker_response:?}");

    let resp = match tracker_response {
        Ok(TrackerResponse::Success(r)) => r,
        e => {
            debug!("tracker error: {:?}", e);
            TrackerResponseSuccess {
                // set peers discovered via DHT
                peers,
                // should never poll again, use rely on DHT
                interval: i64::MAX,
                ..Default::default()
            }
        }
    };

    let state = Arc::new(Mutex::new(State {
        config: config.clone(),
        metainfo: metainfo.clone(),
        tracker_response: resp.clone(),
        info_hash,
        peer_id: p_state.lock().await.peer_id.to_vec(),
        pieces: init_pieces(&metainfo.info),
        peers: resp
            .peers
            .into_iter()
            .map(|p| (p.clone(), Peer::new(p)))
            .collect(),
        status: TorrentStatus::Started,
    }));
    trace!("init state: {:?}", state);

    let peer_loop_h = spawn(peer_loop(state.clone()));
    // TODO: DHT discover loop
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

    let mut dht_peers: BTreeSet<PeerInfo> = state
        .peers
        .values()
        .filter(|p| p.dht_port.is_some())
        .map(|p| PeerInfo {
            ip: p.info.ip.clone(),
            port: p.dht_port.unwrap(),
        })
        .collect();
    debug!("discovered {} dht peers: {:?}", dht_peers.len(), dht_peers);
    p_state.lock().await.dht_peers.append(&mut dht_peers);

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
        FileInfo::Single(file) => vec![file],
        FileInfo::Multi(files) => files,
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

async fn discover_peers(
    p_state: Arc<Mutex<PersistState>>,
    info_hash: &[u8],
) -> Result<BTreeSet<PeerInfo>> {
    let (dht_peers, peer_id) = {
        let p_state = p_state.lock().await;
        (p_state.dht_peers.clone(), p_state.peer_id.clone())
    };
    let mut peers = BTreeSet::new();
    // TODO: make configurable
    let min_p = 50;

    let handles = dht_peers
        .into_iter()
        .map(|p| spawn(find_peers(p, peer_id.clone(), info_hash.to_vec(), min_p)))
        .collect::<FuturesUnordered<_>>();
    for h in handles {
        match h.await {
            Ok(Ok(ps)) => {
                for p in ps {
                    peers.insert(p);
                }
                if peers.len() >= min_p {
                    break;
                }
            }
            e => {
                trace!("dht error: {:?}", e);
            }
        }
    }
    info!("discovered {} peers: {:?}", peers.len(), peers);
    Ok(peers)
}
