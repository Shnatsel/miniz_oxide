//! Streaming compression functionality.

use std::{cmp, mem, ptr};
use std::io::{self, Cursor, Seek, SeekFrom, Write};

use super::CompressionLevel;
use super::deflate_flags::*;
use super::super::*;
use shared::{HUFFMAN_LENGTH_ORDER, MZ_ADLER32_INIT, update_adler32};
use deflate::buffer::{HashBuffers, LZ_CODE_BUF_SIZE, OUT_BUF_SIZE, LocalBuf};

const MAX_PROBES_MASK: i32 = 0xFFF;

const MAX_SUPPORTED_HUFF_CODESIZE: usize = 32;

/// Length code for length values.
#[cfg_attr(rustfmt, rustfmt_skip)]
const LEN_SYM: [u16; 256] = [
    257, 258, 259, 260, 261, 262, 263, 264, 265, 265, 266, 266, 267, 267, 268, 268,
    269, 269, 269, 269, 270, 270, 270, 270, 271, 271, 271, 271, 272, 272, 272, 272,
    273, 273, 273, 273, 273, 273, 273, 273, 274, 274, 274, 274, 274, 274, 274, 274,
    275, 275, 275, 275, 275, 275, 275, 275, 276, 276, 276, 276, 276, 276, 276, 276,
    277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277,
    278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278,
    279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279,
    280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280,
    281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281,
    281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281,
    282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282,
    282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282,
    283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283,
    283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283,
    284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284,
    284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 285
];

/// Number of extra bits for length values.
#[cfg_attr(rustfmt, rustfmt_skip)]
const LEN_EXTRA: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 0
];

/// Distance codes for distances smaller than 512.
#[cfg_attr(rustfmt, rustfmt_skip)]
const SMALL_DIST_SYM: [u8; 512] = [
     0,  1,  2,  3,  4,  4,  5,  5,  6,  6,  6,  6,  7,  7,  7,  7,
     8,  8,  8,  8,  8,  8,  8,  8,  9,  9,  9,  9,  9,  9,  9,  9,
    10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10,
    11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17
];

/// Number of extra bits for distances smaller than 512.
#[cfg_attr(rustfmt, rustfmt_skip)]
const SMALL_DIST_EXTRA: [u8; 512] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7
];

/// Base values to calculate distances above 512.
#[cfg_attr(rustfmt, rustfmt_skip)]
const LARGE_DIST_SYM: [u8; 128] = [
     0,  0, 18, 19, 20, 20, 21, 21, 22, 22, 22, 22, 23, 23, 23, 23,
    24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 25,
    26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29
];

/// Number of extra bits distances above 512.
#[cfg_attr(rustfmt, rustfmt_skip)]
const LARGE_DIST_EXTRA: [u8; 128] = [
     0,  0,  8,  8,  9,  9,  9,  9, 10, 10, 10, 10, 10, 10, 10, 10,
    11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13
];

#[cfg_attr(rustfmt, rustfmt_skip)]
const BITMASKS: [u32; 17] = [
    0x0000, 0x0001, 0x0003, 0x0007, 0x000F, 0x001F, 0x003F, 0x007F, 0x00FF,
    0x01FF, 0x03FF, 0x07FF, 0x0FFF, 0x1FFF, 0x3FFF, 0x7FFF, 0xFFFF
];

/// The maximum number of checks for matches in the hash table the compressor will make for each
/// compression level.
const NUM_PROBES: [u32; 11] = [0, 1, 6, 32, 16, 32, 128, 256, 512, 768, 1500];

#[derive(Copy, Clone)]
struct SymFreq {
    key: u16,
    sym_index: u16,
}

/// Compression callback function type.
pub type PutBufFuncPtrNotNull = unsafe extern "C" fn(*const c_void, c_int, *mut c_void) -> bool;
/// `Option` alias for compression callback function type.
pub type PutBufFuncPtr = Option<PutBufFuncPtrNotNull>;

pub mod deflate_flags {
    /// Whether to use a zlib wrapper.
    pub const TDEFL_WRITE_ZLIB_HEADER: u32 = 0x0000_1000;
    /// Should we compute the adler32 checksum.
    pub const TDEFL_COMPUTE_ADLER32: u32 = 0x0000_2000;
    /// Should we use greedy parsing (as opposed to lazy parsing where look ahead one or more
    /// bytes to check for better matches.)
    pub const TDEFL_GREEDY_PARSING_FLAG: u32 = 0x0000_4000;
    /// TODO
    pub const TDEFL_NONDETERMINISTIC_PARSING_FLAG: u32 = 0x0000_8000;
    /// Only look for matches with a distance of 0.
    pub const TDEFL_RLE_MATCHES: u32 = 0x0001_0000;
    /// Only use matches that are at least 6 bytes long.
    pub const TDEFL_FILTER_MATCHES: u32 = 0x0002_0000;
    /// Force the compressor to only output static blocks. (Blocks using the default huffman codes
    /// specified in the deflate specification.)
    pub const TDEFL_FORCE_ALL_STATIC_BLOCKS: u32 = 0x0004_0000;
    /// Force the compressor to only output raw/uncompressed blocks.
    pub const TDEFL_FORCE_ALL_RAW_BLOCKS: u32 = 0x0008_0000;
}

/// Used to generate deflate flags with `create_comp_flags_from_zip_params`.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CompressionStrategy {
    /// Don't use any of the special strategies.
    Default = 0,
    /// Only use matches that are at least 5 bytes long.
    Filtered = 1,
    /// Don't look for matches, only huffman encode the literals.
    HuffmanOnly = 2,
    /// Only look for matches with a distance of 1, i.e do run-length encoding only.
    RLE = 3,
    /// Only use static/fixed blocks. (Blocks using the default huffman codes
    /// specified in the deflate specification.)
    Fixed = 4,
}

/// A list of deflate flush types.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TDEFLFlush {
    None = 0,
    Sync = 2,
    Full = 3,
    Finish = 4,
}

impl From<MZFlush> for TDEFLFlush {
    fn from(flush: MZFlush) -> Self {
        match flush {
            MZFlush::None => TDEFLFlush::None,
            MZFlush::Sync => TDEFLFlush::Sync,
            MZFlush::Full => TDEFLFlush::Full,
            MZFlush::Finish => TDEFLFlush::Finish,
            _ => TDEFLFlush::None, // TODO: ??? What to do ???
        }
    }
}

impl TDEFLFlush {
    pub fn new(flush: c_int) -> Result<Self, MZError> {
        match flush {
            0 => Ok(TDEFLFlush::None),
            2 => Ok(TDEFLFlush::Sync),
            3 => Ok(TDEFLFlush::Full),
            4 => Ok(TDEFLFlush::Finish),
            _ => Err(MZError::Param),
        }
    }
}

/// Return status codes.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TDEFLStatus {
    BadParam = -2,
    PutBufFailed = -1,
    Okay = 0,
    Done = 1,
}

const MAX_HUFF_SYMBOLS: usize = 288;
/// Size of hash values in the hash chains.
const LZ_HASH_BITS: i32 = 15;
/// Size of hash chain for fast compression mode.
const LEVEL1_HASH_SIZE_MASK: u32 = 4095;
/// How many bits to shift when updating the current hash value.
const LZ_HASH_SHIFT: i32 = (LZ_HASH_BITS + 2) / 3;
/// Size of the chained hash tables.
const LZ_HASH_SIZE: usize = 1 << LZ_HASH_BITS;

/// The number of huffman tables used by the compressor.
/// Literal/length, Distances and Length of the huffman codes for the other two tables.
const MAX_HUFF_TABLES: usize = 3;
/// Literal/length codes
const MAX_HUFF_SYMBOLS_0: usize = 288;
/// Distance codes.
const MAX_HUFF_SYMBOLS_1: usize = 32;
/// Huffman length values.
const MAX_HUFF_SYMBOLS_2: usize = 19;
/// Size of the chained hash table.
pub const LZ_DICT_SIZE: usize = 32_768;
/// Mask used when stepping through the hash chains.
const LZ_DICT_SIZE_MASK: u32 = LZ_DICT_SIZE as u32 - 1;
/// The minimum length of a match.
const MIN_MATCH_LEN: u32 = 3;
/// The maximum length of a match.
pub const MAX_MATCH_LEN: usize = 258;

fn memset<T: Copy>(slice: &mut [T], val: T) {
    for x in slice {
        *x = val
    }
}

#[cfg(all(target_endian = "little", test))]
#[inline]
fn write_u16_le(val: u16, slice: &mut [u8], pos: usize) {
    assert!(pos + 1 < slice.len());
    assert!(pos < slice.len());
    // # Unsafe
    // We just checked that there is space.
    unsafe {
        ptr::write_unaligned(slice.as_mut_ptr() as *mut u16, val);
    }
}

#[cfg(all(target_endian = "big", test))]
#[inline]
fn write_u16_le(val: u16, slice: &mut [u8], pos: usize) {
    slice[pos] = val as u8;
    slice[pos + 1] = (val >> 8) as u8;
}

#[cfg(target_endian = "little")]
#[inline]
unsafe fn write_u16_le_uc(val: u16, slice: &mut [u8], pos: usize) {
    ptr::write_unaligned(slice.as_mut_ptr().offset(pos as isize) as *mut u16, val);
}

#[cfg(target_endian = "big")]
#[inline]
unsafe fn write_u16_le_uc(val: u16, slice: &mut [u8], pos: usize) {
    *slice.as_mut_ptr().offset(pos as isize) = val as u8;
    *slice.as_mut_ptr().offset(pos as isize + 1) = (val >> 8) as u8;
}


