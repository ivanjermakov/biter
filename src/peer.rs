use anyhow::{ensure, Context, Error, Result};
use core::fmt;
use std::io::{BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::str::FromStr;
use std::time::Duration;

use crate::hex::hex;
use crate::tracker::TrackerPeer;
use crate::types::ByteString;

pub struct Block(Vec<u8>);

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<block>")
    }
}

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

pub fn handshake(
    peer: &TrackerPeer,
    info_hash: &ByteString,
    peer_id: &ByteString,
) -> Result<TcpStream> {
    let cn_timeout = Duration::new(1, 0);
    let hs_timeout = Duration::new(1, 0);
    let rw_timeout = Duration::new(2, 0);
    debug!("connecting to peer {peer:?}");
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::from_str(&peer.ip)?, peer.port as u16),
        cn_timeout,
    )?;
    // special short timeout for the first read
    stream.set_read_timeout(Some(hs_timeout))?;
    stream.set_write_timeout(Some(rw_timeout))?;
    let handshake: Vec<u8> = Message::Handshake {
        info_hash: info_hash.clone(),
        peer_id: peer_id.clone(),
    }
    .into();

    debug!("writing handshake {}", hex(&handshake.to_vec()));
    stream.write_all(&handshake).context("write error")?;
    stream.flush()?;

    let mut reader = BufReader::new(&stream);
    let mut read_packet = [0; 68];
    debug!("reading handshake");
    reader.read_exact(&mut read_packet).context("read error")?;
    let msg: Vec<u8> = read_packet.to_vec();
    debug!("peer response: {}", hex(&msg));
    if let Message::Handshake {
        info_hash: h_info_hash,
        peer_id: h_peer_id,
    } = Message::try_from(msg)
        .map_err(Error::msg)
        .context("handshake parse error")?
    {
        ensure!(h_info_hash == *info_hash, "response `info_hash` differ");
        if h_peer_id != peer.peer_id {
            debug!("peer id differ")
        }
        stream.set_read_timeout(Some(rw_timeout))?;
        Ok(stream)
    } else {
        Err(Error::msg("unexpected message"))
    }
}

pub fn read_message(stream: &TcpStream) -> Result<Message> {
    let mut reader = BufReader::new(stream);
    fn u32_from_slice(slice: &[u8]) -> Result<u32> {
        Ok(u32::from_be_bytes(slice.try_into()?))
    }

    let mut len_p = [0; 4];
    reader.read_exact(&mut len_p)?;
    let len = u32::from_be_bytes(len_p);
    if len == 0 {
        return Ok(Message::KeepAlive);
    }

    let mut id_p = [0; 1];
    reader.read_exact(&mut id_p).context("id_p read error")?;
    let id = u8::from_be_bytes(id_p);

    let msg = match id {
        0 if len == 1 => Ok(Message::Choke),
        1 if len == 1 => Ok(Message::Unchoke),
        2 if len == 1 => Ok(Message::Interested),
        3 if len == 1 => Ok(Message::NotInterested),
        _ if len == 1 => Err(Error::msg("unexpected message of size 1")),
        _ => {
            let mut payload_p = vec![0; len as usize - 1];
            reader
                .read_exact(&mut payload_p)
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
    debug!("<<< read message: {:?}", msg);
    Ok(msg)
}

pub fn send_message(mut stream: &TcpStream, message: Message) -> Result<()> {
    debug!(">>> sending message: {:?}", message);
    let msg_p: Vec<u8> = message.into();
    trace!("raw message: {}", hex(&msg_p));
    stream.write_all(&msg_p)?;
    stream.flush()?;
    Ok(())
}
