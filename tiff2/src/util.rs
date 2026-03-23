use crate::ByteOrder;

/// Fix endianness. use the constant NATIVE_ENDIAN for conversion to the host
///
/// ```
/// use tiff2::{NATIVE_ENDIAN, ByteOrder};
/// use tiff2::util::fix_endianness;
/// // assuming we're on a little-endian system
/// let mut val = 42u64.to_be_bytes();
/// fix_endianness(&mut val, ByteOrder::BigEndian, NATIVE_ENDIAN, 64);
/// assert_eq!(u64::from_ne_bytes(val), 42);
/// ```
pub fn fix_endianness(
    buf: &mut [u8],
    endianness: ByteOrder,
    target_endianness: ByteOrder,
    bit_depth: u16,
) {
    if endianness != target_endianness {
        // so the only thing is that we have to swap
        match bit_depth {
            0..=8 => {}
            9..=16 => buf.chunks_exact_mut(2).for_each(|v| {
                v.copy_from_slice(&u16::from_le_bytes((*v).try_into().unwrap()).to_be_bytes())
            }),
            17..=32 => buf.chunks_exact_mut(4).for_each(|v| {
                v.copy_from_slice(&u32::from_le_bytes((*v).try_into().unwrap()).to_be_bytes())
            }),
            _ => buf.chunks_exact_mut(8).for_each(|v| {
                v.copy_from_slice(&u64::from_le_bytes((*v).try_into().unwrap()).to_be_bytes())
            }),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_fix_endianness() {
        let mut u64_be = 42u64.to_be_bytes();
        fix_endianness(
            &mut u64_be,
            ByteOrder::BigEndian,
            ByteOrder::LittleEndian,
            64,
        );
        assert_eq!(u64_be, 42u64.to_le_bytes());
        fix_endianness(
            &mut u64_be,
            ByteOrder::LittleEndian,
            ByteOrder::LittleEndian,
            64,
        );
        assert_eq!(u64_be, 42u64.to_le_bytes());
        fix_endianness(
            &mut u64_be,
            ByteOrder::LittleEndian,
            ByteOrder::BigEndian,
            64,
        );
        assert_eq!(u64_be, 42u64.to_be_bytes());
        let mut u32_be = 42u32.to_be_bytes();
        fix_endianness(
            &mut u32_be,
            ByteOrder::BigEndian,
            ByteOrder::LittleEndian,
            32,
        );
        assert_eq!(u32_be, 42u32.to_le_bytes());
        fix_endianness(
            &mut u32_be,
            ByteOrder::LittleEndian,
            ByteOrder::BigEndian,
            32,
        );
        assert_eq!(u32_be, 42u32.to_be_bytes());
        let mut u16_be = 42u16.to_be_bytes();
        fix_endianness(
            &mut u16_be,
            ByteOrder::BigEndian,
            ByteOrder::LittleEndian,
            16,
        );
        assert_eq!(u16_be, 42u16.to_le_bytes());
        fix_endianness(
            &mut u16_be,
            ByteOrder::LittleEndian,
            ByteOrder::BigEndian,
            16,
        );
        assert_eq!(u16_be, 42u16.to_be_bytes());
    }
}
