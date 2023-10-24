use sha1::{Digest, Sha1};

use crate::types::ByteString;

pub fn encode(value: ByteString) -> ByteString {
    let mut sha = Sha1::default();
    sha.update(value);
    sha.finalize().to_vec()
}
