use anyhow::{ensure, Context, Error, Result};
use std::collections::BTreeSet;
use std::io::SeekFrom;
use std::path::Path;
use std::time::Instant;
use std::{fs, path::PathBuf, sync::Arc};
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::{spawn, sync::Mutex};

use crate::types::ByteString;
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
    tracker::tracker_loop,
};

pub fn get_info_hash(value: &BencodeValue) -> Result<ByteString> {
    if let BencodeValue::Dict(d) = value {
        let str = d.get("info").context("no 'info' key")?.encode();
        Ok(sha1::encode(str))
    } else {
        Err(Error::msg("value is not a dict"))
    }
}

pub async fn download_torrent(
    path: &Path,
    config: &Config,
    p_state: Arc<Mutex<PersistState>>,
) -> Result<()> {
    let started = Instant::now();
    let (metainfo, info_hash) = metainfo_from_path(path)?;
    let (dht_peers, peer_id) = {
        let p_state = p_state.lock().await;
        (
            p_state.dht_peers.iter().cloned().collect(),
            p_state.peer_id.clone(),
        )
    };
    let peers = find_peers(
        dht_peers,
        peer_id.clone(),
        info_hash.to_vec(),
        config.dht_min_peers,
        config.dht_chunk,
    )
    .await?;
    info!("discovered {} dht peers", peers.len());

    let pieces = init_pieces(&metainfo.info);
    let state = State {
        config: config.clone(),
        metainfo: Some(metainfo),
        tracker_response: None,
        info_hash,
        peer_id: p_state.lock().await.peer_id.to_vec(),
        pieces: Some(pieces),
        peers: peers
            .into_iter()
            .map(|p| (p.clone(), Peer::new(p)))
            .collect(),
        status: TorrentStatus::Downloading,
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
        state.pieces.as_ref().unwrap().len() == state.metainfo.as_ref().unwrap().info.pieces.len(),
        "pieces length mismatch"
    );
    let incomplete = state
        .pieces
        .as_ref()
        .unwrap()
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
    let piece = {
        state
            .lock()
            .await
            .pieces
            .as_ref()
            .unwrap()
            .get(&piece_idx)
            .cloned()
            .unwrap()
    };
    debug!("writing piece: {:?}", piece.file_locations);
    for f in piece.file_locations {
        let path = PathBuf::from("download")
            .join(&metainfo.as_ref().unwrap().info.name)
            .join(
                metainfo.as_ref().unwrap().info.file_info.files()[f.file_index]
                    .path
                    .clone(),
            );
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
        let p = state.pieces.as_mut().unwrap().get_mut(&piece_idx).unwrap();
        p.status = TorrentStatus::Saved;
        p.blocks.clear();
    }
    Ok(())
}

pub fn metainfo_from_path(path: &Path) -> Result<(Metainfo, ByteString)> {
    debug!("reading torrent file: {:?}", path);
    let bencoded = fs::read(path).context("no metadata file")?;
    metainfo_from_str(bencoded)
}

pub fn metainfo_from_str(bencoded: ByteString) -> Result<(Metainfo, ByteString)> {
    let metainfo_dict = match parse_bencoded(bencoded) {
        (Some(metadata), left) if left.is_empty() => metadata,
        _ => return Err(Error::msg("metadata file parsing error")),
    };
    debug!("metainfo dict: {metainfo_dict:?}");
    let info_hash = get_info_hash(&metainfo_dict)?;
    info!("info hash: {info_hash:?}");
    let metainfo = match Metainfo::try_from(metainfo_dict) {
        Ok(info) => info,
        Err(e) => return Err(Error::msg(e).context("metadata file structure error")),
    };
    info!("metainfo: {metainfo:?}");
    Ok((metainfo, info_hash))
}
