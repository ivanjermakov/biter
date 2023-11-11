use anyhow::{anyhow, ensure, Context, Result};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    select, spawn,
    sync::Mutex,
    time::{sleep, timeout},
};

use crate::{
    bencode::{parse_bencoded, BencodeValue},
    extension::Extension,
    feature::Feature,
    hex::hex,
    message::{read_message, Message},
    metainfo::Metainfo,
    peer_metainfo::{PeerMetainfoMessage, METAINFO_PIECE_SIZE},
    sha1,
    state::{init_pieces, Block, Peer, PeerInfo, PeerStatus, Piece, State, TorrentStatus, BLOCK_SIZE},
    torrent::write_piece,
    types::ByteString,
};

/// Generate random 20 byte string, starting with -<2 byte client name><4 byte client version>-
pub fn generate_peer_id() -> ByteString {
    let rand = thread_rng().sample_iter(&Alphanumeric).take(12).collect::<Vec<_>>();
    ["-ER0000-".as_bytes(), &rand].concat()
}

pub async fn handshake(peer: &PeerInfo, state: Arc<Mutex<State>>) -> Result<(TcpStream, Message)> {
    let (info_hash, peer_id, peer_connect_timeout) = {
        let state = state.lock().await;
        (
            state.info_hash.clone(),
            state.peer_id.clone(),
            state.config.peer_connect_timeout,
        )
    };
    let mut stream = timeout(peer_connect_timeout, TcpStream::connect(peer.to_addr())).await??;

    let handshake: Vec<u8> = Message::Handshake {
        info_hash: info_hash.clone(),
        peer_id: peer_id.clone(),
        reserved: Feature::new_with(&[Feature::Dht, Feature::Extension]),
    }
    .into();

    trace!("writing handshake {}", hex(&handshake.to_vec()));
    stream.write_all(&handshake).await.context("write error")?;
    stream.flush().await?;

    let mut read_packet = [0; 68];
    trace!("reading handshake");
    stream.read_exact(&mut read_packet).await.context("read error")?;
    let msg: Vec<u8> = read_packet.to_vec();
    trace!("peer response: {}", hex(&msg));

    let msg = Message::try_from(msg).context("handshake parse error")?;
    if let Message::Handshake {
        info_hash: ref h_info_hash,
        ..
    } = msg
    {
        ensure!(h_info_hash.clone() == info_hash, "response `info_hash` differ");
        Ok((stream, msg))
    } else {
        Err(anyhow!("unexpected message"))
    }
}

