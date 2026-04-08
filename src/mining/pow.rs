use anyhow::Result;
use starcoin_consensus::Consensus;
use starcoin_crypto::HashValue;
use starcoin_types::block::BlockHeaderExtra;
use starcoin_types::genesis_config::ConsensusStrategy;
use starcoin_types::U256;

pub fn calculate_pow_hash(
    strategy: ConsensusStrategy,
    blob: &[u8],
    nonce: u32,
    extra: &BlockHeaderExtra,
) -> Result<HashValue> {
    strategy.calculate_pow_hash(blob, nonce, extra)
}

pub fn hash_meets_target(hash: &HashValue, share_target: U256) -> bool {
    let hash_u256: U256 = (*hash).into();
    hash_u256 <= share_target
}

pub fn nonce_to_hex(nonce: u32) -> String {
    hex::encode(nonce.to_le_bytes())
}

pub fn hash_to_result_hex(hash: &HashValue) -> String {
    let mut bytes = hash.to_vec();
    bytes.reverse();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use starcoin_crypto::HashValue;
    use starcoin_types::U256;

    #[test]
    fn nonce_hex_is_little_endian() {
        assert_eq!(nonce_to_hex(0), "00000000");
        assert_eq!(nonce_to_hex(0x12345678), "78563412");
    }

    #[test]
    fn result_hex_is_little_endian() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0x11;
        bytes[31] = 0xaa;
        let hash = HashValue::from_slice(bytes).expect("hash");
        let result = hash_to_result_hex(&hash);
        assert!(result.starts_with("aa"));
        assert!(result.ends_with("11"));
    }

    #[test]
    fn target_check_uses_u256_ordering() {
        let low = HashValue::zero();
        assert!(hash_meets_target(&low, U256::from(1u64)));
    }
}
