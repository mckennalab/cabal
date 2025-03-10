
use std::cmp::Ordering;
use std::fmt;
use std::ops::{BitAnd, BitOr, BitXor, Shl, Shr};
use serde::{Serialize, Deserialize};


/// our core Fasta base - representing a single fasta character in a u8 data store. We actually pack
/// everything into a u4 (if that was a type) but FastaString below handles the more dense packing and
/// unpacking into u64 structures
#[derive(Clone, Copy, Serialize, Deserialize, Hash)]
pub struct FastaBase(u8);
//type FastaBase = u8;

impl FastaBase {
    pub fn valid(ch: &u8) -> bool {
        u8_to_encoding(ch).is_some()
    }

    #[inline(always)]
    pub fn identity(&self, other: &FastaBase) -> bool {
        //(*self ^ *other).0 == 0 //-- even this slight indirection (to double access the .0 member) was costing us -- we go right to the source below
        self.0 ^ other.0 == 0
    }
    #[inline(always)]
    pub fn strict_identity(&self, other: &FastaBase) -> bool {
        //(*self ^ *other).0 == 0 //-- even this slight indirection (to double access the .0 member) was costing us -- we go right to the source below
        self.0 == other.0
    }

    pub fn from_string(st: &str) -> Vec<FastaBase> {
        st.chars().map(|c| u8_to_encoding(&(c as u8)).unwrap()).collect()
    }

    pub fn from_str(st: &str) -> Vec<FastaBase> {
        st.chars().map(|c| u8_to_encoding(&(c as u8)).unwrap()).collect()
    }

    pub fn from_vec_u8(st: &[u8]) -> Vec<FastaBase> {
        st.iter().map(|c| u8_to_encoding(c).unwrap_or_else(|| panic!("Vec<u8> to FastaBase failed on: {}", c))).collect()
    }

    pub fn from_u8_slice(st: &[u8]) -> Vec<FastaBase> {
        let rt = st.iter().map(|c| u8_to_encoding(c).unwrap()).collect::<Vec<FastaBase>>();
        rt.to_vec()
    }

    pub fn from_vec_u8_default_ns(st: &[u8]) -> Vec<FastaBase> {
        st.iter().map(u8_to_encoding_defaulted_to_n).collect()
    }

    pub fn string(bases: &[FastaBase]) -> String {
        String::from_utf8(FastaBase::vec_u8(bases)).unwrap()
    }

    pub fn string_from_slice(bases: &[FastaBase]) -> String {
        String::from_utf8(FastaBase::slice_to_vec_u8(bases)).unwrap()
    }

    pub fn slice_to_vec_u8(bases: &[FastaBase]) -> Vec<u8> {
        bases.iter().map(encoding_to_u8).collect()
    }

    pub fn vec_u8(bases: &[FastaBase]) -> Vec<u8> {
        bases.iter().map(encoding_to_u8).collect()
    }

    pub fn strip_gaps(bases: &[FastaBase]) -> Vec<FastaBase> {
        bases.iter().filter(|b| *b != &FASTA_UNSET).cloned().collect()
    }

    pub fn vec_u8_strip_gaps(bases: &[FastaBase]) -> Vec<u8> {
        bases.iter().filter(|x| *x != &FASTA_UNSET).map(encoding_to_u8).collect()
    }

    pub fn edit_distance(fasta_bases: &Vec<FastaBase>, other: &Vec<FastaBase>) -> usize {
        assert_eq!(fasta_bases.len(), other.len());
        let mut distance = 0;
        for (base, other_base) in fasta_bases.iter().zip(other.iter()) {
            if !base.identity(other_base) {
                distance += 1;
            }
        }
        distance
    }
}

impl From<u8> for FastaBase {
    fn from(ch: u8) -> Self {
        u8_to_encoding(&ch).unwrap()
    }
}

/// TODO: Should this exist?
impl From<char> for FastaBase {
    fn from(ch: char) -> Self {
        assert_eq!(ch.len_utf8(), 1); // trying to be a bit safer
        u8_to_encoding(&(ch as u8)).unwrap()
    }
}

