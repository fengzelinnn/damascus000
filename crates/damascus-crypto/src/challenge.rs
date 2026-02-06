use damascus_types::codec;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ChallengeError {
    #[error("q modulus must be >= 2")]
    BadModulus,
    #[error("failed to encode input: {0}")]
    Codec(#[from] codec::CodecError),
    #[error("value has no inverse mod q (value={value}, q={q})")]
    NoInverse { value: u64, q: u64 },
}

pub fn hash_to_nonzero_mod_q(domain: &[u8], input: &[u8], q: u64) -> Result<u64, ChallengeError> {
    if q < 2 {
        return Err(ChallengeError::BadModulus);
    }

    let mut counter: u32 = 0;
    loop {
        let mut bytes = Vec::with_capacity(domain.len() + input.len() + 4);
        bytes.extend_from_slice(domain);
        bytes.extend_from_slice(input);
        bytes.extend_from_slice(&counter.to_be_bytes());

        let hash = blake3::hash(&bytes);
        let mut limb = [0u8; 8];
        limb.copy_from_slice(&hash.as_bytes()[0..8]);
        let x = u64::from_be_bytes(limb) % q;
        if x != 0 {
            return Ok(x);
        }
        counter = counter.wrapping_add(1);
    }
}

pub fn mod_inv(value: u64, q: u64) -> Result<u64, ChallengeError> {
    let (g, x, _) = egcd_i128(value as i128, q as i128);
    if g != 1 {
        return Err(ChallengeError::NoInverse { value, q });
    }
    let mut inv = x % (q as i128);
    if inv < 0 {
        inv += q as i128;
    }
    Ok(inv as u64)
}

fn egcd_i128(a: i128, b: i128) -> (i128, i128, i128) {
    if a == 0 {
        return (b, 0, 1);
    }
    let (g, x, y) = egcd_i128(b % a, a);
    (g, y - (b / a) * x, x)
}

pub fn encode_many<T: Serialize>(values: &T) -> Result<Vec<u8>, ChallengeError> {
    Ok(codec::encode(values)?)
}
