use anyhow::{ensure, Result};
use rand::{thread_rng, Rng};
use reqwest::Url;

use crate::{
    hex::hex,
    state::PeerInfo,
    tracker::{TrackerEvent, TrackerRequest, TrackerResponse, TrackerResponseSuccess},
    udp::send_udp,
};

pub async fn tracker_request_udp(announce: String, request: TrackerRequest) -> Result<TrackerResponse> {
    fn i32_from_slice(slice: &[u8]) -> Result<i32> {
        Ok(i32::from_be_bytes(slice.try_into()?))
    }

    let url = Url::parse(&announce)?;
    let tracker_addr = format!("{}:{}", url.host().expect("no host"), url.port().expect("no port"));

    let conn_id: i64 = 0x41727101980;
    let tx_id: i32 = thread_rng().gen();
    let connect_pkt = [&conn_id.to_be_bytes()[..], &0_i32.to_be_bytes(), &tx_id.to_be_bytes()].concat();
    trace!("sending connect pkt: {}", hex(&connect_pkt));
    let pkt = send_udp(&tracker_addr, &connect_pkt).await?.0;
    trace!("read connect pkt: {}", hex(&pkt));
    ensure!(pkt.len() >= 16, "connect packet too short");
    let conn_id = {
        ensure!(i32_from_slice(&pkt[0..4])? == 0, "action is not connect");
        ensure!(i32_from_slice(&pkt[4..8])? == tx_id, "transaction id doesn't match");
        i64::from_be_bytes(pkt[8..16].try_into()?)
    };
    trace!("connection id: {}", hex(&conn_id.to_be_bytes()));

    let tx_id: i32 = thread_rng().gen();
    let announce_pkt = [
        &conn_id.to_be_bytes()[..],
        &1_i32.to_be_bytes(),
        &tx_id.to_be_bytes(),
        &request.info_hash,
        &request.peer_id,
        &request.downloaded.to_be_bytes(),
        &request.left.to_be_bytes(),
        &request.uploaded.to_be_bytes(),
        &match request.event {
            Some(TrackerEvent::Completed) => 1i32,
            Some(TrackerEvent::Started) => 2i32,
            Some(TrackerEvent::Stopped) => 3i32,
            None => 0i32,
        }
        .to_be_bytes(),
        // TODO: ip
        &0_u32.to_be_bytes(),
        // TODO: key
        &0_u32.to_be_bytes(),
        // TODO: numwant
        &(-1_i32).to_be_bytes(),
        // TODO: port
        &0_u16.to_be_bytes(),
    ]
    .concat();
    ensure!(
        announce_pkt.len() == 98,
        format!("announce pkt is incorrect size: {}", announce_pkt.len())
    );
    trace!("sending announce pkt: {}", hex(&connect_pkt));
    let (pkt, addr) = send_udp(&tracker_addr, &announce_pkt).await?;
    if addr.is_ipv6() {
        todo!("ipv6 tracker response");
    }
    ensure!(pkt.len() >= 20, "announce packet too short");
    ensure!((pkt.len() - 20) % 6 == 0, "announce packet wierd size");
    ensure!(i32::from_be_bytes(pkt[0..4].try_into()?) == 1, "action is not announce");
    ensure!(i32_from_slice(&pkt[4..8])? == tx_id, "transaction id doesn't match");
    let addr_count = (pkt.len() - 20) / 6;
    let peers = (0..addr_count)
        .map(|i| 20 + 6 * i)
        .map(|i| PeerInfo::try_from(&pkt[i..i + 6]))
        .collect::<Result<_, _>>()?;

    let resp = TrackerResponse::Success(TrackerResponseSuccess {
        peers,
        interval: i32::from_be_bytes(pkt[8..12].try_into()?) as i64,
        warning_message: None,
        min_interval: None,
        tracker_id: None,
        complete: None,
        incomplete: None,
    });
    debug!("tracker response: {:?}", resp);
    Ok(resp)
}
