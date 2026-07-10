#![allow(missing_docs)]

use ring::digest;

pub fn sas_code(transcript: &[u8]) -> String {
    let mut input = b"MRD_SAS_V1".to_vec();
    input.extend_from_slice(transcript);
    let digest = digest::digest(&digest::SHA256, &input);
    let number = u32::from_be_bytes([digest.as_ref()[0], digest.as_ref()[1], digest.as_ref()[2], digest.as_ref()[3]]) % 1_000_000;
    format!("{number:06}")
}
