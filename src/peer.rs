use anyhow::{ensure, Context, Error, Result};
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
    abort::EnsureAbort,
    hex::hex,
    sha1,
    state::{Block, Peer, PeerInfo, PeerStatus, State, TorrentStatus, BLOCK_SIZE},
    types::ByteString,
};

#[derive(Debug)]
pub enum Message {
    Handshake {
        info_hash: Vec<u8>,
        peer_id: Vec<u8>,
    },
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have {
        piece_index: u32,
    },
    Bitfield {
        bitfield: Vec<u8>,
    },
    Request {
        piece_index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        piece_index: u32,
        begin: u32,
        block: Block,
    },
    Cancel {
        piece_index: u32,
        begin: u32,
        length: u32,
    },
    Port {
        port: u8,
    },
}

impl From<Message> for Vec<u8> {
    fn from(value: Message) -> Self {
        fn u32tb(n: u32) -> Vec<u8> {
            n.to_be_bytes().to_vec()
        }
        match value {
            Message::Handshake { info_hash, peer_id } => {
                let pstr = "BitTorrent protocol";
                let pstrlen = &[pstr.len() as u8];
                let reserved = &[0u8; 8];
                [pstrlen, pstr.as_bytes(), reserved, &info_hash, &peer_id].concat()
            }
            Message::KeepAlive => [u32tb(0).as_slice()].concat(),
            Message::Choke => [u32tb(1).as_slice(), &[0]].concat(),
            Message::Unchoke => [u32tb(1).as_slice(), &[1]].concat(),
            Message::Interested => [u32tb(1).as_slice(), &[2]].concat(),
            Message::NotInterested => [u32tb(1).as_slice(), &[3]].concat(),
            Message::Have { piece_index } => {
                [u32tb(5).as_slice(), &[4], &u32tb(piece_index)].concat()
            }
            Message::Bitfield { bitfield } => {
                [u32tb(1 + bitfield.len() as u32).as_slice(), &[5], &bitfield].concat()
            }
            Message::Request {
                piece_index,
                begin,
                length,
            } => [
                u32tb(13).as_slice(),
                &[6],
                &u32tb(piece_index),
                &u32tb(begin),
                &u32tb(length),
            ]
            .concat(),
            Message::Piece {
                piece_index,
                begin,
                block,
            } => [
                u32tb(9 + block.0.len() as u32).as_slice(),
                &[7],
                &u32tb(piece_index),
                &u32tb(begin),
                &block.0,
            ]
            .concat(),
            Message::Cancel {
                piece_index,
                begin,
                length,
            } => [
                u32tb(13).as_slice(),
                &[8],
                &u32tb(piece_index),
                &u32tb(begin),
                &u32tb(length),
            ]
            .concat(),
            Message::Port { port } => [u32tb(3).as_slice(), &[9], &[port]].concat(),
        }
    }
}

impl TryFrom<Vec<u8>> for Message {
    type Error = String;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        if value.len() != 68 {
            return Err(format!("invalid handshake len: {}", value.len()));
        }
        let pstrlen = &value.as_slice()[0..1];
        if pstrlen != [19u8] {
            return Err(format!("invalid pstrlen: {}", hex(pstrlen)));
        }
        let pstr = &value.as_slice()[1..20];
        if pstr != "BitTorrent protocol".as_bytes() {
            return Err(format!("invalid pstr: {}", hex(pstr)));
        }
        Ok(Message::Handshake {
            info_hash: value.as_slice()[28..48].to_vec(),
            peer_id: value.as_slice()[48..68].to_vec(),
        })
    }
}

/// Generate random 20 byte string, starting with -<2 byte client name><4 byte client version>-
pub fn generate_peer_id() -> ByteString {
    let rand = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .collect::<Vec<_>>();
    ["-ER0000-".as_bytes(), &rand].concat()
}

