use core::fmt;
use std::{collections::BTreeMap, time::Duration};

use rand::{seq::IteratorRandom, thread_rng};

use crate::{
    hex::hex,
    metainfo::{Info, Metainfo},
    types::ByteString, config::Config,
};

pub const BLOCK_SIZE: u32 = 1 << 14;

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct State {
    pub config: Config,
    pub metainfo: Metainfo,
    pub tracker_timeout: Duration,
    pub info_hash: Vec<u8>,
    pub peer_id: Vec<u8>,
    pub pieces: BTreeMap<u32, Piece>,
    pub peers: BTreeMap<ByteString, Peer>,
    pub status: TorrentStatus,
}

impl State {
    pub fn next_piece(&mut self) -> Option<Piece> {
        let piece = self
            .pieces
            .values()
            .filter(|p| !p.completed)
            .choose(&mut thread_rng())
            .cloned();
        if piece.is_none() {
            debug!("torrent is downloaded");
            self.status = TorrentStatus::Downloaded;
        }
        piece
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub enum TorrentStatus {
    Started,
    Downloaded,
    Saved,
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Piece {
    pub hash: PieceHash,
    pub index: u32,
    pub length: u32,
    /// Map of blocks <block index> -> <block>
    pub blocks: BTreeMap<u32, Block>,
    pub completed: bool,
}

impl Piece {
    pub fn total_blocks(&self) -> u32 {
        (self.length as f64 / BLOCK_SIZE as f64).ceil() as u32
    }
}

#[derive(Clone, PartialEq, PartialOrd, Hash)]
pub struct PieceHash(pub ByteString);

impl fmt::Debug for PieceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", hex(&self.0))
    }
}

#[derive(Clone, PartialEq, PartialOrd, Hash)]
pub struct Block(pub Vec<u8>);

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<block {}>", self.0.len())
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Peer {
    pub info: PeerInfo,
    pub status: PeerStatus,
    pub am_choked: bool,
    pub am_interested: bool,
    pub choked: bool,
    pub interested: bool,
    pub bitfield: Option<Vec<u8>>,
}

impl Peer {
    pub fn new(info: PeerInfo) -> Peer {
        Peer {
            info,
            status: PeerStatus::Disconnected,
            am_choked: true,
            am_interested: false,
            choked: true,
            interested: false,
            bitfield: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub enum PeerStatus {
    Disconnected,
    Connected,
    Done,
}

#[derive(Clone, PartialEq, PartialOrd, Hash)]
pub struct PeerInfo {
    pub peer_id: ByteString,
    pub ip: String,
    pub port: i64,
}

impl fmt::Debug for PeerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("TrackerPeer")
            .field(
                "peer_id",
                &match String::from_utf8(self.peer_id.clone()) {
                    Ok(str) => str,
                    _ => "<non-utf>".into(),
                },
            )
            .field("ip", &self.ip)
            .field("port", &self.port)
            .finish()
    }
}

pub fn init_pieces(info: &Info) -> BTreeMap<u32, Piece> {
    let total_len = info.file_info.total_length() as u32;
    assert!(info.pieces.len() == (total_len as f64 / info.piece_length as f64).ceil() as usize);
    info.pieces
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, p)| {
            (
                i as u32,
                Piece {
                    hash: p,
                    index: i as u32,
                    length: if i == info.pieces.len() - 1 {
                        total_len % info.piece_length
                    } else {
                        info.piece_length
                    },
                    blocks: BTreeMap::new(),
                    completed: false,
                },
            )
        })
        .collect()
}
