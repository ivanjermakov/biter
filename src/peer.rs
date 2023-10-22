use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use crate::hex::hex;
use crate::tracker::TrackerPeer;
use crate::types::ByteString;

pub fn handshake(
    peer: &TrackerPeer,
    info_hash: &ByteString,
    peer_id: &ByteString,
) -> io::Result<()> {
    println!("connecting to peer {peer:?}");
    let timeout = Duration::new(2, 0);
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::from_str(&peer.ip).unwrap(), peer.port as u16),
        timeout,
    )?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let handshake = handshake_packet(info_hash, peer_id);
    println!("writing handshake {}", hex(&handshake.to_vec()));
    match stream.write_all(&handshake) {
        Err(e) => {
            eprintln!("write error: {}", e);
            return Err(e);
        }
        _ => println!("write ok"),
    };
    stream.flush()?;
    let mut reader = stream;
    let mut read_packet = vec![];
    println!("reading response");
    let mut retry = 0;
    loop {
        if retry > 3 {
            return Err(io::Error::new(io::ErrorKind::Other, "read timeout"));
        };
        match reader.read_to_end(&mut read_packet) {
            Err(e) => {
                eprintln!("read error: {}", e);
                return Err(e);
            }
            Ok(n) if n > 0 => {
                println!("peer response: {}", hex(&read_packet.to_vec()));
            }
            _ => {
                println!("no data");
                retry += 1;
                sleep(Duration::new(1, 0));
            }
        };
    }
}

pub fn handshake_packet(info_hash: &ByteString, peer_id: &ByteString) -> Vec<u8> {
    let pstr = "BitTorrent protocol";
    let pstrlen = &[pstr.len() as u8];
    let reserved = &[0u8; 8];
    [pstrlen, pstr.as_bytes(), reserved, &info_hash, &peer_id].concat()
}
