use core::fmt;
use std::sync::Arc;

use anyhow::{Context, Error, Result};
use reqwest::Client;
use tokio::{spawn, sync::Mutex, time::sleep};
use urlencoding::encode_binary;

use crate::{
    bencode::{parse_bencoded, BencodeValue},
    state::{Peer, PeerInfo, PeerStatus, State},
    types::ByteString,
};

#[allow(dead_code)]
pub struct TrackerRequest {
    info_hash: ByteString,
    peer_id: ByteString,
    port: i64,
    uploaded: i64,
    downloaded: i64,
    left: i64,
    compact: i64,
    no_peer_id: i64,
    event: Option<TrackerEvent>,
    ip: Option<ByteString>,
    numwant: Option<i64>,
    key: Option<ByteString>,
    tracker_id: Option<ByteString>,
}

impl TrackerRequest {
    pub fn new(
        info_hash: ByteString,
        peer_id: ByteString,
        port: u16,
        event: Option<TrackerEvent>,
        tracker_id: Option<ByteString>,
    ) -> TrackerRequest {
        TrackerRequest {
            info_hash,
            peer_id,
            port: port as i64,
            // TODO
            uploaded: 0,
            // TODO
            downloaded: 0,
            // TODO
            left: 0,
            // TODO: compact mode
            compact: 0,
            // TODO: no_peer_id
            no_peer_id: 0,
            event,
            // TODO
            ip: None,
            numwant: None,
            key: None,
            tracker_id,
        }
    }

    pub fn to_params(&self) -> Vec<(String, String)> {
        let mut params: Vec<(&str, Vec<u8>)> = vec![
            ("info_hash", self.info_hash.clone()),
            ("peer_id", self.peer_id.clone()),
            ("port", self.port.to_string().into()),
            ("uploaded", self.uploaded.to_string().into()),
            ("downloaded", self.downloaded.to_string().into()),
            ("left", self.left.to_string().into()),
            ("compact", self.compact.to_string().into()),
            ("no_peer_id", self.no_peer_id.to_string().into()),
        ];

        if let Some(event) = &self.event {
            params.push(("event", event.to_string().into()));
        }

        params
            .iter()
            .map(|(k, v)| (k.to_string(), encode_binary(v.as_slice()).as_ref().into()))
            .collect()
    }
}

#[allow(dead_code)]
#[derive(Debug, PartialEq, PartialOrd)]
pub enum TrackerEvent {
    Started,
    Stopped,
    Completed,
}

impl fmt::Display for TrackerEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TrackerEvent::Started => "started",
            TrackerEvent::Stopped => "stopped",
            TrackerEvent::Completed => "completed",
        })
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum TrackerResponse {
    Failure { failure_reason: String },
    Success(TrackerResponseSuccess),
}

impl TryFrom<BencodeValue> for TrackerResponse {
    type Error = String;