impl fmt::Debug for FastaBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", encoding_to_u8(self) as char)
    }
}


/// our comparisons are done using logical AND operations for speed (the layout of bits is really important).
/// Each degenerate base should also be 'equal' to the correct 'ACGT' matches and be equal to other degenerate
/// bases that share any overlap in their base patterns. E.g. R == K, but R != C (and N == everything)
impl PartialEq for FastaBase {
    fn eq(&self, other: &Self) -> bool {
        self.0 & other.0 > 0
    }
}

impl Eq for FastaBase {}

impl PartialOrd for FastaBase {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Right now we simply offer a stable sort for bases -- there's no natural ordering to nucleotides;
/// you could argue alphabetical, but we simply sort on their underlying bit encoding
impl Ord for FastaBase {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl fmt::Display for FastaBase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", encoding_to_u8(self))
    }
}


impl BitOr for FastaBase {
    type Output = FastaBase;
    fn bitor(self, rhs: FastaBase) -> FastaBase {
        FastaBase(self.0.bitor(rhs.0))
    }
}

impl BitXor for FastaBase {
    type Output = FastaBase;
    fn bitxor(self, rhs: FastaBase) -> FastaBase {
        FastaBase(self.0.bitxor(rhs.0))
    }
}

impl BitAnd for FastaBase {
    type Output = FastaBase;
    fn bitand(self, rhs: FastaBase) -> FastaBase {
        FastaBase(self.0.bitand(rhs.0))
    }
}

impl BitAnd for &FastaBase {
    type Output = FastaBase;
    fn bitand(self, rhs: &FastaBase) -> FastaBase {
        FastaBase(self.0.bitand(rhs.0))
    }
}

impl Shl<usize> for FastaBase {
    type Output = FastaBase;

    fn shl(self, rhs: usize) -> FastaBase {
        FastaBase(self.0 << rhs)
    }
}

impl Shr<usize> for FastaBase {
    type Output = FastaBase;

    fn shr(self, rhs: usize) -> FastaBase {
        FastaBase(self.0 >> rhs)
    }
}


/// TODO: so I couldn't figure out how to create the const version as a standard trait, so I did it here (self note: just look at u8 impl you dummy)
const fn add_two_string_encodings(a: FastaBase, b: FastaBase) -> FastaBase {
    FastaBase(a.0 + b.0)
}

pub const FASTA_UNSET: FastaBase = FastaBase(0x10);
pub const FASTA_N: FastaBase = FastaBase(0xF);
pub const FASTA_A: FastaBase = FastaBase(0x1);
pub const FASTA_C: FastaBase = FastaBase(0x2);
pub const FASTA_G: FastaBase = FastaBase(0x4);
pub const FASTA_T: FastaBase = FastaBase(0x8);
pub const FASTA_R: FastaBase = add_two_string_encodings(FASTA_A, FASTA_G);
pub const FASTA_Y: FastaBase = add_two_string_encodings(FASTA_C, FASTA_T);
pub const FASTA_K: FastaBase = add_two_string_encodings(FASTA_G, FASTA_T);
pub const FASTA_M: FastaBase = add_two_string_encodings(FASTA_A, FASTA_C);
pub const FASTA_S: FastaBase = add_two_string_encodings(FASTA_C, FASTA_G);
pub const FASTA_W: FastaBase = add_two_string_encodings(FASTA_A, FASTA_T);
pub const FASTA_B: FastaBase = add_two_string_encodings(FASTA_C, add_two_string_encodings(FASTA_G, FASTA_T));
pub const FASTA_D: FastaBase = add_two_string_encodings(FASTA_A, add_two_string_encodings(FASTA_G, FASTA_T));
pub const FASTA_H: FastaBase = add_two_string_encodings(FASTA_A, add_two_string_encodings(FASTA_C, FASTA_T));
pub const FASTA_V: FastaBase = add_two_string_encodings(FASTA_A, add_two_string_encodings(FASTA_C, FASTA_G));


