use core::fmt;
use std::path::PathBuf;

use crate::bencode::BencodeValue;

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Metainfo {
    pub info: Info,
    pub announce: String,
    pub announce_list: Option<Vec<String>>,
    pub creation_date: Option<i64>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub encoding: Option<String>,
}

#[derive(PartialEq, Eq, Hash)]
pub struct PieceHash(Vec<u8>);

impl fmt::Debug for PieceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .take(1)
            .map(|c| format!("{:x?}", c))
            .collect::<String>()
            .fmt(f)
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Info {
    pub piece_length: i64,
    pub pieces: Vec<PieceHash>,
    pub file_info: FileInfo,
    pub private: Option<bool>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum FileInfo {
    Single {
        name: String,
        length: i64,
        md5_sum: Option<String>,
    },
    Multi {
        name: String,
        files: FilesInfo,
    },
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct FilesInfo {
    pub length: i64,
    pub path: PathBuf,
    pub md5_sum: Option<String>,
}

impl TryFrom<BencodeValue> for Metainfo {
    type Error = String;

    fn try_from(value: BencodeValue) -> Result<Self, Self::Error> {
        let dict = match value {
            BencodeValue::Dict(d) => d,
            _ => return Err("metafile is not a dict".into()),
        };
        let info_dict = match dict.get("info") {
            Some(BencodeValue::Dict(d)) => d,
            _ => return Err("'info' is not a dict".into()),
        };
        let pieces: Vec<PieceHash> = match info_dict.get("pieces") {
            Some(BencodeValue::String(s)) => s
                .as_slice()
                .chunks(20)
                .map(|c| PieceHash(c.to_vec()))
                .collect(),
            _ => return Err("'pieces' missing".into()),
        };
        let name = match info_dict.get("name") {
            Some(BencodeValue::String(s)) => {
                String::from_utf8(s.clone()).map_err(|_| "'name' is not utf-8")?
            }
            _ => return Err("'name' missing".into()),
        };
        let file_info = if info_dict.get("files").is_some() {
            FileInfo::Multi {
                name,
                files: match parse_files_info(info_dict.get("files").unwrap().clone()) {
                    Ok(fi) => fi,
                    Err(e) => return Err(e),
                },
            }
        } else {
            FileInfo::Single {
                name,
                length: match info_dict.get("length") {
                    Some(BencodeValue::Int(v)) => *v,
                    _ => return Err("'length' missing".into()),
                },
                // TODO
                md5_sum: None,
            }
        };
        let metainfo = Metainfo {
            info: Info {
                piece_length: match info_dict.get("piece length") {
                    Some(BencodeValue::Int(v)) => *v,
                    _ => return Err("'piece length' missing".into()),
                },
                pieces,
                file_info,
                // TODO
                private: None,
            },
            announce: match dict.get("announce") {
                Some(BencodeValue::String(s)) => {
                    String::from_utf8(s.clone()).map_err(|_| "'announce' is not utf-8")?
                }
                _ => return Err("'announce' missing".into()),
            },
            // TODO
            announce_list: None,
            // TODO
            creation_date: None,
            // TODO
            comment: None,
            // TODO
            created_by: None,
            // TODO
            encoding: None,
        };
        Ok(metainfo)
    }
}

fn parse_files_info(_value: BencodeValue) -> Result<FilesInfo, String> {
    todo!("parse files info")
}
