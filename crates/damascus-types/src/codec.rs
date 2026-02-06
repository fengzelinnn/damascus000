use bincode::Options as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("bincode encode: {0}")]
    Encode(#[from] Box<bincode::ErrorKind>),
}

pub fn bincode_options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
        .with_big_endian()
        .with_fixint_encoding()
        .with_no_limit()
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    Ok(bincode_options().serialize(value)?)
}

pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, CodecError> {
    Ok(bincode_options().deserialize(bytes)?)
}