pub fn encoding_to_u8(base: &FastaBase) -> u8 {
    match base {
        x if x.0 == FASTA_UNSET.0 => { b'-' }
        x if x.0 == FASTA_A.0 => { b'A' }
        x if x.0 == FASTA_C.0 => { b'C' }
        x if x.0 == FASTA_G.0 => { b'G' }
        x if x.0 == FASTA_T.0 => { b'T' }

        x if x.0 == FASTA_R.0 => { b'R' }
        x if x.0 == FASTA_Y.0 => { b'Y' }
        x if x.0 == FASTA_K.0 => { b'K' }
        x if x.0 == FASTA_M.0 => { b'M' }

        x if x.0 == FASTA_S.0 => { b'S' }
        x if x.0 == FASTA_W.0 => { b'W' }
        x if x.0 == FASTA_B.0 => { b'B' }
        x if x.0 == FASTA_D.0 => { b'D' }

        x if x.0 == FASTA_H.0 => { b'H' }
        x if x.0 == FASTA_V.0 => { b'V' }
        x if x.0 == FASTA_N.0 => { b'N' }
        _ => {
            println!("Unable to convert {}", base.0);
            panic!("Unable to convert {:?}", base)
        }
    }
}

fn u8_to_encoding_defaulted_to_n(base: &u8) -> FastaBase {
    match base {
        b'-' => FASTA_UNSET,

        b'A' | b'a' => FASTA_A,
        b'C' | b'c' => FASTA_C,
        b'G' | b'g' => FASTA_G,
        b'T' | b't' => FASTA_T,

        b'R' | b'r' => FASTA_R,
        b'Y' | b'y' => FASTA_Y,
        b'K' | b'k' => FASTA_K,
        b'M' | b'm' => FASTA_M,

        b'S' | b's' => FASTA_S,
        b'W' | b'w' => FASTA_W,
        b'B' | b'b' => FASTA_B,
        b'D' | b'd' => FASTA_D,

        b'H' | b'h' => FASTA_H,
        b'V' | b'v' => FASTA_V,

        _ => FASTA_N,
    }
}

#[allow(dead_code)]
fn u8_to_encoding(base: &u8) -> Option<FastaBase> {
    match base {
        b'-' => Some(FASTA_UNSET),

        b'A' | b'a' => Some(FASTA_A),
        b'C' | b'c' => Some(FASTA_C),
        b'G' | b'g' => Some(FASTA_G),
        b'T' | b't' => Some(FASTA_T),

        b'R' | b'r' => Some(FASTA_R),
        b'Y' | b'y' => Some(FASTA_Y),
        b'K' | b'k' => Some(FASTA_K),
        b'M' | b'm' => Some(FASTA_M),

        b'S' | b's' => Some(FASTA_S),
        b'W' | b'w' => Some(FASTA_W),
        b'B' | b'b' => Some(FASTA_B),
        b'D' | b'd' => Some(FASTA_D),

        b'H' | b'h' => Some(FASTA_H),
        b'V' | b'v' => Some(FASTA_V),

        b'N' | b'n' => Some(FASTA_N),
        _ => {
            None
        }
    }
}

// see this RFC about why we have to do the match guard approach: https://rust-lang.github.io/rfcs/1445-restrict-constants-in-patterns.html
fn complement(base: &FastaBase) -> FastaBase {
    match *base {
        x if x.identity(&FASTA_UNSET) => FASTA_UNSET,
        x if x.identity(&FASTA_N) => FASTA_N,
        x if x.identity(&FASTA_A) => FASTA_T,
        x if x.identity(&FASTA_C) => FASTA_G,
        x if x.identity(&FASTA_T) => FASTA_A,
        x if x.identity(&FASTA_G) => FASTA_C,
        x if x.identity(&FASTA_R) => FASTA_Y,
        x if x.identity(&FASTA_Y) => FASTA_R,
        x if x.identity(&FASTA_K) => FASTA_M,
        x if x.identity(&FASTA_M) => FASTA_K,
        x if x.identity(&FASTA_S) => FASTA_W,
        x if x.identity(&FASTA_W) => FASTA_S,
        x if x.identity(&FASTA_B) => FASTA_V,
        x if x.identity(&FASTA_V) => FASTA_B,
        x if x.identity(&FASTA_D) => FASTA_H,
        x if x.identity(&FASTA_H) => FASTA_D,
        _ => panic!("Unknown base {}", base),
    }
}