#[cfg(target_endian = "little")]
#[inline]
fn read_u16_le(slice: &[u8], pos: usize) -> u16{
    assert!(pos + 1 < slice.len());
    assert!(pos < slice.len());
    // # Unsafe
    // We just checked that there is space.
    // We also make the assumption here that slice can't be longer than isize::max.
    unsafe {
        ptr::read_unaligned(slice.as_ptr().offset(pos as isize) as *mut u16)
    }
}

#[cfg(target_endian = "big")]
#[inline]
fn read_u16_le(slice: &[u8], pos: usize) -> u16{
    slice[pos] as u16 | ((slice[pos + 1] as u16) << 8)
}

/// Main compression struct.
pub struct CompressorOxide {
    lz: LZOxide,
    params: ParamsOxide,
    huff: Box<HuffmanOxide>,
    dict: DictOxide,
}

impl CompressorOxide {
    pub fn new(flags: u32) -> Self {
        CompressorOxide {
            lz: LZOxide::new(),
            params: ParamsOxide::new(flags),
            /// Put HuffmanOxide on the heap with default trick to avoid
            /// excessive stack copies.
            huff: Box::default(),
            dict: DictOxide::new(flags),
        }
    }

    pub fn adler32(&self) -> u32 {
        self.params.adler32
    }

    pub fn prev_return_status(&self) -> TDEFLStatus {
        self.params.prev_return_status
    }

    pub fn flags(&self) -> i32 {
        self.params.flags as i32
    }
}

/// Callback function and user used in `compress_to_output`.
#[derive(Copy, Clone)]
pub struct CallbackFunc {
    pub put_buf_func: PutBufFuncPtrNotNull,
    pub put_buf_user: *mut c_void,
}

impl CallbackFunc {
    fn flush_output(
        &mut self,
        saved_output: SavedOutputBufferOxide,
        params: &mut ParamsOxide,
    ) -> i32 {
        // TODO: As this could be unsafe since
        // we can't verify the function pointer
        // this whole function should maybe be unsafe as well.
        let call_success = unsafe {
            (self.put_buf_func)(
                &params.local_buf.b[0] as *const u8 as *const c_void,
                saved_output.pos as i32,
                self.put_buf_user,
            )
        };

        if !call_success {
            params.prev_return_status = TDEFLStatus::PutBufFailed;
            return params.prev_return_status as i32;
        }

        params.flush_remaining as i32
    }
}

struct CallbackBuf<'a> {
    pub out_buf: &'a mut [u8],
}

impl<'a> CallbackBuf<'a> {
    fn flush_output(
        &mut self,
        saved_output: SavedOutputBufferOxide,
        params: &mut ParamsOxide,
    ) -> i32 {
        if saved_output.local {
            let n = cmp::min(
                saved_output.pos as usize,
                self.out_buf.len() - params.out_buf_ofs,
            );
            (&mut self.out_buf[params.out_buf_ofs..params.out_buf_ofs + n])
                .copy_from_slice(&params.local_buf.b[..n]);

            params.out_buf_ofs += n;
            if saved_output.pos != n as u64 {
                params.flush_ofs = n as u32;
                params.flush_remaining = (saved_output.pos - n as u64) as u32;
            }
        } else {
            params.out_buf_ofs += saved_output.pos as usize;
        }

        params.flush_remaining as i32
    }
}

enum CallbackOut<'a> {
    Func(CallbackFunc),
    Buf(CallbackBuf<'a>),
}

impl<'a> CallbackOut<'a> {
    fn new_output_buffer<'b>(
        &'b mut self,
        local_buf: &'b mut [u8],
        out_buf_ofs: usize,
    ) -> OutputBufferOxide<'b> {
        let is_local;
        let buf_len = OUT_BUF_SIZE - 16;
        let chosen_buffer = match *self {
            CallbackOut::Buf(ref mut cb)
                if cb.out_buf.len() - out_buf_ofs >= OUT_BUF_SIZE => {
                is_local = false;
                &mut cb.out_buf[out_buf_ofs..out_buf_ofs + buf_len]
            }
            _ => {
                is_local = true;
                &mut local_buf[..buf_len]
            }
        };

        let cursor = Cursor::new(chosen_buffer);
        OutputBufferOxide {
            inner: cursor,
            local: is_local,
            bit_buffer: 0,
            bits_in: 0,
        }
    }
}

struct CallbackOxide<'a> {
    in_buf: Option<&'a [u8]>,
    in_buf_size: Option<&'a mut usize>,
    out_buf_size: Option<&'a mut usize>,
    out: CallbackOut<'a>,
}

impl<'a> CallbackOxide<'a> {
    fn new_callback_buf(in_buf: &'a [u8], out_buf: &'a mut [u8]) -> Self {
        CallbackOxide {
            in_buf: Some(in_buf),
            in_buf_size: None,
            out_buf_size: None,
            out: CallbackOut::Buf(CallbackBuf { out_buf: out_buf }),
        }
    }

    fn new_callback_func(in_buf: &'a [u8], callback_func: CallbackFunc) -> Self {
        CallbackOxide {
            in_buf: Some(in_buf),
            in_buf_size: None,
            out_buf_size: None,
            out: CallbackOut::Func(callback_func),
        }
    }

    fn update_size(&mut self, in_size: Option<usize>, out_size: Option<usize>) {
        if let Some(in_size) = in_size {
            self.in_buf_size.as_mut().map(|size| **size = in_size);
        }

        if let Some(out_size) = out_size {
            self.out_buf_size.as_mut().map(|size| **size = out_size);
        }
    }

    fn flush_output(
        &mut self,
        saved_output: SavedOutputBufferOxide,
        params: &mut ParamsOxide,
    ) -> i32 {
        if saved_output.pos == 0 {
            return params.flush_remaining as i32;
        }

        self.update_size(Some(params.src_pos), None);
        match self.out {
            CallbackOut::Func(ref mut cf) => cf.flush_output(saved_output, params),
            CallbackOut::Buf(ref mut cb) => cb.flush_output(saved_output, params),
        }
    }
}

struct OutputBufferOxide<'a> {
    pub inner: Cursor<&'a mut [u8]>,
    pub local: bool,

    pub bit_buffer: u32,
    pub bits_in: u32,
}

impl<'a> OutputBufferOxide<'a> {
    fn put_bits(&mut self, bits: u32, len: u32) {
        assert!(bits <= ((1u32 << len) - 1u32));
        self.bit_buffer |= bits << self.bits_in;
        self.bits_in += len;
        while self.bits_in >= 8 {
            let pos = self.inner.position();
            self.inner.get_mut()[pos as usize] = self.bit_buffer as u8;
            self.inner.set_position(pos + 1);
            self.bit_buffer >>= 8;
            self.bits_in -= 8;
        }
    }

    fn save(&self) -> SavedOutputBufferOxide {
        SavedOutputBufferOxide {
            pos: self.inner.position(),
            bit_buffer: self.bit_buffer,
            bits_in: self.bits_in,
            local: self.local,
        }
    }

    fn load(&mut self, saved: SavedOutputBufferOxide) {
        self.inner.set_position(saved.pos);
        self.bit_buffer = saved.bit_buffer;
        self.bits_in = saved.bits_in;
        self.local = saved.local;
    }

    fn pad_to_bytes(&mut self) {
        if self.bits_in != 0 {
            let len = 8 - self.bits_in;
            self.put_bits(0, len);
        }
    }
}

struct SavedOutputBufferOxide {
    pub pos: u64,
    pub bit_buffer: u32,
    pub bits_in: u32,
    pub local: bool,
}

struct BitBuffer {
    pub bit_buffer: u64,
    pub bits_in: u32,
}

impl BitBuffer {
    fn put_fast(&mut self, bits: u64, len: u32) {
        self.bit_buffer |= bits << self.bits_in;
        self.bits_in += len;
    }

    fn flush(&mut self, output: &mut OutputBufferOxide) -> io::Result<()> {
        let pos = output.inner.position() as usize;
        let inner = &mut ((*output.inner.get_mut())[pos]) as *mut u8 as *mut u64;
        // # Unsafe
        // TODO: check unsafety
        unsafe {
            ptr::write_unaligned(inner, self.bit_buffer.to_le());
        }
        output.inner.seek(
            SeekFrom::Current((self.bits_in >> 3) as i64),
        )?;
        self.bit_buffer >>= self.bits_in & !7;
        self.bits_in &= 7;
        Ok(())
    }
}

/// A struct containing data about huffman codes and symbol frequencies.
///
/// NOTE: Only the literal/lengths have enough symbols to actually use
/// the full array. It's unclear why it's defined like this in miniz,
/// it could be for cache/alignment reasons.
struct HuffmanOxide {
    /// Number of occurrences of each symbol.
    pub count: [[u16; MAX_HUFF_SYMBOLS]; MAX_HUFF_TABLES],
    /// The bits of the huffman code assigned to the symbol
    pub codes: [[u16; MAX_HUFF_SYMBOLS]; MAX_HUFF_TABLES],
    /// The length of the huffman code assigned to the symbol.
    pub code_sizes: [[u8; MAX_HUFF_SYMBOLS]; MAX_HUFF_TABLES],
}

/// Tables used for literal/lengths in `HuffmanOxide`.
const LITLEN_TABLE: usize = 0;
/// Tables for distances.
const DIST_TABLE: usize = 1;
/// Tables for the run-length encoded huffman lenghts for literals/lengths/distances.
const HUFF_CODES_TABLE: usize = 2;