pub async fn handshake(peer: &PeerInfo, state: Arc<Mutex<State>>) -> Result<TcpStream> {
    let (info_hash, peer_id, peer_connect_timeout) = {
        let state = state.lock().await;
        (
            state.info_hash.clone(),
            state.peer_id.clone(),
            state.config.peer_connect_timeout,
        )
    };
    debug!("connecting to peer {peer:?}");
    let mut stream = timeout(
        peer_connect_timeout,
        TcpStream::connect(format!("{}:{}", peer.ip, peer.port)),
    )
    .await??;
    let handshake: Vec<u8> = Message::Handshake {
        info_hash: info_hash.clone(),
        peer_id: peer_id.clone(),
    }
    .into();

    trace!("writing handshake {}", hex(&handshake.to_vec()));
    stream.write_all(&handshake).await.context("write error")?;
    stream.flush().await?;

    let mut read_packet = [0; 68];
    debug!("reading handshake");
    stream
        .read_exact(&mut read_packet)
        .await
        .context("read error")?;
    let msg: Vec<u8> = read_packet.to_vec();
    debug!("peer response: {}", hex(&msg));
    if let Message::Handshake {
        info_hash: h_info_hash,
        ..
    } = Message::try_from(msg)
        .map_err(Error::msg)
        .context("handshake parse error")?
    {
        ensure!(h_info_hash == *info_hash, "response `info_hash` differ");
        Ok(stream)
    } else {
        Err(Error::msg("unexpected message"))
    }
}

pub async fn read_message(stream: &mut OwnedReadHalf) -> Result<Message> {
    fn u32_from_slice(slice: &[u8]) -> Result<u32> {
        Ok(u32::from_be_bytes(slice.try_into()?))
    }

    let mut len_p = [0; 4];
    stream.read_exact(&mut len_p).await?;
    let len = u32::from_be_bytes(len_p);
    if len == 0 {
        return Ok(Message::KeepAlive);
    }

    let mut id_p = [0; 1];
    stream
        .read_exact(&mut id_p)
        .await
        .context("id_p read error")?;
    let id = u8::from_be_bytes(id_p);

    let msg = match id {
        0 if len == 1 => Ok(Message::Choke),
        1 if len == 1 => Ok(Message::Unchoke),
        2 if len == 1 => Ok(Message::Interested),
        3 if len == 1 => Ok(Message::NotInterested),
        _ if len == 1 => Err(Error::msg("unexpected message of size 1")),
        _ => {
            let mut payload_p = vec![0; len as usize - 1];
            stream
                .read_exact(&mut payload_p)
                .await
                .context("payload_p read error")?;
            match id {
                4 if len == 5 => Ok(Message::Have {
                    piece_index: u32_from_slice(&payload_p[0..4])?,
                }),
                5 => Ok(Message::Bitfield {
                    bitfield: payload_p,
                }),
                6 if len == 13 => Ok(Message::Request {
                    piece_index: u32_from_slice(&payload_p[0..4])?,
                    begin: u32_from_slice(&payload_p[4..8])?,
                    length: u32_from_slice(&payload_p[8..12])?,
                }),
                7 if len > 9 => Ok(Message::Piece {
                    piece_index: u32_from_slice(&payload_p[0..4])?,
                    begin: u32_from_slice(&payload_p[4..8])?,
                    block: Block(payload_p[8..].to_vec()),
                }),
                8 if len == 13 => Ok(Message::Cancel {
                    piece_index: u32_from_slice(&payload_p[0..4])?,
                    begin: u32_from_slice(&payload_p[4..8])?,
                    length: u32_from_slice(&payload_p[8..12])?,
                }),
                9 if len == 3 => Ok(Message::Port { port: payload_p[0] }),
                _ => Err(Error::msg(format!(
                    "unexpected message: {}",
                    hex(&[len_p.as_ref(), &id_p, payload_p.as_slice()].concat())
                ))),
            }
        }
    }?;
    trace!("<<< read message: {:?}", msg);
    Ok(msg)
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
                // this is important to ensure that no tasks hold Arc<State> reference
                trace!("closing {} peer connections", handles.len());
                for h in handles {
                    let _ = h.ensure_abort().await;
                }
                trace!("peer connections closed");
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
            Some(p) if p.status == PeerStatus::Connected => {
                return Err(Error::msg("peer is already connected"))
            }
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
    state
        .lock()
        .await
        .peers
        .get_mut(&peer)
        .context("no peer")?
        .status = if res.is_err() {
        PeerStatus::Disconnected
    } else {
        PeerStatus::Done
    };

    res
}

