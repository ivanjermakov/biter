use core::fmt;
use std::collections::BTreeMap;

use crate::{
    hex::hex,
    metainfo::{Info, Metainfo},
    types::ByteString,
};

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct State {
    pub metainfo: Metainfo,
    pub info_hash: Vec<u8>,
    pub peer_id: Vec<u8>,
    pub pieces: Vec<Piece>,
    pub peers: BTreeMap<ByteString, Peer>,
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Piece {
    pub hash: PieceHash,
    pub index: i64,
    pub length: i64,
    pub blocks: Vec<Block>,
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
        f.write_str("<block>")
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Peer {
    pub info: PeerInfo,
    pub am_choked: bool,
    pub am_interested: bool,
    pub choked: bool,
    pub interested: bool,
    pub bitfield: Option<Vec<u8>>,
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

pub fn init_pieces(info: &Info) -> Vec<Piece> {
    let total_len = info.file_info.total_length();
    assert!(info.pieces.len() == (total_len as f64 / info.piece_length as f64).ceil() as usize);
    info.pieces
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, p)| Piece {
            hash: p,
            index: i as i64,
            length: if i == info.pieces.len() - 1 {
                total_len % info.piece_length
            } else {
                info.piece_length
            },
            blocks: vec![],
        })
        .collect()
}