/// Status of RLE encoding of huffman code lengths.
struct RLE {
    pub z_count: u32,
    pub repeat_count: u32,
    pub prev_code_size: u8,
}

impl RLE {
    fn prev_code_size(
        &mut self,
        packed_code_sizes: &mut Cursor<&mut [u8]>,
        h: &mut HuffmanOxide,
    ) -> io::Result<()> {
        let counts = &mut h.count[HUFF_CODES_TABLE];
        if self.repeat_count != 0 {
            if self.repeat_count < 3 {
                counts[self.prev_code_size as usize] = counts[self.prev_code_size as usize]
                    .wrapping_add(self.repeat_count as u16);
                let code = self.prev_code_size;
                packed_code_sizes.write_all(
                    &[code, code, code][..self.repeat_count as
                                            usize],
                )?;
            } else {
                counts[16] = counts[16].wrapping_add(1);
                packed_code_sizes.write_all(
                    &[16, (self.repeat_count - 3) as u8][..],
                )?;
            }
            self.repeat_count = 0;
        }

        Ok(())
    }

    fn zero_code_size(
        &mut self,
        packed_code_sizes: &mut Cursor<&mut [u8]>,
        h: &mut HuffmanOxide,
    ) -> io::Result<()> {
        let counts = &mut h.count[HUFF_CODES_TABLE];
        if self.z_count != 0 {
            if self.z_count < 3 {
                counts[0] = counts[0].wrapping_add(self.z_count as u16);
                packed_code_sizes.write_all(
                    &[0, 0, 0][..self.z_count as usize],
                )?;
            } else if self.z_count <= 10 {
                counts[17] = counts[17].wrapping_add(1);
                packed_code_sizes.write_all(
                    &[17, (self.z_count - 3) as u8][..],
                )?;
            } else {
                counts[18] = counts[18].wrapping_add(1);
                packed_code_sizes.write_all(
                    &[18, (self.z_count - 11) as u8][..],
                )?;
            }
            self.z_count = 0;
        }

        Ok(())
    }
}

impl Default for HuffmanOxide {
    fn default() -> Self {
        HuffmanOxide {
            count: [[0; MAX_HUFF_SYMBOLS]; MAX_HUFF_TABLES],
            codes: [[0; MAX_HUFF_SYMBOLS]; MAX_HUFF_TABLES],
            code_sizes: [[0; MAX_HUFF_SYMBOLS]; MAX_HUFF_TABLES],
        }
    }
}

impl HuffmanOxide {
    fn radix_sort_symbols<'a>(
        symbols0: &'a mut [SymFreq],
        symbols1: &'a mut [SymFreq],
    ) -> &'a mut [SymFreq] {
        let mut hist = [[0; 256]; 2];

        for freq in symbols0.iter() {
            hist[0][(freq.key & 0xFF) as usize] += 1;
            hist[1][((freq.key >> 8) & 0xFF) as usize] += 1;
        }

        let mut n_passes = 2;
        if symbols0.len() == hist[1][0] {
            n_passes -= 1;
        }

        let mut current_symbols = symbols0;
        let mut new_symbols = symbols1;

        for pass in 0..n_passes {
            let mut offsets = [0; 256];
            let mut offset = 0;
            for i in 0..256 {
                offsets[i] = offset;
                offset += hist[pass][i];
            }

            for sym in current_symbols.iter() {
                let j = ((sym.key >> (pass * 8)) & 0xFF) as usize;
                new_symbols[offsets[j]] = *sym;
                offsets[j] += 1;
            }

            mem::swap(&mut current_symbols, &mut new_symbols);
        }

        current_symbols
    }

    fn calculate_minimum_redundancy(symbols: &mut [SymFreq]) {
        match symbols.len() {
            0 => (),
            1 => symbols[0].key = 1,
            n => {
                symbols[0].key += symbols[1].key;
                let mut root = 0;
                let mut leaf = 2;
                for next in 1..n - 1 {
                    if (leaf >= n) || (symbols[root].key < symbols[leaf].key) {
                        symbols[next].key = symbols[root].key;
                        symbols[root].key = next as u16;
                        root += 1;
                    } else {
                        symbols[next].key = symbols[leaf].key;
                        leaf += 1;
                    }

                    if (leaf >= n) || (root < next && symbols[root].key < symbols[leaf].key) {
                        symbols[next].key = symbols[next].key.wrapping_add(symbols[root].key);
                        symbols[root].key = next as u16;
                        root += 1;
                    } else {
                        symbols[next].key = symbols[next].key.wrapping_add(symbols[leaf].key);
                        leaf += 1;
                    }
                }

                symbols[n - 2].key = 0;
                for next in (0..n - 2).rev() {
                    symbols[next].key = symbols[symbols[next].key as usize].key + 1;
                }

                let mut avbl = 1;
                let mut used = 0;
                let mut dpth = 0;
                let mut root = (n - 2) as i32;
                let mut next = (n - 1) as i32;
                while avbl > 0 {
                    while (root >= 0) && (symbols[root as usize].key == dpth) {
                        used += 1;
                        root -= 1;
                    }
                    while avbl > used {
                        symbols[next as usize].key = dpth;
                        next -= 1;
                        avbl -= 1;
                    }
                    avbl = 2 * used;
                    dpth += 1;
                    used = 0;
                }
            }
        }
    }

    fn enforce_max_code_size(
        num_codes: &mut [i32],
        code_list_len: usize,
        max_code_size: usize,
    ) {
        if code_list_len <= 1 {
            return;
        }

        num_codes[max_code_size] += num_codes[max_code_size + 1..].iter().sum::<i32>();
        let total = num_codes[1..max_code_size + 1]
            .iter()
            .rev()
            .enumerate()
            .fold(0u32, |total, (i, &x)| total + ((x as u32) << i));

        for _ in (1 << max_code_size)..total {
            num_codes[max_code_size] -= 1;
            for i in (1..max_code_size).rev() {
                if num_codes[i] != 0 {
                    num_codes[i] -= 1;
                    num_codes[i + 1] += 2;
                    break;
                }
            }
        }
    }

    fn optimize_table(
        &mut self,
        table_num: usize,
        table_len: usize,
        code_size_limit: usize,
        static_table: bool,
    ) {
        let mut num_codes = [0 as i32; MAX_SUPPORTED_HUFF_CODESIZE + 1];
        let mut next_code = [0 as u32; MAX_SUPPORTED_HUFF_CODESIZE + 1];

        if static_table {
            for &code_size in &self.code_sizes[table_num][..table_len] {
                num_codes[code_size as usize] += 1;
            }
        } else {
            let mut symbols0 = [SymFreq {
                key: 0,
                sym_index: 0,
            }; MAX_HUFF_SYMBOLS];
            let mut symbols1 = [SymFreq {
                key: 0,
                sym_index: 0,
            }; MAX_HUFF_SYMBOLS];

            let mut num_used_symbols = 0;
            for i in 0..table_len {
                if self.count[table_num][i] != 0 {
                    symbols0[num_used_symbols] = SymFreq {
                        key: self.count[table_num][i],
                        sym_index: i as u16,
                    };
                    num_used_symbols += 1;
                }
            }

            let symbols = Self::radix_sort_symbols(
                &mut symbols0[..num_used_symbols],
                &mut symbols1[..num_used_symbols],
            );
            Self::calculate_minimum_redundancy(symbols);

            for symbol in symbols.iter() {
                num_codes[symbol.key as usize] += 1;
            }

            Self::enforce_max_code_size(&mut num_codes, num_used_symbols, code_size_limit);

            memset(&mut self.code_sizes[table_num][..], 0);
            memset(&mut self.codes[table_num][..], 0);

            let mut last = num_used_symbols;
            for i in 1..code_size_limit + 1 {
                let first = last - num_codes[i] as usize;
                for symbol in &symbols[first..last] {
                    self.code_sizes[table_num][symbol.sym_index as usize] = i as u8;
                }
                last = first;
            }
        }

        let mut j = 0;
        next_code[1] = 0;
        for i in 2..code_size_limit + 1 {
            j = (j + num_codes[i - 1]) << 1;
            next_code[i] = j as u32;
        }

        for (&code_size, huff_code) in
            self.code_sizes[table_num].iter().take(table_len).zip(
                self.codes[table_num].iter_mut().take(table_len),
            )
        {
            if code_size == 0 {
                continue;
            }

            let mut code = next_code[code_size as usize];
            next_code[code_size as usize] += 1;

            let mut rev_code = 0;
            for _ in 0..code_size {
                rev_code = (rev_code << 1) | (code & 1);
                code >>= 1;
            }
            *huff_code = rev_code as u16;
        }
    }

    fn start_static_block(&mut self, output: &mut OutputBufferOxide) {
        memset(&mut self.code_sizes[LITLEN_TABLE][0..144], 8);
        memset(&mut self.code_sizes[LITLEN_TABLE][144..256], 9);
        memset(&mut self.code_sizes[LITLEN_TABLE][256..280], 7);
        memset(&mut self.code_sizes[LITLEN_TABLE][280..288], 8);

        memset(&mut self.code_sizes[DIST_TABLE][..32], 5);

        self.optimize_table(LITLEN_TABLE, 288, 15, true);
        self.optimize_table(DIST_TABLE, 32, 15, true);

        output.put_bits(0b01, 2)
    }

    fn start_dynamic_block(&mut self, output: &mut OutputBufferOxide) -> io::Result<()> {
        // There will always be one, and only one end of block code.
        self.count[0][256] = 1;

        self.optimize_table(0, MAX_HUFF_SYMBOLS_0, 15, false);
        self.optimize_table(1, MAX_HUFF_SYMBOLS_1, 15, false);

        let num_lit_codes = 286 -
            &self.code_sizes[0][257..286]
                .iter()
                .rev()
                .take_while(|&x| *x == 0)
                .count();

        let num_dist_codes = 30 -
            &self.code_sizes[1][1..30]
                .iter()
                .rev()
                .take_while(|&x| *x == 0)
                .count();

        let mut code_sizes_to_pack = [0u8; MAX_HUFF_SYMBOLS_0 + MAX_HUFF_SYMBOLS_1];
        let mut packed_code_sizes = [0u8; MAX_HUFF_SYMBOLS_0 + MAX_HUFF_SYMBOLS_1];

        let total_code_sizes_to_pack = num_lit_codes + num_dist_codes;

        code_sizes_to_pack[..num_lit_codes].copy_from_slice(&self.code_sizes[0][..num_lit_codes]);

        code_sizes_to_pack[num_lit_codes..total_code_sizes_to_pack]
            .copy_from_slice(&self.code_sizes[1][..num_dist_codes]);

        let mut rle = RLE {
            z_count: 0,
            repeat_count: 0,
            prev_code_size: 0xFF,
        };

        memset(
            &mut self.count[HUFF_CODES_TABLE][..MAX_HUFF_SYMBOLS_2],
            0,
        );

        let mut packed_code_sizes_cursor = Cursor::new(&mut packed_code_sizes[..]);
        for &code_size in &code_sizes_to_pack[..total_code_sizes_to_pack] {
            if code_size == 0 {
                rle.prev_code_size(&mut packed_code_sizes_cursor, self)?;
                rle.z_count += 1;
                if rle.z_count == 138 {
                    rle.zero_code_size(&mut packed_code_sizes_cursor, self)?;
                }
            } else {
                rle.zero_code_size(&mut packed_code_sizes_cursor, self)?;
                if code_size != rle.prev_code_size {
                    rle.prev_code_size(&mut packed_code_sizes_cursor, self)?;
                    self.count[HUFF_CODES_TABLE][code_size as usize] =
                        self.count[HUFF_CODES_TABLE][code_size as usize].wrapping_add(1);
                    packed_code_sizes_cursor.write_all(&[code_size][..])?;
                } else {
                    rle.repeat_count += 1;
                    if rle.repeat_count == 6 {
                        rle.prev_code_size(&mut packed_code_sizes_cursor, self)?;
                    }
                }
            }
            rle.prev_code_size = code_size;
        }

        if rle.repeat_count != 0 {
            rle.prev_code_size(&mut packed_code_sizes_cursor, self)?;
        } else {
            rle.zero_code_size(&mut packed_code_sizes_cursor, self)?;
        }

        self.optimize_table(2, MAX_HUFF_SYMBOLS_2, 7, false);

        output.put_bits(2, 2);

        output.put_bits((num_lit_codes - 257) as u32, 5);
        output.put_bits((num_dist_codes - 1) as u32, 5);

        let mut num_bit_lengths = 18 -
            HUFFMAN_LENGTH_ORDER
                .iter()
                .rev()
                .take_while(|&swizzle| {
                    self.code_sizes[HUFF_CODES_TABLE][*swizzle as usize] == 0
                })
                .count();

        num_bit_lengths = cmp::max(4, num_bit_lengths + 1);
        output.put_bits(num_bit_lengths as u32 - 4, 4);
        for &swizzle in &HUFFMAN_LENGTH_ORDER[..num_bit_lengths] {
            output.put_bits(
                self.code_sizes[HUFF_CODES_TABLE][swizzle as usize] as u32,
                3,
            );
        }

        let mut packed_code_size_index = 0 as usize;
        let packed_code_sizes = packed_code_sizes_cursor.get_ref();
        while packed_code_size_index < packed_code_sizes_cursor.position() as usize {
            let code = packed_code_sizes[packed_code_size_index] as usize;
            packed_code_size_index += 1;
            assert!(code < MAX_HUFF_SYMBOLS_2);
            output.put_bits(
                self.codes[HUFF_CODES_TABLE][code] as u32,
                self.code_sizes[HUFF_CODES_TABLE][code] as u32,
            );
            if code >= 16 {
                output.put_bits(
                    packed_code_sizes[packed_code_size_index] as u32,
                    [2, 3, 7][code - 16],
                );
                packed_code_size_index += 1;
            }
        }

        Ok(())
    }
}