pub async fn do_handle_peer(peer: PeerInfo, state: Arc<Mutex<State>>) -> Result<()> {
    let stream = handshake(&peer, state.clone())
        .await
        .context("handshake error")?;
    info!("successfull handshake with peer {:?}", peer);

    if let Some(p) = state.lock().await.peers.get_mut(&peer) {
        p.status = PeerStatus::Connected;
    }

    let (r_stream, mut w_stream) = stream.into_split();

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

pub async fn write_loop(
    mut stream: OwnedWriteHalf,
    peer: PeerInfo,
    state: Arc<Mutex<State>>,
) -> Result<()> {
    loop {
        {
            let state = state.lock().await;
            if state.config.respect_choke {
                let p = state.peers.get(&peer).cloned();
                if let Some(p) = p {
                    if p.choked {
                        info!("peer is choked, waiting");
                        sleep(state.config.choke_wait).await;
                        continue;
                    }
                }
            }
        }

        let piece = match state.lock().await.next_piece() {
            Some(p) => p,
            _ => {
                debug!("no more pieces to request, disconnecting");
                return Ok(());
            }
        };

        debug!("next request piece: {:?}", piece);
        let total_blocks = piece.total_blocks();

        // TODO: only request blocks you don't have
        for i in 0..total_blocks {
            let request_msg = Message::Request {
                piece_index: piece.index,
                begin: i * BLOCK_SIZE,
                length: if i == total_blocks - 1 && piece.length % BLOCK_SIZE != 0 {
                    piece.length % BLOCK_SIZE
                } else {
                    BLOCK_SIZE
                },
            };
            send_message(&mut stream, request_msg).await?;
        }

        let wait = state.lock().await.config.piece_request_wait;
        sleep(wait).await;
    }
}

async fn read_loop(
    mut stream: OwnedReadHalf,
    peer: PeerInfo,
    state: Arc<Mutex<State>>,
) -> Result<()> {
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
                if begin % BLOCK_SIZE != 0 {
                    warn!("block begin is not a multiple of block size");
                    continue;
                }
                let block_index = begin / BLOCK_SIZE;
                let mut state = state.lock().await;
                let piece = match state.pieces.get_mut(&piece_index) {
                    Some(p) => p,
                    _ => {
                        debug!("no piece with index {:?}", piece_index);
                        continue;
                    }
                };
                if piece.completed {
                    debug!("downloaded block of already completed piece, loss");
                    continue;
                }
                let total_blocks = piece.total_blocks();
                if block_index != total_blocks - 1 && block.0.len() != BLOCK_SIZE as usize {
                    debug!("block of unexpected size: {}", block.0.len());
                    continue;
                }
                if piece.blocks.insert(block_index, block).is_some() {
                    debug!("repeaded block download, loss");
                };
                trace!("got block {}/{}", piece.blocks.len(), total_blocks);
                if piece.blocks.len() as u32 == total_blocks {
                    let piece_data: Vec<u8> = piece
                        .blocks
                        .values()
                        .flat_map(|b| b.0.as_slice())
                        .copied()
                        .collect();
                    let piece_hash = sha1::encode(piece_data);
                    if piece_hash != piece.hash.0 {
                        warn!("piece hash does not match: {:?}", piece);
                        trace!("{}", hex(&piece_hash));
                        trace!("{}", hex(&piece.hash.0));
                        continue;
                    }
                    piece.completed = true;
                    info!(
                        "piece completed {}/{}",
                        state.pieces.values().filter(|p| p.completed).count(),
                        state.pieces.len(),
                    );
                }
            }
            Ok(msg) => {
                debug!("no handler for message, skipping: {:?}", msg);
            }
            Err(e) => {
                warn!("{e:#}");
                return Err(e);
            }
        };
    }
}
