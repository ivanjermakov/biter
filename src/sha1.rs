use sha1::{Sha1, Digest};

use crate::types::ByteString;

pub fn encode(value: ByteString) -> ByteString {
    let mut sha = Sha1::default();
    sha.update(value);
    sha.finalize().to_vec()
}