    fn try_from(value: BencodeValue) -> Result<Self, Self::Error> {
        let dict = match value {
            BencodeValue::Dict(d) => d,
            _ => return Err("response is not a dict".into()),
        };
        let peers = match dict.get("peers") {
            Some(BencodeValue::List(ps)) => ps
                .iter()
                .map(|p| match p {
                    BencodeValue::Dict(p_dict) => Ok(PeerInfo {
                        peer_id: match p_dict.get("peer id") {
                            Some(BencodeValue::String(v)) => v.clone(),
                            _ => return Err("'peer id' missing".into()),
                        },
                        ip: match p_dict.get("ip") {
                            Some(BencodeValue::String(i)) => {
                                String::from_utf8(i.clone()).map_err(|e| e.to_string())?
                            }
                            _ => return Err("'ip' missing".into()),
                        },
                        port: match p_dict.get("port") {
                            Some(BencodeValue::Int(p)) => *p,
                            _ => return Err("'port' missing".into()),
                        },
                    }),
                    _ => Err("'peers' missing".into()),
                })
                .collect::<Result<_, String>>()?,
            _ => return Err("'peers' missing".into()),
        };
        let resp = TrackerResponse::Success(TrackerResponseSuccess {
            peers,
            interval: match dict.get("interval") {
                Some(BencodeValue::Int(p)) => *p,
                _ => return Err("'interval' missing".into()),
            },
            warning_message: dict.get("warning_message").and_then(|m| match m {
                BencodeValue::String(s) => Some(String::from_utf8_lossy(s).into()),
                _ => None,
            }),
            min_interval: dict.get("min_interval").and_then(|m| match m {
                BencodeValue::Int(i) => Some(*i),
                _ => None,
            }),
            tracker_id: dict.get("tracker id").and_then(|m| match m {
                BencodeValue::String(s) => Some(s.clone()),
                _ => None,
            }),
            complete: dict.get("complete").and_then(|m| match m {
                BencodeValue::Int(i) => Some(*i),
                _ => None,
            }),
            incomplete: dict.get("incomplete").and_then(|m| match m {
                BencodeValue::Int(i) => Some(*i),
                _ => None,
            }),
        });
        Ok(resp)
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct TrackerResponseSuccess {
    pub peers: Vec<PeerInfo>,
    pub interval: i64,
    pub warning_message: Option<String>,
    pub min_interval: Option<i64>,
    pub tracker_id: Option<ByteString>,
    pub complete: Option<i64>,
    pub incomplete: Option<i64>,
}

pub async fn tracker_request(announce: String, request: TrackerRequest) -> Result<TrackerResponse> {
    let params = format!(
        "?{}",
        request
            .to_params()
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&")
    );
    let url = format!("{announce}{params}");
    debug!("url: {url}");
    let resp = spawn(Client::new().get(url).send())
        .await?
        .context("request error")?
        .bytes()
        .await
        .context("request body error")?;
    debug!("raw response: {}", String::from_utf8_lossy(&resp));
    let resp_dict = parse_bencoded(resp.to_vec())
        .0
        .context("malformed response")?;
    debug!("response: {resp_dict:?}");
    TrackerResponse::try_from(resp_dict).map_err(Error::msg)
}

pub async fn tracker_loop(state: Arc<Mutex<State>>) {
    loop {
        let (announce, info_hash, peer_id, tracker_timeout) = {
            let state = state.lock().await;
            (
                state.metainfo.announce.clone(),
                state.info_hash.clone(),
                state.peer_id.clone(),
                state.tracker_timeout,
            )
        };
        // TODO: include tracker id
        let tracker_response = tracker_request(
            announce,
            TrackerRequest::new(
                info_hash,
                peer_id,
                state.lock().await.config.port,
                None,
                None,
            ),
        )
        .await
        .context("request failed");
        info!("tracker response: {tracker_response:?}");

        // TODO: in case of error, try trackers from announce-list
        match tracker_response {
            Ok(TrackerResponse::Success(resp)) => {
                let mut state = state.lock().await;
                let new_peers: Vec<_> = resp
                    .peers
                    .into_iter()
                    .filter(|p| !state.peers.contains_key(&p.peer_id))
                    .map(Peer::new)
                    .collect();
                info!("received {} new peers", new_peers.len());
                for p in new_peers {
                    state.peers.insert(p.info.peer_id.clone(), p);
                }
                info!(
                    "total {} peers, {} connected",
                    state.peers.len(),
                    state
                        .peers
                        .values()
                        .filter(|p| p.status == PeerStatus::Connected)
                        .count()
                );
            }
            Ok(TrackerResponse::Failure { failure_reason }) => {
                debug!("tracker failure: {}", failure_reason);
            }
            Err(e) => {
                debug!("{e:#}");
            }
        };

        debug!("tracker timeout is {:?}", tracker_timeout);
        sleep(tracker_timeout).await;
    }
}
