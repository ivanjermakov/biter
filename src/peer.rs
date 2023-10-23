use anyhow::{ensure, Context, Error, Result};
use std::io::{BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::str::FromStr;
use std::time::Duration;

use crate::hex::hex;
use crate::tracker::TrackerPeer;
use crate::types::ByteString;

pub struct PeerHandshake {
    info_hash: Vec<u8>,
    peer_id: Vec<u8>,
}

impl From<PeerHandshake> for Vec<u8> {
    fn from(value: PeerHandshake) -> Self {
        let pstr = "BitTorrent protocol";
        let pstrlen = &[pstr.len() as u8];
        let reserved = &[0u8; 8];
        [
            pstrlen,
            pstr.as_bytes(),
            reserved,
            &value.info_hash,
            &value.peer_id,
        ]
        .concat()
    }
}

impl TryFrom<Vec<u8>> for PeerHandshake {
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
        Ok(PeerHandshake {
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
    let timeout = Duration::new(4, 0);
    debug!("connecting to peer {peer:?}");
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::from_str(&peer.ip)?, peer.port as u16),
        timeout,
    )?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let handshake: Vec<u8> = PeerHandshake {
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
    let hp = PeerHandshake::try_from(msg)
        .map_err(Error::msg)
        .context("handshake parse error")?;
    ensure!(hp.info_hash == *info_hash, "response `info_hash` differ");
    if hp.peer_id != peer.peer_id {
        debug!("peer id differ")
    }
    Ok(stream)
}
