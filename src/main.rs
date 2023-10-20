use std::{fs, path::PathBuf};

use crate::bencode::parse_bencoded;

mod bencode;

fn main() {
    let path = PathBuf::from("data/archlinux-2023.10.14-x86_64.iso.torrent");
    let bencoded = fs::read(path).unwrap();
    if let (Some(metadata), left) = parse_bencoded(bencoded) {
        if !left.is_empty() {
            panic!("trailing bencoded data: {}", String::from_utf8(left).unwrap());
        }
        println!("{:#?}", metadata);
    }
}
