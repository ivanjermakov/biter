use std::collections::BTreeMap;

use anyhow::{Context, Error};

use crate::{
    bencode::{parse_bencoded, BencodeValue},
    state::Block,
};

pub const METAINFO_PIECE_SIZE: usize = 1 << 14;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MetainfoState {
    pub total_size: Option<usize>,
    pub pieces: BTreeMap<usize, Block>,
}

impl MetainfoState {
    pub fn next_piece(&self) -> Option<usize> {
        if self.total_size.is_none() {
            Some(0)
        } else {
            (0..self.total_size?.div_ceil(METAINFO_PIECE_SIZE)).find(|i| !self.pieces.contains_key(i))
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PeerMetainfoMessage {
    Request {
        piece: usize,
    },
    Data {
        piece: usize,
        total_size: usize,
        data: Block,
    },
    Reject,
}

impl PeerMetainfoMessage {
    pub fn msg_type(&self) -> u8 {
        match self {
            PeerMetainfoMessage::Request { .. } => 0,
            PeerMetainfoMessage::Data { .. } => 1,
            PeerMetainfoMessage::Reject => 2,
        }
    }
}

impl From<PeerMetainfoMessage> for Vec<u8> {
    fn from(value: PeerMetainfoMessage) -> Self {
        let msg_type = BencodeValue::from(value.msg_type() as i64);
        match value {
            PeerMetainfoMessage::Request { piece } => BencodeValue::Dict(
                [
                    ("msg_type".into(), msg_type),
                    ("piece".into(), BencodeValue::from(piece as i64)),
                ]
                .into_iter()
                .collect(),
            )
            .encode(),
            PeerMetainfoMessage::Data { .. } => todo!(),
            PeerMetainfoMessage::Reject => {
                BencodeValue::Dict([("msg_type".into(), msg_type)].into_iter().collect()).encode()
            }
        }
    }
}

impl TryFrom<Vec<u8>> for PeerMetainfoMessage {
    type Error = Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let (dict, data) = match parse_bencoded(value) {
            (Some(BencodeValue::Dict(d)), left) => (d, left),
            _ => return Err(Error::msg("parse error")),
        };
        let msg_type = dict.get("msg_type").context("no msg_type")?;
        Ok(match msg_type {
            BencodeValue::Int(0) => {
                let piece = match dict.get("piece").context("no piece")? {
                    BencodeValue::Int(i) => *i as usize,
                    _ => return Err(Error::msg("unexpected piece")),
                };
                PeerMetainfoMessage::Request { piece }
            }
            BencodeValue::Int(1) => {
                let piece = match dict.get("piece").context("no piece")? {
                    BencodeValue::Int(i) => *i as usize,
                    _ => return Err(Error::msg("unexpected piece")),
                };
                let total_size = match dict.get("total_size").context("no total_size")? {
                    BencodeValue::Int(i) => *i as usize,
                    _ => return Err(Error::msg("unexpected total_size")),
                };
                PeerMetainfoMessage::Data {
                    piece,
                    total_size,
                    data: Block(data),
                }
            }
            BencodeValue::Int(2) => PeerMetainfoMessage::Reject,
            _ => return Err(Error::msg("unexpected msg_type")),
        })
    }
}
