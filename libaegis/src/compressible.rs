#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::vec::Vec;

/// Compresses a struct with many zero-valued 64-bit fields
///
/// The output format: `mask | values`
/// - `mask`   is a bitmask indicating which **u64 chunks** are non-zero
/// - `values` are the raw u64 values for those chunks, in increasing index order
///
/// # Safety
/// Self must be **8 byte aligned**
/// size_of::<Self>() % 8 == 0
pub unsafe trait Compressible: Sized {
    const BYTES: usize = core::mem::size_of::<Self>();
    const CHUNKS: usize = Self::BYTES / 8;
    const BITMAP_BYTES: usize = (Self::CHUNKS + 7) / 8;

    /// Compresses this type into the out vector
    #[inline(always)]
    fn compress(&self, out: &mut Vec<u8>) {
        debug_assert_eq!(Self::BYTES % 8, 0);

        let words: &[u64] =
            unsafe { core::slice::from_raw_parts(self as *const Self as *const u64, Self::CHUNKS) };

        out.clear();
        out.reserve(Self::BITMAP_BYTES + Self::CHUNKS * 8);
        out.resize(Self::BITMAP_BYTES, 0);

        // Build bitmap
        for (i, &w) in words.iter().enumerate() {
            if w != 0 {
                out[i >> 3] |= 1 << (i & 7);
            }
        }

        // Emit payload
        for &w in words {
            if w != 0 {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
    }

    /// Writes this object as a set of bytes to a given
    fn to_bytes<'a>(&self, mut buff: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        debug_assert_eq!(Self::BYTES % 8, 0);

        let words: &[u64] =
            unsafe { core::slice::from_raw_parts(self as *const Self as *const u64, Self::CHUNKS) };

        if buff.len() < Self::BITMAP_BYTES {
            return Err(());
        }
        buff[..Self::BITMAP_BYTES].fill(0);

        // Build bitmap
        for (i, &w) in words.iter().enumerate() {
            if w != 0 {
                buff[i >> 3] |= 1 << (i & 7);
            }
        }

        buff = &mut buff[Self::BITMAP_BYTES..];

        // Emit payload
        for &w in words {
            if w != 0 {
                if buff.len() < 8 {
                    return Err(());
                }

                buff[..8].copy_from_slice(&w.to_le_bytes());
                buff = &mut buff[8..];
            }
        }

        Ok(buff)
    }

    /// Creates a self out of the byte array
    #[inline(always)]
    fn decompress<'a>(input: &'a [u8], out: &mut Self) -> Result<&'a [u8], ()> {
        if input.len() < Self::BITMAP_BYTES {
            return Err(());
        }

        let words: &mut [u64] =
            unsafe { core::slice::from_raw_parts_mut(out as *mut Self as *mut u64, Self::CHUNKS) };

        // Zero everything
        for w in words.iter_mut() {
            *w = 0;
        }

        unsafe {
            core::ptr::write_bytes(words.as_mut_ptr(), 0, Self::CHUNKS);
        }

        let bitmap = &input[..Self::BITMAP_BYTES];
        let mut payload = &input[Self::BITMAP_BYTES..];

        for i in 0..Self::CHUNKS {
            if (bitmap[i >> 3] >> (i & 7)) & 1 != 0 {
                if input.len() < 8 {
                    return Err(());
                }

                let (b, rest) = payload.split_at(8);
                words[i] = u64::from_le_bytes(b.try_into().unwrap());
                payload = rest;
            }
        }

        Ok(payload)
    }
}
