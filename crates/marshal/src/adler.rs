//! Adler-32, the checksum a marshal stream may carry.

const MODULUS: u32 = 65_521;

/// Largest number of bytes that can be summed before the running totals must
/// be reduced to stay inside a `u32` (the value zlib uses).
const BLOCK: usize = 5_552;

/// Adler-32 of `data`, starting from the standard initial value of 1.
pub fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for block in data.chunks(BLOCK) {
        for &byte in block {
            a += u32::from(byte);
            b += a;
        }
        a %= MODULUS;
        b %= MODULUS;
    }
    // The two halves never overlap: both are below 65,521.
    (b << 16) + a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_one() {
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn matches_known_values() {
        // Reference values from zlib.adler32.
        assert_eq!(adler32(b"a"), 0x0062_0062);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn long_input_does_not_overflow() {
        // 1 MiB of 0xFF is the worst case for the running sums.
        let data = vec![0xFF_u8; 1 << 20];
        assert_eq!(adler32(&data), 0x8E88_EF11);
    }
}
