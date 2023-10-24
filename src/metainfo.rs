use core::fmt;
use std::path::PathBuf;

use crate::{bencode::BencodeValue, state::PieceHash};

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Metainfo {
    pub info: Info,
    pub announce: String,
    pub announce_list: Option<Vec<String>>,
    pub creation_date: Option<i64>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub encoding: Option<String>,
}

#[derive(Clone, PartialEq, PartialOrd, Hash)]
pub struct Info {
    pub piece_length: i64,
    pub pieces: Vec<PieceHash>,
    pub name: String,
    pub file_info: FileInfo,
    pub private: Option<bool>,
}

impl fmt::Debug for Info {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Info")
            .field("piece_length", &self.piece_length)
            .field("pieces", &format!("<{} hidden>", self.pieces.len()))
            .field("file_info", &self.file_info)
            .field("private", &self.private)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub enum FileInfo {
    Single {
        length: i64,
        md5_sum: Option<String>,
    },
    Multi {
        files: Vec<FilesInfo>,
    },
}

impl FileInfo {
    pub fn total_length(&self) -> i64 {
        match self {
            FileInfo::Single { length, .. } => *length,
            FileInfo::Multi { files } => files.iter().map(|f| f.length).sum(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
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
            Some(BencodeValue::String(s)) => String::from_utf8_lossy(s.as_slice()).into(),
            _ => return Err("'name' missing".into()),
        };
        let file_info = match info_dict.get("files") {
            Some(d) => FileInfo::Multi {
                files: parse_files_info(d)?,
            },
            None => {
                FileInfo::Single {
                    length: match info_dict.get("length") {
                        Some(BencodeValue::Int(v)) => *v,
                        _ => return Err("'length' missing".into()),
                    },
                    // TODO
                    md5_sum: None,
                }
            }
        };
        let metainfo = Metainfo {
            info: Info {
                piece_length: match info_dict.get("piece length") {
                    Some(BencodeValue::Int(v)) => *v,
                    _ => return Err("'piece length' missing".into()),
                },
                pieces,
                name,
                file_info,
                // TODO
                private: None,
            },
            announce: match dict.get("announce") {
                Some(BencodeValue::String(s)) => String::from_utf8_lossy(s.as_slice()).into(),
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

fn parse_files_info(value: &BencodeValue) -> Result<Vec<FilesInfo>, String> {
    match value {
        BencodeValue::List(l) => l
            .iter()
            .map(|i| match i {
                BencodeValue::Dict(d) => {
                    let path = match d.get("path") {
                        Some(BencodeValue::List(p)) => p
                            .iter()
                            .map(|dir| match dir {
                                BencodeValue::String(dir) => Ok(PathBuf::from(
                                    String::from_utf8_lossy(dir.as_slice()).to_string(),
                                )),
                                _ => Err("'path' item is not a string".into()),
                            })
                            .collect::<Result<PathBuf, String>>()?,
                        _ => return Err("'path' is not a list".into()),
                    };
                    Ok(FilesInfo {
                        length: match d.get("length") {
                            Some(BencodeValue::Int(v)) => *v,
                            _ => return Err("'length' missing".into()),
                        },
                        path,
                        // TODO
                        md5_sum: None,
                    })
                }
                _ => Err("'files' item is not a dict".into()),
            })
            .collect(),
        _ => Err("'files' is not a list".into()),
    }
}
