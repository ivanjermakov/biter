use std::{fs, path::PathBuf};

use bencode::parse_bencoded;

use crate::metainfo::Metainfo;

mod bencode;
mod metainfo;

fn main() {
    let path = PathBuf::from("data/Learn You a Haskell for Great Good!.torrent");
    let bencoded = fs::read(path).unwrap();
    let metainfo_dict = match parse_bencoded(bencoded) {
        (Some(metadata), left) if left.is_empty() => metadata,
        _ => panic!("metadata file parsing error"),
    };
    println!("{metainfo_dict:#?}");
    let metainfo = match Metainfo::try_from(metainfo_dict) {
        Ok(info) => info,
        Err(e) => panic!("metadata file structure error: {e}"),
    };
    println!("{metainfo:?}");
}
