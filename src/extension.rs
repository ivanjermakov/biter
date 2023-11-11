use anyhow::{anyhow, Context, Error};

use crate::bencode::BencodeValue;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Extension {
    Metadata,
    PeerExchange,
}

impl Extension {
    pub fn id(&self) -> usize {
        match self {
            Extension::Metadata => 1,
            Extension::PeerExchange => 2,
        }
    }

    pub fn name(&self) -> String {
        match &self {
            Extension::Metadata => "ut_metadata".into(),
            Extension::PeerExchange => "ut_pex".into(),
        }
    }

    pub fn handshake(extensions: &[Extension]) -> BencodeValue {
        BencodeValue::Dict(
            [(
                "m".into(),
                BencodeValue::Dict(
                    extensions
                        .iter()
                        .enumerate()
                        .map(|(i, ext)| (ext.name(), BencodeValue::from(i as i64 + 1)))
                        .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        )
    }
}

impl TryFrom<usize> for Extension {
    type Error = Error;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        [Extension::Metadata]
            .into_iter()
            .find(|e| e.id() == value)
            .context("unknown id")
    }
}

impl TryFrom<&str> for Extension {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "ut_metadata" => Ok(Extension::Metadata),
            "ut_pex" => Ok(Extension::PeerExchange),
            _ => Err(anyhow!("unknown extension")),
        }
    }
}