struct DictOxide {
    /// The maximum number of checks in the hash chain, for the initial,
    /// and the lazy match respectively.
    pub max_probes: [u32; 2],
    /// Buffer of input data.
    /// Padded with 1 byte to simplify matching code in `compress_fast`.
    pub b: Box<HashBuffers>,

    pub code_buf_dict_pos: u32,
    pub lookahead_size: u32,
    pub lookahead_pos: u32,
    pub size: u32,
}

impl DictOxide {
    fn new(flags: u32) -> Self {
        DictOxide {
            max_probes: [1 + ((flags & 0xFFF) + 2) / 3, 1 + (((flags & 0xFFF) >> 2) + 2) / 3],
            b: Box::default(),
            code_buf_dict_pos: 0,
            lookahead_size: 0,
            lookahead_pos: 0,
            size: 0,
        }
    }

    /// Do an unaligned read of the data at `pos` in the dictionary and treat it as if it was of
    /// type T.
    ///
    /// Unsafe due to reading without bounds checking and type casting.
    unsafe fn read_unaligned<T>(&self, pos: isize) -> T {
        ptr::read_unaligned((&self.b.dict as *const [u8] as *const u8).offset(pos) as
                            *const T)
    }

    /// Try to find a match for the data at lookahead_pos in the dictionary that is
    /// longer than `match_len`.
    /// Returns a tuple containing (match_distance, match_length). Will be equal to the input
    /// values if no better matches were found.
    fn find_match(
        &self,
        lookahead_pos: u32,
        max_dist: u32,
        max_match_len: u32,
        mut match_dist: u32,
        mut match_len: u32,
    ) -> (u32, u32) {
        // Clamp the match len and max_match_len to be valid. (It should be when this is called, but
        // do it for now just in case for safety reasons.)
        // This should normally end up as at worst conditional moves,
        // so it shouldn't slow us down much.
        // TODO: Statically verify these so we don't need to do this.
        let max_match_len = cmp::min(MAX_MATCH_LEN as u32, max_match_len);
        match_len = cmp::max(match_len, 1);

        let pos = lookahead_pos & LZ_DICT_SIZE_MASK;
        let mut probe_pos = pos;
        // Number of probes into the hash chains.
        let mut num_probes_left = self.max_probes[(match_len >= 32) as usize];

        // If we already have a match of the full length don't bother searching for another one.
        if max_match_len <= match_len {
            return (match_dist, match_len);
        }

        // Read the last byte of the current match, and the next one, used to compare matches.
        // # Unsafe
        // `pos` is masked by `LZ_DICT_SIZE_MASK`
        // `match_len` is clamped be at least 1.
        // If it is larger or equal to the maximum length, this statement won't be reached.
        // As the size of self.dict is LZ_DICT_SIZE + MAX_MATCH_LEN - 1 + DICT_PADDING,
        // this will not go out of bounds.
        let mut c01: u16 = unsafe {self.read_unaligned((pos + match_len - 1) as isize)};
        // Read the two bytes at the end position of the current match.
        // # Unsafe
        // See previous.
        let s01: u16 = unsafe {self.read_unaligned(pos as isize)};

        'outer: loop {
            let mut dist;
            'found: loop {
                num_probes_left -= 1;
                if num_probes_left == 0 {
                    // We have done as many probes in the hash chain as the current compression
                    // settings allow, so return the best match we found, if any.
                    return (match_dist, match_len);
                }

                for _ in 0..3 {
                    let next_probe_pos = self.b.next[probe_pos as usize] as u32;

                    dist = ((lookahead_pos - next_probe_pos) & 0xFFFF) as u32;
                    if next_probe_pos == 0 || dist > max_dist {
                        // We reached the end of the hash chain, or the next value is further away
                        // than the maximum allowed distance, so return the best match we found, if
                        // any.
                        return (match_dist, match_len);
                    }

                    // Mask the position value to get the position in the hash chain of the next
                    // position to match against.
                    probe_pos = next_probe_pos & LZ_DICT_SIZE_MASK;
                    // # Unsafe
                    // See the beginning of this function.
                    // probe_pos and match_length are still both bounded.
                    unsafe {
                        // The first two bytes, last byte and the next byte matched, so
                        // check the match further.
                        if self.read_unaligned::<u16>((probe_pos + match_len - 1) as isize) == c01 {
                            break 'found;
                        }
                    }
                }
            }

            if dist == 0 {
                return (match_dist, match_len);
            }
            // # Unsafe
            // See the beginning of this function.
            // probe_pos is bounded by masking with LZ_DICT_SIZE_MASK.
            unsafe {
                if self.read_unaligned::<u16>(probe_pos as isize) != s01 {
                    continue;
                }
            }

