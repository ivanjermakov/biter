use crate::types::ByteString;

pub fn hex(str: &ByteString) -> String {
    str.iter().map(|c| format!("{:x?}", c)).collect::<String>()
}
