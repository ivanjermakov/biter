use std::{collections::BTreeSet, time::Duration};

use anyhow::{Context, Error, Result};
use async_recursion::async_recursion;
use futures::stream::FuturesUnordered;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use tokio::{spawn, time::timeout};

use crate::{
    bencode::{parse_bencoded, BencodeValue},
    hex::hex,
    state::PeerInfo,
    types::ByteString,
    udp::send_udp,
};

#[async_recursion]
pub async fn find_peers(
    peer: PeerInfo,
    peer_id: ByteString,
    info_hash: ByteString,
    min: usize,
) -> Result<BTreeSet<PeerInfo>> {
    if min == 0 {
        return Ok(BTreeSet::new());
    }
    trace!("quering dht peer: {:?}, {} left", peer, min);
    let res = timeout(
        // TODO: make configurable
        Duration::from_secs(1),
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

    let values: Option<Vec<PeerInfo>> = match r_dict.get("values").cloned() {
        Some(BencodeValue::List(vs)) => vs
            .into_iter()
            .map(|b_v| {
                let v = match b_v {
                    BencodeValue::String(s) => s,
                    _ => return Err(Error::msg("value is not a string")),
                };
                PeerInfo::try_from(v.as_slice())
            })
            .collect::<Result<Vec<PeerInfo>>>()
            .ok(),
        _ => None,
    };

    let nodes: Option<Vec<PeerInfo>> = match r_dict.get("nodes").cloned() {
        Some(BencodeValue::String(ns_str)) => {
            if ns_str.len() % 6 != 0 {
                trace!("nodes string length is weird: {}", hex(&ns_str));
            }
            Some(
                ns_str
                    .chunks_exact(6)
                    .map(PeerInfo::try_from)
                    .collect::<Result<Vec<PeerInfo>>>()?,
            )
        }
        _ => None,
    };

    debug!(
        "received {} values, {} nodes",
        values.clone().unwrap_or_default().len(),
        nodes.clone().unwrap_or_default().len()
    );
    let mut found = BTreeSet::new();
    if let Some(vs) = values {
        return Ok(vs.into_iter().collect());
    }

    if let Some(ns) = nodes {
        let handles = ns
            .into_iter()
            .map(|n| {
                spawn(find_peers(
                    n,
                    peer_id.clone(),
                    info_hash.clone(),
                    min - found.len(),
                ))
            })
            .collect::<FuturesUnordered<_>>();
        for h in handles {
            if let Ok(Ok(vs)) = h.await {
                for v in vs {
                    found.insert(v);
                }
                if found.len() >= min {
                    return Ok(found);
                }
            }
        }
    }
    if found.is_empty() {
        Err(Error::msg("no peers found"))
    } else {
        Ok(found)
    }
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