pub async fn send_message(stream: &mut OwnedWriteHalf, message: Message) -> Result<()> {
    trace!(">>> sending message: {:?}", message);
    let msg_p: Vec<u8> = message.into();
    trace!("raw message: {}", hex(&msg_p));
    stream.write_all(&msg_p).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn peer_loop(state: Arc<Mutex<State>>) -> Result<()> {
    let config = state.lock().await.config.clone();
    let mut handles = vec![];
    loop {
        debug!("reconnecting peers");
        let peers: Vec<PeerInfo> = state
            .lock()
            .await
            .peers
            .values()
            .filter(|p| p.status == PeerStatus::Disconnected)
            .map(|p| p.info.clone())
            .collect();
        trace!("disconnected peers: {}", peers.len());
        peers.into_iter().for_each(|p| {
            let state = state.clone();
            handles.push(spawn(async {
                if let Err(e) = handle_peer(p, state).await.context("peer error") {
                    debug!("{e:#}");
                };
            }));
        });

        select!(
            _ = async {
                loop {
                    if state.lock().await.status == TorrentStatus::Downloaded {
                        return;
                    }
                    sleep(config.downloaded_check_wait).await
                }
            } => {
                return Ok(())
            },
            _ = sleep(config.reconnect_wait) => ()
        );
    }
}

pub async fn handle_peer(peer: PeerInfo, state: Arc<Mutex<State>>) -> Result<()> {
    {
        debug!("connecting to peer: {:?}", peer);
        let mut state = state.lock().await;
        match state.peers.get_mut(&peer) {
            Some(p) if p.status == PeerStatus::Connected => return Err(anyhow!("peer is already connected")),
            Some(p) => p.status = PeerStatus::Connected,
            None => {
                let mut p = Peer::new(peer.clone());
                p.status = PeerStatus::Connected;
                state.peers.insert(peer.clone(), p);
            }
        };
    };

    let res = do_handle_peer(peer.clone(), state.clone()).await;

    debug!("peer disconnected: {:?}", peer);
    state.lock().await.peers.get_mut(&peer).context("no peer")?.status = if res.is_err() {
        PeerStatus::Disconnected
    } else {
        PeerStatus::Done
    };

    res
}

pub async fn do_handle_peer(peer: PeerInfo, state: Arc<Mutex<State>>) -> Result<()> {
    let (stream, handshake) = handshake(&peer, state.clone()).await.context("handshake error")?;
    info!("successfull handshake with peer {:?}", peer);

    if let Some(p) = state.lock().await.peers.get_mut(&peer) {
        p.status = PeerStatus::Connected;
    }

    let (r_stream, mut w_stream) = stream.into_split();

    let supports_ext = match handshake {
        Message::Handshake { reserved, .. } => Feature::Extension.enabled(&reserved),
        _ => false,
    };
    if supports_ext {
        send_message(
            &mut w_stream,
            Message::Extended {
                ext_id: 0,
                payload: Some(Extension::handshake(&[Extension::Metadata]).encode()),
            },
        )
        .await?;
    }
    send_message(&mut w_stream, Message::Unchoke).await?;
    send_message(&mut w_stream, Message::Interested).await?;

    select!(
        r = {
            let state = state.clone();
            write_loop(w_stream, peer.clone(), state)
        } => r.context("write error"),
        r = {
            let state = state.clone();
            read_loop(r_stream, peer.clone(), state)
        } => r.context("read error")
    )?;

    Ok(())
}

async fn write_loop(mut stream: OwnedWriteHalf, peer: PeerInfo, state: Arc<Mutex<State>>) -> Result<()> {
    loop {
        let (config, p) = {
            let state = state.lock().await;
            (
                state.config.clone(),
                state.peers.get(&peer).cloned().context("no peer")?,
            )
        };
        if config.respect_choke && p.choked {
            debug!("peer is choked, waiting");
            sleep(config.choke_wait).await;
            continue;
        }

        let status = state.lock().await.status.clone();
        match status {
            TorrentStatus::Metainfo => {
                write_metainfo(&mut stream, state.clone(), p).await?;
            }
            TorrentStatus::Downloading => {
                let piece = state.lock().await.next_piece();
                match piece {
                    Some(piece) => {
                        write_piece_request(&mut stream, piece).await?;
                    }
                    _ => {
                        info!("torrent is downloaded");
                        state.lock().await.status = TorrentStatus::Downloaded;
                        debug!("nothing else to do, disconnecting");
                        return Ok(());
                    }
                };
            }
            _ => {
                debug!("nothing else to do, disconnecting");
                return Ok(());
            }
        };
        let wait = state.lock().await.config.piece_request_wait;
        sleep(wait).await;
    }
}

async fn write_metainfo(stream: &mut OwnedWriteHalf, state: Arc<Mutex<State>>, p: Peer) -> Result<()> {
    if let Some(ext_id) = p.extension_map.get(&Extension::Metadata).copied() {
        let metainfo = state.lock().await.metainfo.clone();
        if let Err(m_state) = metainfo {
            if let Some(i) = m_state.next_piece() {
                debug!("requesting metainfo piece {}", i);
                let msg = Message::Extended {
                    ext_id,
                    payload: Some(PeerMetainfoMessage::Request { piece: i }.into()),
                };
                let v: Vec<u8> = PeerMetainfoMessage::Request { piece: i }.into();
                trace!("msg: {}, {}", hex(&v), String::from_utf8_lossy(&v));
                send_message(stream, msg).await?;
            } else {
                debug!("all metainfo pieces downloaded");
                let mut state = state.lock().await;
                let data = m_state.pieces.into_values().flat_map(|b| b.0).collect::<Vec<_>>();
                if let (Some(info_dict), _) = parse_bencoded(data) {
                    debug!("bencoded metainfo: {:?}", info_dict);
                    // since peer metainfo protocol only transfers info dict, it needs
                    // to be inserted into fake metainfo dict to parse properly
                    let metainfo_dict = BencodeValue::Dict([("info".into(), info_dict)].into_iter().collect());
                    match Metainfo::try_from(metainfo_dict) {
                        Ok(metainfo) => {
                            state.pieces = Some(init_pieces(&metainfo.info));
                            state.metainfo = Ok(metainfo);
                            state.status = TorrentStatus::Downloading;
                            info!("metainfo is downloaded: {:?}", state.metainfo);
                        }
                        Err(e) => {
                            panic!("unable to parse metainfo from bencoded: {:#}", e);
                        }
                    }
                } else {
                    warn!("unable to parse bencoded metainfo");
                }
            }
        } else {
            unreachable!("metainfo not available");
        };
    }
    Ok(())
}

async fn write_piece_request(stream: &mut OwnedWriteHalf, piece: Piece) -> Result<()> {
    debug!("next request piece: {:?}", piece);
    let total_blocks = piece.total_blocks();

    let block_idxs = (0..total_blocks)
        .filter(|i| !piece.blocks.contains_key(i))
        .collect::<Vec<_>>();
    for i in block_idxs {
        let request_msg = Message::Request {
            piece_index: piece.index,
            begin: i * BLOCK_SIZE,
            length: if i == total_blocks - 1 && piece.length % BLOCK_SIZE != 0 {
                piece.length % BLOCK_SIZE
            } else {
                BLOCK_SIZE
            },
        };
        send_message(stream, request_msg).await?;
    }
    Ok(())
}

async fn read_loop(mut stream: OwnedReadHalf, peer: PeerInfo, state: Arc<Mutex<State>>) -> Result<()> {
    loop {
        match read_message(&mut stream).await {
            Ok(Message::Choke) => match state.lock().await.peers.get_mut(&peer) {
                Some(p) => p.choked = true,
                _ => debug!("no peer {:?}", peer),
            },
            Ok(Message::Unchoke) => match state.lock().await.peers.get_mut(&peer) {
                Some(p) => p.choked = false,
                _ => debug!("no peer {:?}", peer),
            },
            Ok(Message::Piece {
                piece_index,
                begin,
                block,
            }) => {
                if let Err(e) = read_piece(state.clone(), piece_index, begin, block).await {
                    debug!("{e:#}");
                }
            }
            Ok(Message::Port { port }) => match state.lock().await.peers.get_mut(&peer) {
                Some(p) => {
                    debug!("received port {}", port);
                    p.dht_port = Some(port)
                }
                _ => debug!("no peer {:?}", peer),
            },
            Ok(Message::Extended {
                ext_id,
                payload: Some(payload),
            }) => {
                if let Err(e) = read_ext(state.clone(), &peer, ext_id, payload).await {
                    debug!("read extended error: {e:#}");
                }
            }
            Ok(msg) => {
                debug!("no handler for message, skipping: {:?}", msg);
            }
            Err(e) => {
                warn!("peer message read error: {e:#}");
                return Err(e);
            }
        };
    }
}

async fn read_piece(state: Arc<Mutex<State>>, piece_index: u32, begin: u32, block: Block) -> Result<()> {
    let status = state.lock().await.status.clone();
    if status != TorrentStatus::Downloading {
        debug!("not accepting pieces with status {:?}", status);
        return Ok(());
    }
    if begin % BLOCK_SIZE != 0 {
        return Err(anyhow!("block begin is not a multiple of block size"));
    }
    let block_index = begin / BLOCK_SIZE;

    {
        let mut state = state.lock().await;
        let piece = match state.pieces.as_mut().unwrap().get_mut(&piece_index) {
            Some(p) => p,
            _ => {
                debug!("no piece with index {:?}", piece_index);
                return Ok(());
            }
        };
        if piece.status != TorrentStatus::Downloading {
            debug!("downloaded block of already completed piece, loss");
            return Ok(());
        }
        let total_blocks = piece.total_blocks();
        if block_index != total_blocks - 1 && block.0.len() != BLOCK_SIZE as usize {
            debug!("block of unexpected size: {}", block.0.len());
            return Ok(());
        }
        if piece.blocks.insert(block_index, block).is_some() {
            debug!("repeaded block download, loss");
        };
        trace!("got block {}/{}", piece.blocks.len(), total_blocks);
        if piece.blocks.len() as u32 == total_blocks {
            let piece_data: Vec<u8> = piece.blocks.values().flat_map(|b| b.0.as_slice()).copied().collect();
            let piece_hash = sha1::encode(piece_data);
            if piece_hash != piece.hash.0 {
                warn!("piece hash does not match: {:?}", piece);
                trace!("{}", hex(&piece_hash));
                trace!("{}", hex(&piece.hash.0));
                return Ok(());
            }
            piece.status = TorrentStatus::Downloaded;
            info!(
                "piece {}/{}",
                state
                    .pieces
                    .as_ref()
                    .unwrap()
                    .values()
                    .filter(|p| p.status > TorrentStatus::Downloading)
                    .count(),
                state.pieces.as_ref().unwrap().len(),
            );
        }
    }

    let status = state
        .lock()
        .await
        .pieces
        .as_ref()
        .unwrap()
        .get(&piece_index)
        .context("no piece")?
        .status
        .clone();
    if status == TorrentStatus::Downloaded {
        // TODO: async
        spawn(write_piece(piece_index, state.clone()))
            .await?
            .context("error writing piece")?;
        debug!("piece saved");
    }
    Ok(())
}

async fn read_ext(state: Arc<Mutex<State>>, peer: &PeerInfo, ext_id: u8, payload: Vec<u8>) -> Result<()> {
    debug!("got extended message: #{}", ext_id);
    match ext_id {
        0 => {
            debug!("got extended handshake");
            match parse_bencoded(payload).0 {
                Some(BencodeValue::Dict(dict)) => match dict.get("m") {
                    Some(BencodeValue::Dict(m_d)) => {
                        let ext_map = m_d
                            .iter()
                            .filter_map(|(k, v)| {
                                let ext = Extension::try_from(k.as_str()).ok()?;
                                let num = match v {
                                    BencodeValue::Int(i) => *i as u8,
                                    _ => return Err(anyhow!("ext id is not an int")).ok(),
                                };
                                Some((ext, num))
                            })
                            .collect();
                        trace!("ext map: {:?}", ext_map);
                        state.lock().await.peers.get_mut(peer).context("no peer")?.extension_map = ext_map;
                        Ok(())
                    }
                    _ => Err(anyhow!("no `m` key")),
                },
                _ => Err(anyhow!("parse error")),
            }
        }
        _ => {
            debug!("got extended message #{ext_id}");
            match Extension::try_from(ext_id as usize) {
                Ok(Extension::Metadata) => read_ext_metadata(state, payload).await,
                _ => Err(anyhow!("unsupported extension id: #{}", ext_id)),
            }
        }
    }
}

async fn read_ext_metadata(state: Arc<Mutex<State>>, payload: Vec<u8>) -> Result<()> {
    match PeerMetainfoMessage::try_from(payload) {
        Ok(msg) => {
            debug!("got metadata message {:?}", msg);
            match msg {
                PeerMetainfoMessage::Data {
                    piece,
                    total_size,
                    data,
                } => {
                    let mut state = state.lock().await;
                    if let Err(m_state) = state.metainfo.as_mut() {
                        m_state.pieces.insert(piece, data);
                        m_state.total_size = Some(total_size);
                        debug!(
                            "new metainfo piece {}/{}",
                            m_state.pieces.len(),
                            total_size.div_ceil(METAINFO_PIECE_SIZE)
                        );
                        Ok(())
                    } else {
                        Err(anyhow!("metainfo already set"))
                    }
                }
                _ => Err(anyhow!("unhandled metadata message {:?}", msg)),
            }
        }
        Err(e) => Err(anyhow!("{e:#}")),
    }
}
