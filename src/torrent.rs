use anyhow::{ensure, Context, Error, Result};
use std::collections::BTreeSet;
use std::io::SeekFrom;
use std::path::Path;
use std::time::Instant;
use std::{fs, path::PathBuf, sync::Arc};
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::{spawn, sync::Mutex};

use crate::{
    abort::EnsureAbort,
    bencode::{parse_bencoded, BencodeValue},
    config::Config,
    dht::find_peers,
    metainfo::Metainfo,
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

    let (dht_peers, peer_id) = {
        let p_state = p_state.lock().await;
        (
            p_state.dht_peers.iter().cloned().collect(),
            p_state.peer_id.clone(),
        )
    };
    let peers = find_peers(dht_peers, peer_id.clone(), info_hash.to_vec(), 50, config.dht_chunk).await?;
    info!("discovered {} dht peers", peers.len());

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
        Ok(TrackerResponse::Success(mut r)) => {
            for p in peers {
                r.peers.insert(p);
            }
            r
        }
        e => {
            debug!("tracker error: {:?}", e);
            TrackerResponseSuccess {
                // set peers discovered via DHT
                peers,
                // should never poll again, rely on DHT
                interval: i64::MAX,
                ..Default::default()
            }
        }
    };

    let state = State {
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
    };
    let state = Arc::new(Mutex::new(state));
    trace!("init state: {:?}", state);

    let peer_loop_h = spawn(peer_loop(state.clone()));
    // TODO: DHT discover loop
    let tracker_loop_h = spawn(tracker_loop(state.clone()));
    debug!("connecting to peers");
    peer_loop_h.await??;
    trace!("aborting tracker loop");
    let _ = tracker_loop_h.ensure_abort().await;

    let state = state.lock().await;
    debug!("verifying downloaded pieces");
    ensure!(
        state.pieces.len() == state.metainfo.info.pieces.len(),
        "pieces length mismatch"
    );
    let incomplete = state
        .pieces
        .values()
        .filter(|p| p.status != TorrentStatus::Saved)
        .count();
    if incomplete > 0 {
        return Err(Error::msg(format!("{} incomplete pieces", incomplete)));
    }

    let mut dht_peers: BTreeSet<PeerInfo> = state
        .peers
        .values()
        .filter(|p| p.dht_port.is_some())
        .map(|p| PeerInfo {
            ip: p.info.ip.clone(),
            port: p.dht_port.unwrap(),
        })
        .collect();
    debug!("discovered {} dht nodes: {:?}", dht_peers.len(), dht_peers);
    p_state.lock().await.dht_peers.append(&mut dht_peers);

    info!("done in {}s", started.elapsed().as_secs());
    Ok(())
}

// TODO: initialize every file with `.part` suffix
// if every file piece is written, remove suffix from the filename
pub async fn write_piece(piece_idx: u32, state: Arc<Mutex<State>>) -> Result<()> {
    let metainfo = {
        let state = state.lock().await;
        state.metainfo.clone()
    };
    // TODO: drain data instead of cloning
    let piece = { state.lock().await.pieces.get(&piece_idx).cloned().unwrap() };
    debug!("writing piece: {:?}", piece.file_locations);
    for f in piece.file_locations {
        let path = PathBuf::from("download")
            .join(&metainfo.info.name)
            .join(metainfo.info.file_info.files()[f.file_index].path.clone());
        tokio::fs::create_dir_all(&path.parent().context("no parent")?).await?;
        let data = piece
            .blocks
            .values()
            .flat_map(|b| b.0.clone())
            .skip(f.piece_offset)
            .take(f.length)
            .collect::<Vec<_>>();
        ensure!(data.len() == f.length);
        trace!(
            "witing {} bytes at {} of {}",
            data.len(),
            f.offset,
            path.display()
        );
        let mut file = File::options().create(true).write(true).open(path).await?;
        file.seek(SeekFrom::Start(f.offset as u64)).await?;
        file.write_all(&data).await?;

        let mut state = state.lock().await;
        let p = state.pieces.get_mut(&piece_idx).unwrap();
        p.status = TorrentStatus::Saved;
        p.blocks.clear();
    }
    Ok(())
}
