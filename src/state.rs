use core::fmt;
use std::collections::BTreeMap;

use anyhow::{ensure, Error};
use rand::{seq::IteratorRandom, thread_rng};
use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    extension::Extension,
    hex::hex,
    metainfo::{Info, Metainfo},
    peer_metainfo::MetainfoState,
    tracker::TrackerResponseSuccess,
    types::ByteString,
};

pub const BLOCK_SIZE: u32 = 1 << 14;

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub config: Config,
    pub info_hash: Vec<u8>,
    pub peer_id: Vec<u8>,
    pub peers: BTreeMap<PeerInfo, Peer>,
    pub status: TorrentStatus,
    pub metainfo: Result<Metainfo, MetainfoState>,
    pub tracker_response: Option<TrackerResponseSuccess>,
    pub pieces: Option<BTreeMap<u32, Piece>>,
}

impl State {
    pub fn next_piece(&mut self) -> Option<Piece> {
        self.pieces
            .as_ref()?
            .values()
            .filter(|p| p.status == TorrentStatus::Downloading)
            .choose(&mut thread_rng())
            .cloned()
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub enum TorrentStatus {
    Metainfo,
    Downloading,
    Downloaded,
    Saved,
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct Piece {
    pub hash: PieceHash,
    pub index: u32,
    pub length: u32,
    /// Map of blocks <block index> -> <block>
    pub blocks: BTreeMap<u32, Block>,
    pub status: TorrentStatus,
    pub file_locations: Vec<FileLocation>,
}

impl Piece {
    pub fn total_blocks(&self) -> u32 {
        self.length.div_ceil(BLOCK_SIZE)
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
    pub dht_port: Option<u16>,
    pub extension_map: BTreeMap<Extension, u8>,
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
            dht_port: None,
            extension_map: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub enum PeerStatus {
    Disconnected,
    Connected,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
}

impl PeerInfo {
    pub fn to_addr(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }
}

impl TryFrom<&[u8]> for PeerInfo {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        ensure!(value.len() == 6, "expected 6 byte slice");
        Ok(PeerInfo {
            ip: value[0..4].iter().map(|b| b.to_string()).collect::<Vec<_>>().join("."),
            port: u16::from_be_bytes(value[4..6].try_into()?),
        })
    }
}

pub fn init_pieces(info: &Info) -> BTreeMap<u32, Piece> {
    let files_start = info
        .file_info
        .files()
        .iter()
        .scan(0, |acc, f| {
            let res = *acc;
            *acc += f.length;
            Some(res)
        })
        .collect::<Vec<_>>();
    let total_len = info.file_info.total_length();
    if info.pieces.len() != total_len.div_ceil(info.piece_length) as usize {
        warn!(
            "total length/piece size/piece count inconsistent: {} info pieces, {} expected",
            info.pieces.len(),
            total_len.div_ceil(info.piece_length)
        );
    }
    info.pieces
        .iter()
        .cloned()
        .enumerate()
        .flat_map(|(i, p)| {
            let length: u64 = if i == info.pieces.len() - 1 {
                total_len % info.piece_length
            } else {
                info.piece_length
            };
            let file_locations: Vec<_> = files_start
                .iter()
                .copied()
                .enumerate()
                .flat_map(|(f_i, f_start)| {
                    let f_len = info.file_info.files()[f_i].length;
                    let f_end = f_start + f_len;
                    let piece_start = i as u64 * info.piece_length;
                    let piece_end = piece_start + length;
                    let p_start = (f_start as i64).clamp(piece_start as i64, piece_end as i64);
                    let p_end = (f_end as i64).clamp(piece_start as i64, piece_end as i64);
                    let p_len = p_end - p_start;
                    let offset = p_start - f_start as i64;
                    let piece_offset = (p_start - piece_start as i64) as usize;
                    if p_len != 0 {
                        vec![FileLocation {
                            file_index: f_i,
                            offset: offset as usize,
                            piece_offset,
                            length: p_len as usize,
                        }]
                    } else {
                        vec![]
                    }
                })
                .collect();
            if file_locations.is_empty() {
                debug!("piece does not map to any files: {:?}", p);
                return vec![];
            }
            // TODO: verify files' location integrity
            vec![(
                i as u32,
                Piece {
                    hash: p,
                    index: i as u32,
                    length: length as u32,
                    blocks: BTreeMap::new(),
                    status: TorrentStatus::Downloading,
                    file_locations,
                },
            )]
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct FileLocation {
    pub file_index: usize,
    pub offset: usize,
    pub piece_offset: usize,
    pub length: usize,
}