            let mut p = pos + 2;
            let mut q = probe_pos + 2;
            // Check the length of the match.
            for _ in 0..32 {
                // # Unsafe
                // This loop has a fixed counter, so p_data and q_data will never be
                // increased beyond 250 bytes past the initial values.
                // Both pos and probe_pos are bounded by masking with LZ_DICT_SIZE_MASK,
                // so {pos|probe_pos} + 258 will never exceed dict.len().
                let p_data: u64 = unsafe {u64::from_le(self.read_unaligned(p as isize))};
                let q_data: u64 = unsafe {u64::from_le(self.read_unaligned(q as isize))};
                // Compare of 8 bytes at a time by using unaligned loads of 64-bit integers.
                let xor_data = p_data ^ q_data;
                if xor_data == 0 {
                    p += 8;
                    q += 8;
                } else {
                    // If not all of the last 8 bytes matched, check how may of them did.
                    let trailing = xor_data.trailing_zeros();

                    let probe_len = p - pos + (trailing >> 3);
                    if probe_len > match_len {
                        match_dist = dist;
                        match_len = cmp::min(max_match_len, probe_len);
                        if match_len == max_match_len {
                            return (match_dist, match_len);
                        }
                        // # Unsafe
                        // pos is bounded by masking.
                        unsafe {
                            c01 = self.read_unaligned((pos + match_len - 1) as isize);
                        }
                    }
                    continue 'outer;
                }
            }

            return (dist, cmp::min(max_match_len, MAX_MATCH_LEN as u32));
        }
    }
}

struct ParamsOxide {
    pub flags: u32,
    pub greedy_parsing: bool,
    pub block_index: u32,

    pub saved_match_dist: u32,
    pub saved_match_len: u32,
    pub saved_lit: u8,

    pub flush: TDEFLFlush,
    pub flush_ofs: u32,
    pub flush_remaining: u32,
    pub finished: bool,

    pub adler32: u32,

    pub src_pos: usize,

    pub out_buf_ofs: usize,
    pub prev_return_status: TDEFLStatus,

    pub saved_bit_buffer: u32,
    pub saved_bits_in: u32,

    pub local_buf: Box<LocalBuf>,
}

impl ParamsOxide {
    fn new(flags: u32) -> Self {
        ParamsOxide {
            flags: flags,
            greedy_parsing: flags & TDEFL_GREEDY_PARSING_FLAG != 0,
            block_index: 0,
            saved_match_dist: 0,
            saved_match_len: 0,
            saved_lit: 0,
            flush: TDEFLFlush::None,
            flush_ofs: 0,
            flush_remaining: 0,
            finished: false,
            adler32: MZ_ADLER32_INIT as u32,
            src_pos: 0,
            out_buf_ofs: 0,
            prev_return_status: TDEFLStatus::Okay,
            saved_bit_buffer: 0,
            saved_bits_in: 0,
            local_buf: Box::default()
        }
    }
}

struct LZOxide {
    pub codes: [u8; LZ_CODE_BUF_SIZE],
    pub code_position: usize,
    pub flag_position: usize,

    pub total_bytes: u32,
    pub num_flags_left: u32,
}

impl LZOxide {
    fn new() -> Self {
        LZOxide {
            codes: [0; LZ_CODE_BUF_SIZE],
            code_position: 1,
            flag_position: 0,
            total_bytes: 0,
            num_flags_left: 8,
        }
    }

    fn write_code(&mut self, val: u8) {
        self.codes[self.code_position] = val;
        self.code_position += 1;
    }

    fn init_flag(&mut self) {
        if self.num_flags_left == 8 {
            *self.get_flag() = 0;
            self.code_position -= 1;
        } else {
            *self.get_flag() >>= self.num_flags_left;
        }
    }

    fn get_flag(&mut self) -> &mut u8 {
        &mut self.codes[self.flag_position]
    }

    fn plant_flag(&mut self) {
        self.flag_position = self.code_position;
        self.code_position += 1;
    }

    fn consume_flag(&mut self) {
        self.num_flags_left -= 1;
        if self.num_flags_left == 0 {
            self.num_flags_left = 8;
            self.plant_flag();
        }
    }
}

fn compress_lz_codes(
    huff: &HuffmanOxide,
    output: &mut OutputBufferOxide,
    lz_code_buf: &[u8],
) -> io::Result<bool> {
    let mut flags = 1;
    let mut bb = BitBuffer {
        bit_buffer: output.bit_buffer as u64,
        bits_in: output.bits_in,
    };

    let mut i: usize = 0;
    while i < lz_code_buf.len() {
        if flags == 1 {
            flags = lz_code_buf[i] as u32 | 0x100;
            i += 1;
        }

        // The lz code was a length code
        if flags & 1 == 1 {
            flags >>= 1;

            let sym;
            let num_extra_bits;

            let match_len = lz_code_buf[i] as usize;

            let match_dist =
                read_u16_le(lz_code_buf, i + 1);

            i += 3;

            debug_assert!(huff.code_sizes[0][LEN_SYM[match_len] as usize] != 0);
            bb.put_fast(
                huff.codes[0][LEN_SYM[match_len] as usize] as u64,
                huff.code_sizes[0][LEN_SYM[match_len] as usize] as u32,
            );
            bb.put_fast(
                match_len as u64 & BITMASKS[LEN_EXTRA[match_len] as usize] as u64,
                LEN_EXTRA[match_len] as u32,
            );

            if match_dist < 512 {
                sym = SMALL_DIST_SYM[match_dist as usize] as usize;
                num_extra_bits = SMALL_DIST_EXTRA[match_dist as usize] as usize;
            } else {
                sym = LARGE_DIST_SYM[(match_dist >> 8) as usize] as usize;
                num_extra_bits = LARGE_DIST_EXTRA[(match_dist >> 8) as usize] as usize;
            }

            debug_assert!(huff.code_sizes[1][sym] != 0);
            bb.put_fast(huff.codes[1][sym] as u64, huff.code_sizes[1][sym] as u32);
            bb.put_fast(
                match_dist as u64 & BITMASKS[num_extra_bits as usize] as u64,
                num_extra_bits as u32,
            );
        } else {
            // The lz code was a literal
            for _ in 0..3 {
                flags >>= 1;
                let lit = lz_code_buf[i];
                i += 1;

                debug_assert!(huff.code_sizes[0][lit as usize] != 0);
                bb.put_fast(
                    huff.codes[0][lit as usize] as u64,
                    huff.code_sizes[0][lit as usize] as u32,
                );

                if flags & 1 == 1 || i >= lz_code_buf.len() {
                    break;
                }
            }
        }

        bb.flush(output)?;
    }

    output.bits_in = 0;
    output.bit_buffer = 0;
    while bb.bits_in != 0 {
        let n = cmp::min(bb.bits_in, 16);
        output.put_bits(bb.bit_buffer as u32 & BITMASKS[n as usize], n);
        bb.bit_buffer >>= n;
        bb.bits_in -= n;
    }

    // Output the end of block symbol.
    output.put_bits(huff.codes[0][256] as u32, huff.code_sizes[0][256] as u32);

    Ok(true)
}

fn compress_block(
    huff: &mut HuffmanOxide,
    output: &mut OutputBufferOxide,
    lz: &LZOxide,
    static_block: bool,
) -> io::Result<bool> {
    if static_block {
        huff.start_static_block(output);
    } else {
        huff.start_dynamic_block(output)?;
    }

    compress_lz_codes(huff, output, &lz.codes[..lz.code_position])
}

fn flush_block(
    d: &mut CompressorOxide,
    callback: &mut CallbackOxide,
    flush: TDEFLFlush,
) -> io::Result<i32> {
    let mut saved_buffer;
    {
        let mut output = callback.out.new_output_buffer(
            &mut d.params.local_buf.b,
            d.params.out_buf_ofs,
        );
        output.bit_buffer = d.params.saved_bit_buffer;
        output.bits_in = d.params.saved_bits_in;

        let use_raw_block = (d.params.flags & TDEFL_FORCE_ALL_RAW_BLOCKS != 0) &&
            (d.dict.lookahead_pos - d.dict.code_buf_dict_pos) <= d.dict.size;

        assert!(d.params.flush_remaining == 0);
        d.params.flush_ofs = 0;
        d.params.flush_remaining = 0;

        d.lz.init_flag();

        // If we are at the start of the stream, write the zlib header if requested.
        if d.params.flags & TDEFL_WRITE_ZLIB_HEADER != 0 && d.params.block_index == 0 {
            output.put_bits(0x78, 8);
            output.put_bits(0x01, 8);
        }

        // Output the block header.
        output.put_bits((flush == TDEFLFlush::Finish) as u32, 1);

        saved_buffer = output.save();

        let mut comp_success = false;
        if !use_raw_block {
            let use_static = (d.params.flags & TDEFL_FORCE_ALL_STATIC_BLOCKS != 0) ||
                (d.lz.total_bytes < 48);
            comp_success = compress_block(&mut d.huff, &mut output, &d.lz, use_static)?;
        }

        // If we failed to compress anything and the output would take up more space than the output
        // data, output a stored block instead, which has at most 5 bytes of overhead.
        // We only use some simple heuristics for now.
        // A stored block will have an overhead of at least 4 bytes containing the block length
        // but usually more due to the length parameters having to start at a byte boundary and thus
        // requiring up to 5 bytes of padding.
        // As a static block will have an overhead of at most 1 bit per byte
        // (as literals are either 8 or 9 bytes), a raw block will
        // never take up less space if the number of input bytes are less than 32.
        let expanded = (d.lz.total_bytes > 32) &&
            (output.inner.position() - saved_buffer.pos + 1 >= d.lz.total_bytes as u64) &&
            (d.dict.lookahead_pos - d.dict.code_buf_dict_pos <= d.dict.size);

        if use_raw_block || expanded {
            output.load(saved_buffer);

            // Block header.
            output.put_bits(0, 2);

            // Block length has to start on a byte boundary, so pad.
            output.pad_to_bytes();

            // Block length and ones complement of block length.
            output.put_bits(d.lz.total_bytes & 0xFFFF, 16);
            output.put_bits(!d.lz.total_bytes & 0xFFFF, 16);

            // Write the actual bytes.
            for i in 0..d.lz.total_bytes {
                let pos = (d.dict.code_buf_dict_pos + i) & LZ_DICT_SIZE_MASK;
                output.put_bits(d.dict.b.dict[pos as usize] as u32, 8);
            }
        } else if !comp_success {
            output.load(saved_buffer);
            compress_block(&mut d.huff, &mut output, &d.lz, true)?;
        }

        if flush != TDEFLFlush::None {
            if flush == TDEFLFlush::Finish {
                output.pad_to_bytes();
                if d.params.flags & TDEFL_WRITE_ZLIB_HEADER != 0 {
                    let mut adler = d.params.adler32;
                    for _ in 0..4 {
                        output.put_bits((adler >> 24) & 0xFF, 8);
                        adler <<= 8;
                    }
                }
            } else {
                output.put_bits(0, 3);
                output.pad_to_bytes();
                output.put_bits(0, 16);
                output.put_bits(0xFFFF, 16);
            }
        }

        memset(&mut d.huff.count[0][..MAX_HUFF_SYMBOLS_0], 0);
        memset(&mut d.huff.count[1][..MAX_HUFF_SYMBOLS_1], 0);

        d.lz.code_position = 1;
        d.lz.flag_position = 0;
        d.lz.num_flags_left = 8;
        d.dict.code_buf_dict_pos += d.lz.total_bytes;
        d.lz.total_bytes = 0;
        d.params.block_index += 1;

        saved_buffer = output.save();

        d.params.saved_bit_buffer = saved_buffer.bit_buffer;
        d.params.saved_bits_in = saved_buffer.bits_in;
    }

    Ok(callback.flush_output(saved_buffer, &mut d.params))
}

