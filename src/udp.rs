use std::net::SocketAddr;

use anyhow::Result;
use tokio::net::UdpSocket;

use crate::hex::hex;

pub async fn send_udp(addr: &str, packet: &[u8]) -> Result<(Vec<u8>, SocketAddr)> {
    let local_addr = "0.0.0.0:0";
    trace!("creating socket at {}", local_addr);
    let socket = UdpSocket::bind(local_addr).await?;
    trace!("connecting to {}", addr);
    socket.connect(addr).await?;
    trace!("connected");

    trace!("sending pkg: {}", hex(packet));
    socket.send(packet).await?;

    trace!("reading pkg");
    let mut buf = [0u8; 1 << 16];
    let (n, addr) = socket.recv_from(&mut buf).await?;
    let pkg = buf[0..n].to_vec();
    trace!("read pkg: {}", hex(&pkg));
    Ok((pkg, addr))
}
