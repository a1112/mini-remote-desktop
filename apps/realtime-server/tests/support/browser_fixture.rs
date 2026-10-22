use mrd_identity::DeviceIdentity;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::Value;

pub fn fixture() -> Value {
    serde_json::from_str(include_str!("../fixtures/browser_protocol_v3.json")).unwrap()
}

pub fn identity(seed_hex: &str) -> DeviceIdentity {
    let seed: Vec<u8> = (0..seed_hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&seed_hex[index..index + 2], 16).unwrap())
        .collect();
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    let mut pkcs8 = vec![
        0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    pkcs8.extend_from_slice(&seed);
    pkcs8.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0]);
    pkcs8.extend_from_slice(pair.public_key().as_ref());
    DeviceIdentity::from_pkcs8(&pkcs8).unwrap()
}
