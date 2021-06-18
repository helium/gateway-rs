use bytes::{Buf, BufMut, BytesMut};
use helium_proto::Eui;
use std::hash::Hasher;
use xorf::{Filter as XorFilter, Xor16};
use xxhash_c::XXH64;

pub struct EuiFilter(Xor16);
pub struct DevAddrFilter {
    base: u32,
    mask: u32,
}

impl EuiFilter {
    pub fn from_bin(data: &[u8]) -> Self {
        let mut buf = data;
        let seed = buf.get_u64_le();
        let block_length = buf.get_u64_le() as usize;
        let mut filters: Vec<u16> = Vec::with_capacity(block_length * 3);
        for _ in 0..block_length * 3 {
            filters.push(buf.get_u16_le());
        }
        Self(Xor16 {
            seed,
            block_length,
            fingerprints: filters.into_boxed_slice(),
        })
    }

    pub fn contains(&self, eui: &Eui) -> bool {
        let Eui { deveui, appeui } = eui;
        let mut buf = BytesMut::with_capacity(16);
        buf.put_u64_le(*deveui);
        buf.put_u64_le(*appeui);
        let mut hasher = XXH64::new(0);
        hasher.write(&buf);
        let hash = hasher.finish();
        self.0.contains(&hash)
    }
}

impl DevAddrFilter {
    pub fn from_bin(data: &[u8]) -> Self {
        const BITS_23: u64 = 8388607; // biggest unsigned number in 23 bits
        const BITS_25: u64 = 33554431; // biggest unsigned number in 25 bits

        let mut buf: [u8; 8] = [0; 8];
        buf[2..].copy_from_slice(data);
        let val: u64 = u64::from_be_bytes(buf);
        Self {
            mask: (val & BITS_23) as u32,
            base: ((val >> 23) & BITS_25) as u32,
        }
    }

    pub fn contains(&self, devaddr: &u32) -> bool {
        devaddr & self.mask == self.base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod devaddr {
        use super::*;
        #[test]
        fn from_bin() {
            static MASK: [u8; 6] = [0, 2, 0, 127, 255, 0];
            let filter = DevAddrFilter::from_bin(&MASK);
            assert_eq!(1024, filter.base);
            assert_eq!(8388352, filter.mask);
            assert!(filter.contains(&1024));
        }
    }

    mod eui {
        use super::*;
        #[test]
        // Try an empty serialized filter.
        fn empty_filter() {
            static EMPTY_BIN: [u8; 76] = [
                193, 92, 2, 137, 236, 45, 10, 145, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 236, 22, 0, 0, 0,
                0, 208, 1, 236, 22, 0, 0, 0, 0, 72, 188, 41, 4, 0, 112, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 1, 0, 0, 0, 168, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24, 236, 22, 0, 0, 0, 0, 1,
                104, 2, 0,
            ];
            let filter = EuiFilter::from_bin(&EMPTY_BIN);
            assert!(!filter.contains(&Eui {
                deveui: 0,
                appeui: 0,
            }),);
        }

        #[test]
        //  Test a filter with keys generated in an external (erlang xor16) package.
        fn some_filter() {
            static SOME_KEYS: [[u64; 2]; 10] = [
                [9741577031045377197, 5631624589620531025],
                [4053769789384140926, 261708585656931929],
                [15656485083446225282, 12944688400506628191],
                [2532554414978603187, 5068956979456058210],
                [11707572432716655343, 10251566706728408737],
                [12724588641898500322, 14687969799823696951],
                [1227240127989838526, 4588270702326584272],
                [12607244973879047991, 18360762251427518680],
                [5730053784552344574, 3255002245038872702],
                [6587241094142920615, 11809313843902847396],
            ];
            static SOME_FILTER_BIN: [u8; 100] = [
                193, 92, 2, 137, 236, 45, 10, 145, 14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 13, 213, 0, 0, 0,
                0, 108, 233, 188, 116, 235, 155, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 209, 30, 98, 48, 112, 96, 0, 0, 0, 0, 0, 0, 69, 125, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 223, 21, 0, 0, 198, 225, 145, 206, 0, 0, 99, 63, 0, 0, 217, 218, 224, 20,
                0, 0, 0, 0, 0, 0, 0, 0,
            ];
            let filter = EuiFilter::from_bin(&SOME_FILTER_BIN);
            assert!(!filter.contains(&Eui {
                appeui: 0,
                deveui: 0,
            }));
            for [deveui, appeui] in SOME_KEYS.iter() {
                assert!(filter.contains(&Eui {
                    appeui: *appeui,
                    deveui: *deveui,
                }))
            }
        }
    }
}