fn record_literal(h: &mut HuffmanOxide, lz: &mut LZOxide, lit: u8) {
    lz.total_bytes += 1;
    lz.write_code(lit);

    *lz.get_flag() >>= 1;
    lz.consume_flag();

    h.count[0][lit as usize] += 1;
}

fn record_match(
    h: &mut HuffmanOxide,
    lz: &mut LZOxide,
    mut match_len: u32,
    mut match_dist: u32,
) {
    assert!(match_len >= MIN_MATCH_LEN);
    assert!(match_dist >= 1);
    assert!(match_dist as usize <= LZ_DICT_SIZE);

    lz.total_bytes += match_len;
    match_dist -= 1;
    match_len -= MIN_MATCH_LEN as u32;
    lz.write_code(match_len as u8);
    lz.write_code(match_dist as u8);
    lz.write_code((match_dist >> 8) as u8);

    *lz.get_flag() >>= 1;
    *lz.get_flag() |= 0x80;
    lz.consume_flag();

    let symbol = if match_dist < 512 {
        SMALL_DIST_SYM[match_dist as usize]
    } else {
        LARGE_DIST_SYM[((match_dist >> 8) & 127) as usize]
    } as usize;
    h.count[1][symbol] += 1;
    h.count[0][LEN_SYM[match_len as usize] as usize] += 1;
}

fn compress_normal(d: &mut CompressorOxide, callback: &mut CallbackOxide) -> bool {
    let mut src_pos = d.params.src_pos;
    let in_buf = match callback.in_buf {
        None => return true,
        Some(in_buf) => in_buf,
    };

    let mut lookahead_size = d.dict.lookahead_size;
    let mut lookahead_pos = d.dict.lookahead_pos;
    let mut saved_lit = d.params.saved_lit;
    let mut saved_match_dist = d.params.saved_match_dist;
    let mut saved_match_len = d.params.saved_match_len;

    while src_pos < in_buf.len() || (d.params.flush != TDEFLFlush::None && lookahead_size != 0) {
        let src_buf_left = in_buf.len() - src_pos;
        let num_bytes_to_process =
            cmp::min(src_buf_left, MAX_MATCH_LEN - lookahead_size as usize);

        if lookahead_size + d.dict.size >= MIN_MATCH_LEN - 1 && num_bytes_to_process > 0 {
            let dictb = &mut d.dict.b;

            let mut dst_pos = (lookahead_pos + lookahead_size) & LZ_DICT_SIZE_MASK;
            let mut ins_pos = lookahead_pos + lookahead_size - 2;
            let mut hash = ((dictb.dict[(ins_pos & LZ_DICT_SIZE_MASK) as usize] as u32) <<
                                LZ_HASH_SHIFT) ^
                (dictb.dict[((ins_pos + 1) & LZ_DICT_SIZE_MASK) as usize] as u32);

            lookahead_size += num_bytes_to_process as u32;
            for &c in &in_buf[src_pos..src_pos + num_bytes_to_process] {
                dictb.dict[dst_pos as usize] = c;
                if (dst_pos as usize) < MAX_MATCH_LEN - 1 {
                    dictb.dict[LZ_DICT_SIZE + dst_pos as usize] = c;
                }

                hash = ((hash << LZ_HASH_SHIFT) ^ (c as u32)) &
                    (LZ_HASH_SIZE as u32 - 1);
                dictb.next[(ins_pos & LZ_DICT_SIZE_MASK) as usize] = dictb.hash[hash as
                                                                                  usize];

                dictb.hash[hash as usize] = ins_pos as u16;
                dst_pos = (dst_pos + 1) & LZ_DICT_SIZE_MASK;
                ins_pos += 1;
            }
            src_pos += num_bytes_to_process;
        } else {
            let dictb = &mut d.dict.b;
            for &c in &in_buf[src_pos..src_pos + num_bytes_to_process] {
                let dst_pos = (lookahead_pos + lookahead_size) & LZ_DICT_SIZE_MASK;
                dictb.dict[dst_pos as usize] = c;
                if (dst_pos as usize) < MAX_MATCH_LEN - 1 {
                    dictb.dict[LZ_DICT_SIZE + dst_pos as usize] = c;
                }

                lookahead_size += 1;
                if lookahead_size + d.dict.size >= MIN_MATCH_LEN {
                    let ins_pos = lookahead_pos + lookahead_size - 3;
                    let hash =
                        (((dictb.dict[(ins_pos & LZ_DICT_SIZE_MASK) as usize] as u32) <<
                              (LZ_HASH_SHIFT * 2)) ^
                             (((dictb.dict[((ins_pos + 1) & LZ_DICT_SIZE_MASK) as
                                                usize] as u32) <<
                                   LZ_HASH_SHIFT) ^
                                  (c as u32))) &
                            (LZ_HASH_SIZE as u32 - 1);

                    dictb.next[(ins_pos & LZ_DICT_SIZE_MASK) as usize] = dictb.hash
                        [hash as usize];
                    dictb.hash[hash as usize] = ins_pos as u16;
                }
            }

            src_pos += num_bytes_to_process;
        }

        d.dict.size = cmp::min(LZ_DICT_SIZE as u32 - lookahead_size, d.dict.size);
        if d.params.flush == TDEFLFlush::None && (lookahead_size as usize) < MAX_MATCH_LEN {
            break;
        }

        let mut len_to_move = 1;
        let mut cur_match_dist = 0;
        let mut cur_match_len = if saved_match_len != 0 {
            saved_match_len
        } else {
            MIN_MATCH_LEN - 1
        };
        let cur_pos = lookahead_pos & LZ_DICT_SIZE_MASK;
        if d.params.flags & (TDEFL_RLE_MATCHES | TDEFL_FORCE_ALL_RAW_BLOCKS) != 0 {
            if d.dict.size != 0 && d.params.flags & TDEFL_FORCE_ALL_RAW_BLOCKS == 0 {
                let c = d.dict.b.dict[((cur_pos.wrapping_sub(1)) & LZ_DICT_SIZE_MASK) as usize];
                cur_match_len = d.dict.b.dict[cur_pos as usize..
                                                (cur_pos + lookahead_size) as usize]
                    .iter()
                    .take_while(|&x| *x == c)
                    .count() as u32;
                if cur_match_len < MIN_MATCH_LEN {
                    cur_match_len = 0
                } else {
                    cur_match_dist = 1
                }
            }
        } else {
            let dist_len = d.dict.find_match(
                lookahead_pos,
                d.dict.size,
                lookahead_size,
                cur_match_dist,
                cur_match_len,
            );
            cur_match_dist = dist_len.0;
            cur_match_len = dist_len.1;
        }

        let far_and_small = cur_match_len == MIN_MATCH_LEN && cur_match_dist >= 8 * 1024;
        let filter_small = d.params.flags & TDEFL_FILTER_MATCHES != 0 && cur_match_len <= 5;
        if far_and_small || filter_small || cur_pos == cur_match_dist {
            cur_match_dist = 0;
            cur_match_len = 0;
        }

        if saved_match_len != 0 {
            if cur_match_len > saved_match_len {
                record_literal(&mut d.huff, &mut d.lz, saved_lit);
                if cur_match_len >= 128 {
                    record_match(&mut d.huff, &mut d.lz, cur_match_len, cur_match_dist);
                    saved_match_len = 0;
                    len_to_move = cur_match_len;
                } else {
                    saved_lit = d.dict.b.dict[cur_pos as usize];
                    saved_match_dist = cur_match_dist;
                    saved_match_len = cur_match_len;
                }
            } else {
                record_match(&mut d.huff, &mut d.lz, saved_match_len, saved_match_dist);
                len_to_move = saved_match_len - 1;
                saved_match_len = 0;
            }
        } else if cur_match_dist == 0 {
            record_literal(
                &mut d.huff,
                &mut d.lz,
                d.dict.b.dict[cmp::min(cur_pos as usize, d.dict.b.dict.len() - 1)],
            );
        } else if d.params.greedy_parsing || (d.params.flags & TDEFL_RLE_MATCHES != 0) ||
                   cur_match_len >= 128
        {
            // If we are using lazy matching, check for matches at the next byte if the current
            // match was shorter than 128 bytes.
            record_match(&mut d.huff, &mut d.lz, cur_match_len, cur_match_dist);
            len_to_move = cur_match_len;
        } else {
            saved_lit = d.dict.b.dict[cmp::min(cur_pos as usize, d.dict.b.dict.len() - 1)];
            saved_match_dist = cur_match_dist;
            saved_match_len = cur_match_len;
        }

        lookahead_pos += len_to_move;
        assert!(lookahead_size >= len_to_move);
        lookahead_size -= len_to_move;
        d.dict.size = cmp::min(d.dict.size + len_to_move, LZ_DICT_SIZE as u32);

        let lz_buf_tight = d.lz.code_position > LZ_CODE_BUF_SIZE - 8;
        let raw = d.params.flags & TDEFL_FORCE_ALL_RAW_BLOCKS != 0;
        let fat = ((d.lz.code_position * 115) >> 7) >= d.lz.total_bytes as usize;
        let fat_or_raw = (d.lz.total_bytes > 31 * 1024) && (fat || raw);

        if lz_buf_tight || fat_or_raw {
            d.params.src_pos = src_pos;
            // These values are used in flush_block, so we need to write them back here.
            d.dict.lookahead_size = lookahead_size;
            d.dict.lookahead_pos = lookahead_pos;

            let n = flush_block(d, callback, TDEFLFlush::None).unwrap_or(
                TDEFLStatus::PutBufFailed as
                    i32,
            );
            if n != 0 {
                d.params.saved_lit = saved_lit;
                d.params.saved_match_dist = saved_match_dist;
                d.params.saved_match_len = saved_match_len;
                return n > 0;
            }
        }
    }

    d.params.src_pos = src_pos;
    d.dict.lookahead_size = lookahead_size;
    d.dict.lookahead_pos = lookahead_pos;
    d.params.saved_lit = saved_lit;
    d.params.saved_match_dist = saved_match_dist;
    d.params.saved_match_len = saved_match_len;
    true
}

