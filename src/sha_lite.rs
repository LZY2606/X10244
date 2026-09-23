//! 无外部依赖的 FNV-1a 64bit，仅用于导入完整性指纹（非加密用途）。
pub fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
