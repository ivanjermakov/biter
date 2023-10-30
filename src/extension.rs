use crate::bencode::BencodeValue;

pub enum Extension {
    Metadata,
}

impl Extension {
    pub fn name(&self) -> String {
        match &self {
            Extension::Metadata => "ut_metadata".into(),
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

