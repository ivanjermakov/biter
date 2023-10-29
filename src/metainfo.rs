use core::fmt;
use std::path::PathBuf;

use crate::{bencode::BencodeValue, state::PieceHash};

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Metainfo {
    pub info: Info,
    pub announce: String,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub creation_date: Option<i64>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub encoding: Option<String>,
}

#[derive(Clone, PartialEq, PartialOrd, Hash)]
pub struct Info {
    pub piece_length: u32,
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
    Single(PathInfo),
    Multi(Vec<PathInfo>),
}

impl FileInfo {
    pub fn total_length(&self) -> u32 {
        match self {
            FileInfo::Single(path) => path.length,
            FileInfo::Multi(files) => files.iter().map(|f| f.length).sum(),
        }
    }

    pub fn files(&self) -> Vec<&PathInfo> {
        match self {
            FileInfo::Single(path) => vec![path],
            FileInfo::Multi(files) => files.iter().collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct PathInfo {
    pub length: u32,
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
            Some(BencodeValue::String(s)) => s.chunks(20).map(|c| PieceHash(c.to_vec())).collect(),
            _ => return Err("'pieces' missing".into()),
        };
        let name: String = match info_dict.get("name") {
            Some(BencodeValue::String(s)) => String::from_utf8_lossy(s).into(),
            _ => return Err("'name' missing".into()),
        };
        let file_info = match info_dict.get("files") {
            Some(d) => FileInfo::Multi(parse_files_info(d)?),
            None => FileInfo::Single(PathInfo {
                path: PathBuf::from(&name),
                length: match info_dict.get("length") {
                    Some(BencodeValue::Int(v)) => *v as u32,
                    _ => return Err("'length' missing".into()),
                },
                md5_sum: match info_dict.get("md5_sum") {
                    Some(BencodeValue::String(s)) => Some(String::from_utf8_lossy(s).to_string()),
                    _ => None,
                },
            }),
        };
        let metainfo = Metainfo {
            info: Info {
                piece_length: match info_dict.get("piece length") {
                    Some(BencodeValue::Int(v)) => *v as u32,
                    _ => return Err("'piece length' missing".into()),
                },
                pieces,
                name,
                file_info,
                private: match info_dict.get("private") {
                    Some(BencodeValue::Int(i)) => Some(*i == 1),
                    _ => None,
                },
            },
            announce: match dict.get("announce") {
                Some(BencodeValue::String(s)) => String::from_utf8_lossy(s).into(),
                _ => return Err("'announce' missing".into()),
            },
            announce_list: match dict.get("announce-list") {
                Some(BencodeValue::List(l)) => l
                    .iter()
                    .map(|i| match i {
                        BencodeValue::List(nl) => nl
                            .iter()
                            .map(|ni| match ni {
                                BencodeValue::String(s) => Some(String::from_utf8_lossy(s).into()),
                                _ => None,
                            })
                            .collect::<Option<Vec<String>>>(),
                        _ => None,
                    })
                    .collect::<Option<_>>(),
                _ => None,
            },
            creation_date: match dict.get("creation date") {
                Some(BencodeValue::Int(i)) => Some(*i),
                _ => None,
            },
            comment: match dict.get("comment") {
                Some(BencodeValue::String(s)) => Some(String::from_utf8_lossy(s).into()),
                _ => None,
            },
            created_by: match dict.get("created by") {
                Some(BencodeValue::String(s)) => Some(String::from_utf8_lossy(s).into()),
                _ => None,
            },
            encoding: match dict.get("encoding") {
                Some(BencodeValue::String(s)) => Some(String::from_utf8_lossy(s).into()),
                _ => None,
            },
        };
        Ok(metainfo)
    }
}

fn parse_files_info(value: &BencodeValue) -> Result<Vec<PathInfo>, String> {
    match value {
        BencodeValue::List(l) => l
            .iter()
            .map(|i| match i {
                BencodeValue::Dict(d) => {
                    let path = match d.get("path") {
                        Some(BencodeValue::List(p)) => p
                            .iter()
                            .map(|dir| match dir {
                                BencodeValue::String(dir) => {
                                    Ok(PathBuf::from(String::from_utf8_lossy(dir).to_string()))
                                }
                                _ => Err("'path' item is not a string".into()),
                            })
                            .collect::<Result<PathBuf, String>>()?,
                        _ => return Err("'path' is not a list".into()),
                    };
                    Ok(PathInfo {
                        length: match d.get("length") {
                            Some(BencodeValue::Int(v)) => *v as u32,
                            _ => return Err("'length' missing".into()),
                        },
                        path,
                        md5_sum: match d.get("md5_sum") {
                            Some(BencodeValue::String(s)) => {
                                Some(String::from_utf8_lossy(s).to_string())
                            }
                            _ => None,
                        },
                    })
                }
                _ => Err("'files' item is not a dict".into()),
            })
            .collect(),
        _ => Err("'files' is not a list".into()),
    }
}