pub(crate) fn reverse_complement(bases: &[FastaBase]) -> Vec<FastaBase> {
    let mut new_bases = bases.iter().map(|b| {
        complement(b)
    }).collect::<Vec<FastaBase>>();
    new_bases.reverse();
    new_bases
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Instant;
    use super::*;

    #[test]
    fn test_identity() {
        assert!(FASTA_N.identity(&FASTA_N));
        assert!(!FASTA_N.identity(&FASTA_A));
    }

    #[test]
    fn test_u8_to_encoding_defaulted_to_n() {
        assert_eq!(FASTA_A, u8_to_encoding_defaulted_to_n(&b'A'));
        assert_eq!(FASTA_C, u8_to_encoding_defaulted_to_n(&b'C'));
        assert_eq!(FASTA_G, u8_to_encoding_defaulted_to_n(&b'G'));
        assert_eq!(FASTA_T, u8_to_encoding_defaulted_to_n(&b'T'));

        assert_eq!(FASTA_A, u8_to_encoding_defaulted_to_n(&b'a'));
        assert_eq!(FASTA_C, u8_to_encoding_defaulted_to_n(&b'c'));
        assert_eq!(FASTA_G, u8_to_encoding_defaulted_to_n(&b'g'));
        assert_eq!(FASTA_T, u8_to_encoding_defaulted_to_n(&b't'));

        assert_eq!(FASTA_N, u8_to_encoding_defaulted_to_n(&b'N'));
        assert_eq!(FASTA_N, u8_to_encoding_defaulted_to_n(&b'n'));

        assert_eq!(FASTA_B, u8_to_encoding_defaulted_to_n(&b'B'));
        assert_eq!(FASTA_B, u8_to_encoding_defaulted_to_n(&b'b'));

        assert_eq!(FASTA_D, u8_to_encoding_defaulted_to_n(&b'D'));
        assert_eq!(FASTA_D, u8_to_encoding_defaulted_to_n(&b'd'));

        assert_eq!(FASTA_R, u8_to_encoding_defaulted_to_n(&b'R'));
        assert_eq!(FASTA_R, u8_to_encoding_defaulted_to_n(&b'r'));

        assert_eq!(FASTA_Y, u8_to_encoding_defaulted_to_n(&b'Y'));
        assert_eq!(FASTA_Y, u8_to_encoding_defaulted_to_n(&b'y'));

        assert_eq!(FASTA_K, u8_to_encoding_defaulted_to_n(&b'K'));
        assert_eq!(FASTA_K, u8_to_encoding_defaulted_to_n(&b'k'));

        assert_eq!(FASTA_M, u8_to_encoding_defaulted_to_n(&b'M'));
        assert_eq!(FASTA_M, u8_to_encoding_defaulted_to_n(&b'm'));

        assert_eq!(FASTA_S, u8_to_encoding_defaulted_to_n(&b'S'));
        assert_eq!(FASTA_S, u8_to_encoding_defaulted_to_n(&b's'));

        assert_eq!(FASTA_W, u8_to_encoding_defaulted_to_n(&b'W'));
        assert_eq!(FASTA_W, u8_to_encoding_defaulted_to_n(&b'w'));

        assert_eq!(FASTA_H, u8_to_encoding_defaulted_to_n(&b'H'));
        assert_eq!(FASTA_H, u8_to_encoding_defaulted_to_n(&b'h'));

        assert_eq!(FASTA_V, u8_to_encoding_defaulted_to_n(&b'V'));
        assert_eq!(FASTA_V, u8_to_encoding_defaulted_to_n(&b'v'));
    }

    #[test]
    fn bit_compare_simple() {
        let bit_one = FASTA_A; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_A; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        assert_eq!(bit_one, bit_two);

        let bit_one = FASTA_A; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_C; // StringEncodingPair { bases: FASTA_C, mask: SINGLE_BIT_MASK };
        assert_ne!(bit_one, bit_two);

        let bit_one = FASTA_A; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_N; // StringEncodingPair { bases: FASTA_N, mask: SINGLE_BIT_MASK };
        assert_eq!(bit_one, bit_two);

        let bit_one = FASTA_A; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_R; // StringEncodingPair { bases: FASTA_R, mask: SINGLE_BIT_MASK };
        assert_eq!(bit_one, bit_two);
    }

    #[test]
    fn bit_complement() {
        let reference = FastaBase::from_vec_u8("CCAATCTACTACTGCTTGCA".as_bytes());
        let ref_rev = reverse_complement(&reference);
        let ref_rev2 = reverse_complement(&ref_rev);
        assert_eq!(reference, ref_rev2);
    }

    #[test]
    fn bit_ordering() {
        assert!(FASTA_N > FASTA_A);
        assert!(FASTA_A < FASTA_N);
    }

    #[test]
    fn bit_ordering_vec() {
        let str1 = vec![FASTA_N, FASTA_N, FASTA_N, FASTA_N, FASTA_N];
        let str2 = vec![FASTA_A, FASTA_C, FASTA_G, FASTA_N, FASTA_N];
        assert!(str1 > str2);
    }

    #[test]
    fn test_string_conversion() {
        let str1 = String::from_utf8("NNNNNNNNNNNNNNNNNNNNNNNNNNNNTTACGTTNNNNNNNNNNNNGATGTACCTGTCATCTTAGCTAAGATGACAGGACATGTCCAGGAAGTACTCGAGTACTTCCTGGCCCATGTACTCTGCGTTGATACCACTGCTT".as_bytes().to_vec()).unwrap();
        let str2_partial = &FastaBase::from_string(&str1);
        assert_eq!(str2_partial.get(0).unwrap().clone(), FASTA_N);
        let str2 = FastaBase::string(str2_partial);
        assert_eq!(str1, str2);
    }

    #[test]
    fn bit_compare_all() {
        let mut known_mapping = HashMap::new();
        let mut known_mismapping = HashMap::new();

        known_mapping.insert(b'A', vec![b'A']);
        known_mapping.insert(b'C', vec![b'C']);
        known_mapping.insert(b'G', vec![b'G']);
        known_mapping.insert(b'T', vec![b'T']);
        known_mismapping.insert(b'A', vec![b'C', b'G', b'T']);
        known_mismapping.insert(b'C', vec![b'A', b'G', b'T']);
        known_mismapping.insert(b'G', vec![b'A', b'C', b'T']);
        known_mismapping.insert(b'T', vec![b'A', b'C', b'G']);

        known_mapping.insert(b'R', vec![b'A', b'G']);
        known_mapping.insert(b'Y', vec![b'C', b'T']);
        known_mapping.insert(b'K', vec![b'G', b'T']);
        known_mapping.insert(b'M', vec![b'A', b'C']);
        known_mapping.insert(b'S', vec![b'C', b'G']);
        known_mapping.insert(b'W', vec![b'A', b'T']);

        known_mismapping.insert(b'R', vec![b'T', b'C', b'Y']);
        known_mismapping.insert(b'Y', vec![b'G', b'A', b'R']);
        known_mismapping.insert(b'K', vec![b'A', b'C', b'M']);
        known_mismapping.insert(b'M', vec![b'G', b'T', b'K']);
        known_mismapping.insert(b'S', vec![b'A', b'T', b'W']);
        known_mismapping.insert(b'W', vec![b'C', b'G', b'S']);

        known_mapping.insert(b'B', vec![b'C', b'G', b'T']);
        known_mapping.insert(b'D', vec![b'A', b'G', b'T']);
        known_mapping.insert(b'H', vec![b'A', b'C', b'T']);
        known_mapping.insert(b'V', vec![b'A', b'C', b'G']);

        known_mismapping.insert(b'B', vec![b'A']);
        known_mismapping.insert(b'D', vec![b'C']);
        known_mismapping.insert(b'H', vec![b'G']);
        known_mismapping.insert(b'V', vec![b'T']);

        known_mapping.insert(b'N', vec![b'A', b'C', b'G', b'T']);

        known_mapping.iter().for_each(|(x, y)| {
            y.iter().for_each(|z| {

                assert_eq!(u8_to_encoding_defaulted_to_n(x),
                           u8_to_encoding_defaulted_to_n(z),
                           "Testing {} and {}",
                           String::from_utf8(vec![*x]).unwrap(),
                           String::from_utf8(vec![*z]).unwrap());
            })
        });

        known_mismapping.iter().for_each(|(x, y)| {
            y.iter().for_each(|z| {
                assert_ne!(u8_to_encoding(x).unwrap(),
                           u8_to_encoding(z).unwrap(),
                           "Testing {} and {}",
                           String::from_utf8(vec![*x]).unwrap(),
                           String::from_utf8(vec![*z]).unwrap());

                assert_ne!(u8_to_encoding_defaulted_to_n(x),
                           u8_to_encoding_defaulted_to_n(z),
                           "Testing {} and {}",
                           String::from_utf8(vec![*x]).unwrap(),
                           String::from_utf8(vec![*z]).unwrap());
            })
        });

        assert!(!FASTA_N.strict_identity(&FASTA_A));
    }

    #[test]
    fn to_fasta_base_and_back() {
        let bases = "ACGTACGTACGT---".to_string();
        let fasta_bases = FastaBase::from_string(&bases);
        let rebased = FastaBase::string(&fasta_bases);
        assert_eq!(bases, rebased);

        let bases = "ACGTRYKMSWBDHVN-".to_string();
        let fasta_bases = FastaBase::from_string(&bases);
        let rebased = FastaBase::string(&fasta_bases);
        assert_eq!(bases, rebased);
    }

    #[test]
    fn bit_compare_complex() {
        let bit_one = FASTA_R; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_A; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        assert_eq!(bit_one, bit_two);

        let bit_one = FASTA_R; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_M; // StringEncodingPair { bases: FASTA_C, mask: SINGLE_BIT_MASK };
        assert_eq!(bit_one, bit_two);

        let bit_one = FASTA_M; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_N; // StringEncodingPair { bases: FASTA_N, mask: SINGLE_BIT_MASK };
        assert_eq!(bit_one, bit_two);

        let bit_one = FASTA_V; // StringEncodingPair { bases: FASTA_A, mask: SINGLE_BIT_MASK };
        let bit_two = FASTA_T; // StringEncodingPair { bases: FASTA_R, mask: SINGLE_BIT_MASK };
        assert_ne!(bit_one, bit_two);
    }

    #[test]
    fn test_many_u8s_vs_many_fasta_bases() {
        let bases = "CCAATCTACTACTGACCGATAGATATTTTAGAGCACACACACACATATAGAGAGTCTTGCA".as_bytes().to_vec();
        let bases2 = "GGGGTCTACTACTGACCGATAGATATTTTAGAGCACACACACACATATAGAGAGTCTTGCA".as_bytes().to_vec();
        let fbases = FastaBase::from_vec_u8(&bases);
        let fbases2 = FastaBase::from_vec_u8(&bases2);

        let start = Instant::now();
        let mut _not_equal = 0;
        for _i in 0..10000000 {
            if bases != bases2 {
                _not_equal += 1;
            }
        }
        let duration = start.elapsed();
        println!("Time elapsed in aligning u8 reads is: {:?}", duration);

        let start2 = Instant::now();
        let mut _not_equal = 0;
        for _i in 0..10000000 {
            if fbases != fbases2 {
                _not_equal += 1;
            }
        }
        let duration2 = start2.elapsed();
        println!("Time elapsed in aligning fasta is: {:?}", duration2);

        /* DEGENERATEBASES
        let start3 = Instant::now();
        let mut not_equal = 0;
        for i in 0..10000000 {
            for j in 0..(bases.len()-1) {
                if DEGENERATEBASES.get(&bases[j]).unwrap().contains_key(&bases2[j]) {
                    not_equal += 1;
                }
            }
        }
        let duration3 = start.elapsed();
        println!("Time elapsed in aligning fasta is: {:?}", duration2);*/
    }

/*
        #[test]
        fn basic_bitstring_conversion() {
            let base = "A";
            let converted = FastaString::from(&base);
            let str_version = format!("{:#64b}",converted.packed_bases[0]);

            assert_eq!(str_version," 0b1000000000000000000000000000000000000000000000000000000000000");

            let base = "AN";
            let converted = FastaString::from(&base);
            let str_version = format!("{:#64b}",converted.packed_bases[0]);

            assert_eq!(str_version," 0b1111100000000000000000000000000000000000000000000000000000000");

            let base = "NNNNNNNNNNNNNNNN";
            let converted = FastaString::from(&base);
            let str_version = format!("{:#64b}",converted.packed_bases[0]);

            assert_eq!(str_version,"0b1111111111111111111111111111111111111111111111111111111111111111");

            let base = "NNNNNNNNNNNNNNNNN";
            let converted = FastaString::from(&base);
            let str_version = format!("{:#64b}",converted.packed_bases[0]);

            assert_eq!(str_version,"0b1111111111111111111111111111111111111111111111111111111111111111");
            let str_version = format!("{:#64b}",converted.packed_bases[1]);

            assert_eq!(str_version,"0b1111000000000000000000000000000000000000000000000000000000000000");

            let base = "NNNNNNNNNNNNNNNNA";
            let converted = FastaString::from(&base);
            let str_version = format!("{:#64b}",converted.packed_bases[0]);

            assert_eq!(str_version,"0b1111111111111111111111111111111111111111111111111111111111111111");
            let str_version = format!("{:#64b}",converted.packed_bases[1]);

            assert_eq!(str_version," 0b1000000000000000000000000000000000000000000000000000000000000");
        }



    #[test]
    fn base_compare_speeds() {
        let comp_count = 100000000;
        let fasta_a = FASTA_A;
        let fasta_c = FASTA_C;
        // do 100K comparisons
        let now = Instant::now();

        for _i in 0..comp_count {
            fasta_a == fasta_c;
        }
        println!("equals {}", now.elapsed().as_millis());

        let fasta_a = FASTA_A;
        let fasta_c = FASTA_C;
        // do 100K comparisons
        let now = Instant::now();

        for _i in 0..comp_count {
            fasta_a.identity(&fasta_c);
        }
        println!("ident {}", now.elapsed().as_millis());

        let fasta_a = FASTA_A.0;
        let fasta_c = FASTA_C.0;
        // do 100K comparisons
        let now = Instant::now();

        for _i in 0..comp_count {
            fasta_a | fasta_c == 0;
        }
        println!("ident stripped  {}", now.elapsed().as_millis());
        let byte_a = b'A';
        let byte_c = b'C';
        // do 100K comparisons
        let now = Instant::now();

        for _i in 0..comp_count {
            byte_a == byte_c;
        }
        println!("bytes {}", now.elapsed().as_millis());


        let fasta_n = FASTA_A;
        let fasta_c = FASTA_C;
        // do 100K comparisons
        let now = Instant::now();

        for _i in 0..comp_count {
            fasta_n.identity(&fasta_c);
        }
        println!("ident degenerate {}", now.elapsed().as_millis());

        let byte_n = b'N';
        let byte_a = b'A';
        let byte_c = b'C';
        let byte_g = b'G';
        let byte_t = b'T';
        // do 100K comparisons
        let now = Instant::now();

        for _i in 0..comp_count {
            let _trash = byte_a == byte_n || byte_c == byte_n || byte_g == byte_n || byte_t == byte_n;
        }
        println!("bytes {}", now.elapsed().as_millis());


        //self.0.bitor(other.0) == 0
    }

*/
}