const COMP_FAST_LOOKAHEAD_SIZE: u32 = 4096;

fn compress_fast(d: &mut CompressorOxide, callback: &mut CallbackOxide) -> bool {
    let mut src_pos = d.params.src_pos;
    let mut lookahead_size = d.dict.lookahead_size;
    let mut lookahead_pos = d.dict.lookahead_pos;

    let mut cur_pos = lookahead_pos & LZ_DICT_SIZE_MASK;
    let in_buf = match callback.in_buf {
        None => return true,
        Some(in_buf) => in_buf,
    };

    assert!(d.lz.code_position < LZ_CODE_BUF_SIZE - 2);

    while src_pos < in_buf.len() || (d.params.flush != TDEFLFlush::None && lookahead_size > 0) {
        let mut dst_pos = ((lookahead_pos + lookahead_size) & LZ_DICT_SIZE_MASK) as usize;
        let mut num_bytes_to_process = cmp::min(
            in_buf.len() - src_pos,
            (COMP_FAST_LOOKAHEAD_SIZE - lookahead_size) as usize,
        );
        lookahead_size += num_bytes_to_process as u32;

        while num_bytes_to_process != 0 {
            let n = cmp::min(LZ_DICT_SIZE - dst_pos, num_bytes_to_process);
            d.dict.b.dict[dst_pos..dst_pos + n].copy_from_slice(&in_buf[src_pos..src_pos + n]);

            if dst_pos < MAX_MATCH_LEN - 1 {
                let m = cmp::min(n, MAX_MATCH_LEN - 1 - dst_pos);
                d.dict.b.dict[dst_pos + LZ_DICT_SIZE..dst_pos + LZ_DICT_SIZE + m]
                    .copy_from_slice(&in_buf[src_pos..src_pos + m]);
            }

            src_pos += n;
            dst_pos = (dst_pos + n) & LZ_DICT_SIZE_MASK as usize;
            num_bytes_to_process -= n;
        }

        d.dict.size = cmp::min(LZ_DICT_SIZE as u32 - lookahead_size, d.dict.size);
        if d.params.flush == TDEFLFlush::None && lookahead_size < COMP_FAST_LOOKAHEAD_SIZE {
            break;
        }

        while lookahead_size >= 4 {
            let mut cur_match_len = 1;
            // # Unsafe
            // cur_pos is always masked when assigned to.
            let first_trigram = unsafe {
                u32::from_le(d.dict.read_unaligned::<u32>(cur_pos as isize)) & 0xFF_FFFF
            };

            let hash = (first_trigram ^ (first_trigram >> (24 - (LZ_HASH_BITS - 8)))) &
                LEVEL1_HASH_SIZE_MASK;

            let mut probe_pos = d.dict.b.hash[hash as usize] as u32;
            d.dict.b.hash[hash as usize] = lookahead_pos as u16;

            let mut cur_match_dist = (lookahead_pos - probe_pos) as u16;
            if cur_match_dist as u32 <= d.dict.size {
                probe_pos &= LZ_DICT_SIZE_MASK;
                // # Unsafe
                // probe_pos was just masked so it can't exceed the dictionary size + max_match_len.
                let trigram = unsafe {
                    u32::from_le(d.dict.read_unaligned::<u32>(probe_pos as isize)) & 0xFF_FFFF
                };

                if first_trigram == trigram {
                    // Trigram was tested, so we can start with "+ 3" displacement.
                    let mut p = (cur_pos + 3) as isize;
                    let mut q = (probe_pos + 3) as isize;
                    cur_match_len = 'find_match: loop {
                        for _ in 0..32 {
                            // # Unsafe
                            // This loop has a fixed counter, so p_data and q_data will never be
                            // increased beyond 251 bytes past the initial values.
                            // Both pos and probe_pos are bounded by masking with
                            // LZ_DICT_SIZE_MASK,
                            // so {pos|probe_pos} + 258 will never exceed dict.len().
                            // (As long as dict is padded by one additional byte to have
                            // 259 bytes of lookahead since we start at cur_pos + 3.)
                            let p_data: u64 =
                                unsafe {u64::from_le(d.dict.read_unaligned(p as isize))};
                            let q_data: u64 =
                                unsafe {u64::from_le(d.dict.read_unaligned(q as isize))};
                            let xor_data = p_data ^ q_data;
                            if xor_data == 0 {
                                p += 8;
                                q += 8;
                            } else {
                                let trailing = xor_data.trailing_zeros();
                                break 'find_match p as u32 - cur_pos + (trailing >> 3);
                            }
                        }

                        break 'find_match if cur_match_dist == 0 {
                            0
                        } else {
                            MAX_MATCH_LEN as u32
                        };
                    };

                    if cur_match_len < MIN_MATCH_LEN ||
                        (cur_match_len == MIN_MATCH_LEN && cur_match_dist >= 8 * 1024)
                    {
                        let lit = first_trigram as u8;
                        cur_match_len = 1;
                        d.lz.write_code(lit);
                        *d.lz.get_flag() >>= 1;
                        d.huff.count[0][lit as usize] += 1;
                    } else {
                        // Limit the match to the length of the lookahead so we don't create a match
                        // that ends after the end of the input data.
                        cur_match_len = cmp::min(cur_match_len, lookahead_size);
                        debug_assert!(cur_match_len >= MIN_MATCH_LEN);
                        debug_assert!(cur_match_dist >= 1);
                        debug_assert!(cur_match_dist as usize <= LZ_DICT_SIZE);
                        cur_match_dist -= 1;

                        d.lz.write_code((cur_match_len - MIN_MATCH_LEN) as u8);
                        // # Unsafe
                        // code_position is checked to be smaller than the lz buffer size
                        // at the start of this function and on every loop iteration.
                        unsafe {
                            write_u16_le_uc(cur_match_dist as u16,
                                            &mut d.lz.codes,
                                            d.lz.code_position);
                            d.lz.code_position += 2;
                        }

                        *d.lz.get_flag() >>= 1;
                        *d.lz.get_flag() |= 0x80;
                        if cur_match_dist < 512 {
                            d.huff.count[1][SMALL_DIST_SYM[cur_match_dist as usize] as
                                                usize] += 1;
                        } else {
                            d.huff.count[1][LARGE_DIST_SYM[(cur_match_dist >> 8) as usize] as
                                                usize] += 1;
                        }

                        d.huff.count[0][LEN_SYM[(cur_match_len - MIN_MATCH_LEN) as
                                                          usize] as
                                            usize] += 1;
                    }
                } else {
                    d.lz.write_code(first_trigram as u8);
                    *d.lz.get_flag() >>= 1;
                    d.huff.count[0][first_trigram as u8 as usize] += 1;
                }

                d.lz.consume_flag();
                d.lz.total_bytes += cur_match_len;
                lookahead_pos += cur_match_len;
                d.dict.size = cmp::min(d.dict.size + cur_match_len, LZ_DICT_SIZE as u32);
                cur_pos = (cur_pos + cur_match_len) & LZ_DICT_SIZE_MASK;
                assert!(lookahead_size >= cur_match_len);
                lookahead_size -= cur_match_len;

                if d.lz.code_position > LZ_CODE_BUF_SIZE - 8 {
                    // These values are used in flush_block, so we need to write them back here.
                    d.dict.lookahead_size = lookahead_size;
                    d.dict.lookahead_pos = lookahead_pos;

                    let n = match flush_block(d, callback, TDEFLFlush::None) {
                        Err(_) => {
                            d.params.src_pos = src_pos;
                            d.params.prev_return_status = TDEFLStatus::PutBufFailed;
                            return false;
                        }
                        Ok(status) => status,
                    };
                    if n != 0 {
                        d.params.src_pos = src_pos;
                        return n > 0;
                    }
                    assert!(d.lz.code_position < LZ_CODE_BUF_SIZE - 2);

                    lookahead_size = d.dict.lookahead_size;
                    lookahead_pos = d.dict.lookahead_pos;
                }
            }
        }

        while lookahead_size != 0 {
            let lit = d.dict.b.dict[cur_pos as usize];
            d.lz.total_bytes += 1;
            d.lz.write_code(lit);
            *d.lz.get_flag() >>= 1;
            d.lz.consume_flag();

            d.huff.count[0][lit as usize] += 1;
            lookahead_pos += 1;
            d.dict.size = cmp::min(d.dict.size + 1, LZ_DICT_SIZE as u32);
            cur_pos = (cur_pos + 1) & LZ_DICT_SIZE_MASK;
            lookahead_size -= 1;

            if d.lz.code_position > LZ_CODE_BUF_SIZE - 8 {
                // These values are used in flush_block, so we need to write them back here.
                d.dict.lookahead_size = lookahead_size;
                d.dict.lookahead_pos = lookahead_pos;

                let n = match flush_block(d, callback, TDEFLFlush::None) {
                    Err(_) => {
                        d.params.prev_return_status = TDEFLStatus::PutBufFailed;
                        d.params.src_pos = src_pos;
                        return false;
                    }
                    Ok(status) => status,
                };
                if n != 0 {
                    d.params.src_pos = src_pos;
                    return n > 0;
                }

                lookahead_size = d.dict.lookahead_size;
                lookahead_pos = d.dict.lookahead_pos;
            }
        }
    }

    d.params.src_pos = src_pos;
    d.dict.lookahead_size = lookahead_size;
    d.dict.lookahead_pos = lookahead_pos;
    true
}

