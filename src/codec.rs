//! Little-endian field helpers and CRC-32 used by every on-disk structure.
//!
//! All multi-byte integers are stored little-endian, so images are portable
//! between machines (the doublets store uses native endianness, which is
//! little-endian on every platform LFS targets).

/// Reads a little-endian `u64` at `offset`.
#[must_use]
pub fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(raw)
}

/// Writes a little-endian `u64` at `offset`.
pub fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// Reads a little-endian `u32` at `offset`.
#[must_use]
pub fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(raw)
}

/// Writes a little-endian `u32` at `offset`.
pub fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Reads a little-endian `i64` at `offset`.
#[must_use]
pub fn get_i64(bytes: &[u8], offset: usize) -> i64 {
    get_u64(bytes, offset) as i64
}

/// Writes a little-endian `i64` at `offset`.
pub fn put_i64(bytes: &mut [u8], offset: usize, value: i64) {
    put_u64(bytes, offset, value as u64);
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

static CRC32_TABLE: [u32; 256] = crc32_table();

/// CRC-32 (IEEE 802.3, the zlib/PNG polynomial).
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0, bytes)
}

/// Continues a CRC-32 computation over another slice.
#[must_use]
pub fn crc32_update(crc: u32, bytes: &[u8]) -> u32 {
    let mut value = !crc;
    for &byte in bytes {
        value = CRC32_TABLE[((value ^ u32::from(byte)) & 0xFF) as usize] ^ (value >> 8);
    }
    !value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_update(crc32(b"1234"), b"56789"), 0xCBF4_3926);
    }

    #[test]
    fn integers_round_trip() {
        let mut bytes = [0u8; 16];
        put_u64(&mut bytes, 0, 0x0102_0304_0506_0708);
        put_u32(&mut bytes, 8, 0xAABB_CCDD);
        put_i64(&mut bytes, 8, -5);
        assert_eq!(get_u64(&bytes, 0), 0x0102_0304_0506_0708);
        assert_eq!(bytes[0], 0x08);
        assert_eq!(get_i64(&bytes, 8), -5);
        put_u32(&mut bytes, 12, 7);
        assert_eq!(get_u32(&bytes, 12), 7);
    }
}
