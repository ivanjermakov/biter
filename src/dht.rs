use std::{
    cmp,
    collections::{BTreeSet, VecDeque},
    time::Duration,
};

use anyhow::{Context, Error, Result};
use futures::{stream::FuturesUnordered, StreamExt};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use tokio::time::timeout;

use crate::{
    bencode::{parse_bencoded, BencodeValue},
    hex::hex,
    state::PeerInfo,
    types::ByteString,
    udp::send_udp,
};

pub async fn find_peers(
    dht_peers: Vec<PeerInfo>,
    peer_id: ByteString,
    info_hash: ByteString,
    min: usize,
) -> Result<BTreeSet<PeerInfo>> {
    let mut peers = BTreeSet::new();
    let mut queue = VecDeque::from(dht_peers);
    loop {
        debug!("dht queue: {} nodes", queue.len());

        let chunk = queue
            .drain(..cmp::min(queue.len(), 100))
            .collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }

        let mut handles = chunk
            .into_iter()
            .map(|p| find_peers_single(p.clone(), peer_id.clone(), info_hash.clone()))
            .collect::<FuturesUnordered<_>>();
        while let Some(res) = handles.next().await {
            match res {
                Ok(Ok(values)) => {
                    debug!("received {} new peers via dht", values.len());
                    for v in values {
                        peers.insert(v);
                    }
                    if peers.len() >= min {
                        return Ok(peers);
                    }
                }
                Ok(Err(nodes)) => {
                    for n in nodes {
                        if !queue.contains(&n) {
                            queue.insert(0, n);
                        }
                    }
                }
                Err(e) => {
                    trace!("dht error: {e:#}");
                }
            }
        }
    }

    debug!("dht queue exhausted, found {} peers", peers.len());
    Ok(peers)
}

async fn find_peers_single(
    peer: PeerInfo,
    peer_id: ByteString,
    info_hash: ByteString,
) -> Result<Result<Vec<PeerInfo>, Vec<PeerInfo>>> {
    trace!("quering dht peer: {:?}", peer);
    let res = timeout(
        // TODO: make configurable
        Duration::from_millis(500),
        dht_find_peers(&peer, &peer_id, info_hash.clone()),
    )
    .await??;
    let dict = match res {
        BencodeValue::Dict(dict) => dict,
        _ => return Err(Error::msg("response is not a dict")),
    };

    if matches!(dict.get("y"),  Some(BencodeValue::String(s)) if s == "e".as_bytes()) {
        debug!("krpc error: {:?}", dict);
    }
    let r_dict = match dict.get("r") {
        Some(BencodeValue::Dict(d)) => d,
        _ => return Err(Error::msg("no response dict")),
    };

    if let Some(BencodeValue::List(vs)) = r_dict.get("values") {
        return Ok(Ok(vs
            .iter()
            .map(|b_v| {
                let v = match b_v {
                    BencodeValue::String(s) => s,
                    _ => return Err(Error::msg("value is not a string")),
                };
                PeerInfo::try_from(v.as_slice())
            })
            .collect::<Result<Vec<PeerInfo>>>()?));
    }

    if let Some(BencodeValue::String(ns_str)) = r_dict.get("nodes") {
        if ns_str.len() % 6 != 0 {
            trace!("nodes string length is weird: {}", hex(ns_str));
        }
        return Ok(Err(ns_str
            .chunks_exact(6)
            .map(PeerInfo::try_from)
            .collect::<Result<Vec<PeerInfo>>>()?));
    }

    Err(Error::msg("malformed dht response"))
}

async fn dht_find_peers(
    peer: &PeerInfo,
    peer_id: &ByteString,
    info_hash: ByteString,
) -> Result<BencodeValue> {
    let tx_id = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(2)
        .map(char::from)
        .collect::<String>();
    let req = BencodeValue::Dict(
        [
            ("t".into(), BencodeValue::from(tx_id.as_str())),
            ("y".into(), BencodeValue::from("q")),
            ("q".into(), BencodeValue::from("get_peers")),
            (
                "a".into(),
                BencodeValue::Dict(
                    [
                        ("id".into(), BencodeValue::String(peer_id.clone())),
                        ("info_hash".into(), BencodeValue::String(info_hash)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    // TODO: verify tx_id
    send_krpc(peer, &req).await
}

async fn send_krpc(peer: &PeerInfo, request: &BencodeValue) -> Result<BencodeValue> {
    let packet = request.encode();
    let addr = peer.to_addr();
    trace!("krpc request: {:?}", packet);
    let (resp, _) = send_udp(&addr, &packet).await?;
    trace!("krpc response: {:?}", resp);
    let dict = parse_bencoded(resp).0.context("bencode error")?;
    trace!("krpc response dict: {:?}", dict);
    Ok(dict)
}
