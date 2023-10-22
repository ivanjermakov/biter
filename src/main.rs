use peer::handshake;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::{fs, path::PathBuf};

use bencode::parse_bencoded;
use types::ByteString;

use crate::{
    metainfo::Metainfo,
    tracker::{tracker_request, TrackerRequest, TrackerResponse},
};

mod bencode;
mod hex;
mod metainfo;
mod peer;
mod sha1;
mod tracker;
mod types;

fn main() {
    let path = PathBuf::from("data/knoppix.torrent");
    let bencoded = fs::read(path).unwrap();
    let metainfo_dict = match parse_bencoded(bencoded) {
        (Some(metadata), left) if left.is_empty() => metadata,
        _ => panic!("metadata file parsing error"),
    };
    println!("metainfo dict: {metainfo_dict:?}");
    let metainfo = match Metainfo::try_from(metainfo_dict.clone()) {
        Ok(info) => info,
        Err(e) => panic!("metadata file structure error: {e}"),
    };
    println!("metainfo: {metainfo:?}");
    let info_dict_str = match metainfo_dict {
        bencode::BencodeValue::Dict(d) => d.get("info").unwrap().encode(),
        _ => unreachable!(),
    };
    let info_hash = sha1::encode(info_dict_str);
    let peer_id = generate_peer_id();
    println!("peer id {}", String::from_utf8_lossy(peer_id.as_slice()));
    let tracker_response = tracker_request(
        metainfo.announce,
        TrackerRequest::new(
            info_hash.clone(),
            peer_id.clone(),
            tracker::TrackerEvent::Started,
            None,
        ),
    )
    .expect("request failed");
    println!("tracker response: {tracker_response:?}");
    if let TrackerResponse::Success(resp) = tracker_response {
        for p in resp.peers {
            match handshake(&p, &info_hash, &peer_id) {
                Ok(_) => println!("successfull handshake with peer {:?}", p),
                Err(e) => eprintln!("handshake error: {}", e),
            }
        }
    }
}

/// Generate random 20 byte string, starting with -<2 byte client name><4 byte client version>-
fn generate_peer_id() -> ByteString {
    let rand = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .collect::<Vec<_>>();
    vec!["-ER0000-".as_bytes(), &rand]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}