fn flush_output_buffer(
    c: &mut CallbackOxide,
    p: &mut ParamsOxide,
) -> (TDEFLStatus, usize, usize) {
    let mut res = (TDEFLStatus::Okay, p.src_pos, 0);
    if let CallbackOut::Buf(ref mut cb) = c.out {
        let n = cmp::min(cb.out_buf.len() - p.out_buf_ofs, p.flush_remaining as usize);
        if n != 0 {
            (&mut cb.out_buf[p.out_buf_ofs..p.out_buf_ofs + n])
                .copy_from_slice(&p.local_buf.b[p.flush_ofs as usize..p.flush_ofs as usize + n]);
        }
        p.flush_ofs += n as u32;
        p.flush_remaining -= n as u32;
        p.out_buf_ofs += n;
        res.2 = p.out_buf_ofs;
    }

    if p.finished && p.flush_remaining == 0 {
        res.0 = TDEFLStatus::Done
    }
    res
}

/// Main compression function. Puts output into buffer.
///
/// # Returns
/// Returns a tuple containing the current status of the compressor, the current position
/// in the input buffer and the current position in the output buffer.
pub fn compress(
    d: &mut CompressorOxide,
    in_buf: &[u8],
    out_buf: &mut [u8],
    flush: TDEFLFlush,
) -> (TDEFLStatus, usize, usize) {
    compress_inner(d, &mut CallbackOxide::new_callback_buf(in_buf, out_buf), flush)
}

/// Main compression function. Callbacks output.
///
/// # Returns
/// Returns a tuple containing the current status of the compressor, the current position
/// in the input buffer.
pub fn compress_to_output(
    d: &mut CompressorOxide,
    in_buf: &[u8],
    callback_func: CallbackFunc,
    flush: TDEFLFlush,
) -> (TDEFLStatus, usize) {
    let res = compress_inner(
        d,
        &mut CallbackOxide::new_callback_func(in_buf, callback_func),
        flush
    );

    (res.0, res.1)
}

fn compress_inner(
    d: &mut CompressorOxide,
    callback: &mut CallbackOxide,
    flush: TDEFLFlush,
) -> (TDEFLStatus, usize, usize) {
    d.params.out_buf_ofs = 0;
    d.params.src_pos = 0;

    let prev_ok = d.params.prev_return_status == TDEFLStatus::Okay;
    let flush_finish_once = d.params.flush != TDEFLFlush::Finish || flush == TDEFLFlush::Finish;

    d.params.flush = flush;
    if !prev_ok || !flush_finish_once {
        d.params.prev_return_status = TDEFLStatus::BadParam;
        return (d.params.prev_return_status, 0, 0);
    }

    if d.params.flush_remaining != 0 || d.params.finished {
        let res = flush_output_buffer(callback, &mut d.params);
        d.params.prev_return_status = res.0;
        return res;
    }

    let one_probe = d.params.flags & MAX_PROBES_MASK as u32 == 1;
    let greedy = d.params.flags & TDEFL_GREEDY_PARSING_FLAG != 0;
    let filter_or_rle_or_raw = d.params.flags &
        (TDEFL_FILTER_MATCHES | TDEFL_FORCE_ALL_RAW_BLOCKS | TDEFL_RLE_MATCHES) !=
        0;

    let compress_success = if one_probe && greedy && !filter_or_rle_or_raw {
        compress_fast(d, callback)
    } else {
        compress_normal(d, callback)
    };

    if !compress_success {
        return (
            d.params.prev_return_status,
            d.params.src_pos,
            d.params.out_buf_ofs,
        );
    }

    if let Some(in_buf) = callback.in_buf {
        if d.params.flags & (TDEFL_WRITE_ZLIB_HEADER | TDEFL_COMPUTE_ADLER32) != 0 {
            d.params.adler32 = update_adler32(d.params.adler32, &in_buf[..d.params.src_pos]);
        }
    }

    let flush_none = d.params.flush == TDEFLFlush::None;
    let in_left = callback.in_buf.map_or(0, |buf| buf.len()) - d.params.src_pos;
    let remaining = in_left != 0 || d.params.flush_remaining != 0;
    if !flush_none && d.dict.lookahead_size == 0 && !remaining {
        let flush = d.params.flush;
        match flush_block(d, callback, flush) {
            Err(_) => {
                d.params.prev_return_status = TDEFLStatus::PutBufFailed;
                return (
                    d.params.prev_return_status,
                    d.params.src_pos,
                    d.params.out_buf_ofs,
                );
            }
            Ok(x) if x < 0 => {
                return (
                    d.params.prev_return_status,
                    d.params.src_pos,
                    d.params.out_buf_ofs,
                )
            }
            _ => {
                d.params.finished = d.params.flush == TDEFLFlush::Finish;
                if d.params.flush == TDEFLFlush::Full {
                    memset(&mut d.dict.b.hash[..], 0);
                    memset(&mut d.dict.b.next[..], 0);
                    d.dict.size = 0;
                }
            }
        }
    }

    let res = flush_output_buffer(callback, &mut d.params);
    d.params.prev_return_status = res.0;

    res
}

/// Create a set of compression flags using parameters used by zlib and other compressors.
pub fn create_comp_flags_from_zip_params(level: i32, window_bits: i32, strategy: i32) -> u32 {
    let num_probes = (if level >= 0 {
                          cmp::min(10, level)
                      } else {
                          CompressionLevel::DefaultLevel as i32
                      }) as usize;
    let greedy = if level <= 3 {
        TDEFL_GREEDY_PARSING_FLAG
    } else {
        0
    } as u32;
    let mut comp_flags = NUM_PROBES[num_probes] | greedy;

    if window_bits > 0 {
        comp_flags |= TDEFL_WRITE_ZLIB_HEADER as u32;
    }

    if level == 0 {
        comp_flags |= TDEFL_FORCE_ALL_RAW_BLOCKS;
    } else if strategy == CompressionStrategy::Filtered as i32 {
        comp_flags |= TDEFL_FILTER_MATCHES;
    } else if strategy == CompressionStrategy::HuffmanOnly as i32 {
        comp_flags &= !MAX_PROBES_MASK as u32;
    } else if strategy == CompressionStrategy::Fixed as i32 {
        comp_flags |= TDEFL_FORCE_ALL_STATIC_BLOCKS;
    } else if strategy == CompressionStrategy::RLE as i32 {
        comp_flags |= TDEFL_RLE_MATCHES;
    }

    comp_flags
}


#[cfg(test)]
mod test {
    use super::{write_u16_le, write_u16_le_uc, read_u16_le};

    #[test]
    fn u16_to_slice() {
        let mut slice = [0, 0];
        write_u16_le(2000, &mut slice, 0);
        assert_eq!(slice, [208, 7]);
        let mut slice = [0, 0];
        unsafe {write_u16_le_uc(2000, &mut slice, 0)};
        assert_eq!(slice, [208, 7]);
    }

    #[test]
    fn u16_from_slice() {
        let mut slice = [208, 7];
        assert_eq!(read_u16_le(&mut slice, 0), 2000);
    }
}
