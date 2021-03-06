//! Curve41417 scalar operations
use serialize::hex::ToHex;
use std::default::Default;
use std::fmt::{Show, Formatter, Result};
use std::io::extensions;
use std::rand::{Rand, Rng};

use bytes::{B416, B832, Bytes, Scalar, Uniformity};
use sbuf::{DefaultAllocator, SBuf};
use utils;


static SCE_SIZE: uint = 52;

// L = 2^411 - d
//   = 2^411 - 33364140863755142520810177694098385178984727200411208589594759
static L: [u8, ..52] = [
  0x79, 0xaf, 0x06, 0xe1, 0xa5, 0x71, 0x0e, 0x1b,
  0x18, 0xcf, 0x63, 0xad, 0x38, 0x03, 0x1c, 0x6f,
  0xb3, 0x22, 0x60, 0x70, 0xcf, 0x14, 0x24, 0xc9,
  0x3c, 0xeb, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
  0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
  0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
  0xff, 0xff, 0xff, 0x07];

// LD = 2^5 * d
static LD: [u8, ..27] = [
  0xe0, 0x10, 0x2a, 0xdf, 0x43, 0xcb, 0x31, 0x9e,
  0xfc, 0x1c, 0x86, 0x53, 0xea, 0x98, 0x7f, 0x1c,
  0x92, 0xa9, 0xfb, 0xf3, 0x11, 0x66, 0x7d, 0xdb,
  0x66, 0x98, 0x02];


/// Scalar element used in scalar operations.
///
/// Provide commons Curve41417 scalar operations computed `mod L`, where
/// `L` is the order of the base point.
#[deriving(Clone)]
pub struct ScalarElem {
    elem: SBuf<DefaultAllocator, i64>
}

impl ScalarElem {
    // Return a new scalar element with its value set to `0`.
    fn new_zero() -> ScalarElem {
        ScalarElem::zero()
    }

    /// Generate a new random `ScalarElem` between `[0, L-1]`, its value is
    /// not clamped. Use urandom as PRNG.
    pub fn new_rand() -> ScalarElem {
        let rng = &mut utils::urandom_rng();
        Rand::rand(rng)
    }

    /// Return scalar value representing `0`.
    pub fn zero() -> ScalarElem {
        ScalarElem {
            elem: SBuf::new_zero(SCE_SIZE)
        }
    }

    // Return a reference to the limb at index `index`. Fails if
    // `index` is out of bounds.
    #[doc(hidden)]
    pub fn get<'a>(&'a self, index: uint) -> &'a i64 {
        self.elem.get(index)
    }

    // Return a mutable reference to the limb at index `index`. Fails
    // if `index` is out of bounds.
    #[doc(hidden)]
    pub fn get_mut<'a>(&'a mut self, index: uint) -> &'a mut i64 {
        self.elem.get_mut(index)
    }

    // Conditionally swap this scalar element with `other`. `cond` serves
    // as condition and must be `0` or `1` strictly. Values are swapped iff
    // `cond == 1`.
    fn cswap(&mut self, cond: i64, other: &mut ScalarElem) {
        utils::bytes_cswap::<i64>(cond,
                                  self.elem.as_mut_slice(),
                                  other.elem.as_mut_slice());
    }

    // Requirements: len >= 52
    fn carry(&mut self) {
        let top = self.len() - 1;
        let mut carry: i64;

        for i in range(0u, top) {
            *self.get_mut(i) += 1_i64 << 8;
            carry = *self.get(i) >> 8;
            *self.get_mut(i + 1) += carry - 1;
            *self.get_mut(i) -= carry << 8;
        }

        *self.get_mut(top) += 1_i64 << 8;
        carry = *self.get(top) >> 8;
        for i in range(0u, 27) {
            *self.get_mut(top - 51 + i) += (carry - 1) * (LD[i] as i64);
        }
        *self.get_mut(top) -= carry << 8;
    }

    // Reduce mod 2^416 - 2^5 * d and put limbs between [0, 2^16-1] through
    // carry.
    // Requirements: 52 < nlen <= 104
    fn reduce_weak(&mut self, n: &[i64]) {
        assert!(n.len() > 52);
        assert!(n.len() <= 104);

        let mut t: SBuf<DefaultAllocator, i64> = SBuf::new_zero(78);
        for i in range(0u, 52) {
            *t.get_mut(i) = n[i];
        }

        for i in range(52u, n.len()) {
            for j in range(0u, 27) {
                *t.get_mut(i + j - 52) += n[i] * (LD[j] as i64);
            }
        }

        for i in range(52u, n.len() - 26) {
            for j in range(0u, 27) {
                *t.get_mut(i + j - 52) += *t.get(i) * (LD[j] as i64);
            }
        }

        for i in range(0u, 52) {
            *self.get_mut(i) = *t.get(i);
        }

        self.carry();
        self.carry();
    }

    fn reduce(&mut self) {
        self.carry();
        self.carry();

        // Eliminate multiples of 2^411
        let mut carry: i64 = 0;
        for i in range(0u, 52) {
            *self.get_mut(i) += carry - (*self.get(51) >> 3) * (L[i] as i64);
            carry = *self.get(i) >> 8;
            *self.get_mut(i) &= 0xff;
        }

        // Substract L a last time in case n is in [L, 2^411-1]
        let mut m = ScalarElem::new_zero();
        carry = 0;
        for i in range(0u, 52) {
            *m.get_mut(i) = *self.get(i) + carry - (L[i] as i64);
            carry = *m.get(i) >> 8;
            *m.get_mut(i) &= 0xff;
        }
        self.cswap(1 - (carry & 1), &mut m);
    }

    fn unpack_wo_reduce<T: Bytes>(n: &T) -> ScalarElem {
        let mut r = ScalarElem::new_zero();

        // Note: would be great to also check/assert that n is in [0, L - 1].
        for i in range(0u, 52) {
            *r.get_mut(i) = *n.get(i) as i64;
        }
        r
    }

    fn unpack_w_reduce<T: Bytes>(n: &T) -> ScalarElem {
        let l = n.as_bytes().len();
        let mut t: SBuf<DefaultAllocator, i64> = SBuf::new_zero(l);

        for i in range(0u, l) {
            *t.get_mut(i) = *n.get(i) as i64;
        }

        let mut r = ScalarElem::new_zero();
        r.reduce_weak(t.as_slice());
        r
    }

    /// Unpack `n`:
    ///
    /// * If `n` is a `B416` instance it should represent a value in `[0, L-1]`
    ///   and will not be reduced on unpacking.
    /// * For larger values of `n` i.e. for `B512` and `B832` instances, `n`
    ///   is weakly reduced on input. `B832` might provide a better uniformity
    ///   of distribution on reductions `mod L`.
    ///
    /// In any case it is not until its result is packed back to a byte
    /// representation (through `pack()` method) that it will be reduced to
    /// its canonical form.
    pub fn unpack<T: Bytes>(n: &T) -> Option<ScalarElem> {
        let l = n.as_bytes().len();

        match l {
            52 => Some(ScalarElem::unpack_wo_reduce(n)),
            52..104 => Some(ScalarElem::unpack_w_reduce(n)),
            _ => None
        }
    }

    /// Pack the current scalar value reduced `mod L`.
    pub fn pack(&self) -> Scalar {
        let mut t = self.clone();
        t.reduce();

        let mut b: B416 = Bytes::new_zero();
        for i in range(0u, 52) {
            *b.get_mut(i) = (*t.get(i) & 0xff) as u8;
        }
        Scalar(b)
    }

    /// Pack scalar value `n` reduced `n mod L`.
    pub fn reduce_from_bytes<T: Bytes + Uniformity>(n: &T) -> Scalar {
        ScalarElem::unpack(n).unwrap().pack()
    }
}

impl Add<ScalarElem, ScalarElem> for ScalarElem {
    /// Add scalars.
    fn add(&self, other: &ScalarElem) -> ScalarElem {
        let mut r = self.clone();
        for i in range(0u, self.len()) {
            *r.get_mut(i) += *other.get(i);
        }
        r
    }
}

impl Sub<ScalarElem, ScalarElem> for ScalarElem {
    /// Substract scalars.
    fn sub(&self, other: &ScalarElem) -> ScalarElem {
        let mut r = self.clone();
        for i in range(0u, self.len()) {
            *r.get_mut(i) -= *other.get(i);
        }
        r
    }
}

impl Neg<ScalarElem> for ScalarElem {
    /// Negate scalar.
    fn neg(&self) -> ScalarElem {
        ScalarElem::zero() - *self
    }
}

impl Mul<ScalarElem, ScalarElem> for ScalarElem {
    /// Multiply scalars.
    fn mul(&self, other: &ScalarElem) -> ScalarElem {
        let mut t: SBuf<DefaultAllocator, i64> = SBuf::new_zero(103);

        for i in range(0u, 52) {
            for j in range(0u, 52) {
                *t.get_mut(i + j) += *self.get(i) * *other.get(j);
            }
        }

        let mut r = ScalarElem::new_zero();
        r.reduce_weak(t.as_slice());
        r
    }
}

impl FromPrimitive for ScalarElem {
    #[allow(unused_variable)]
    fn from_i64(n: i64) -> Option<ScalarElem> {
        None
    }

    fn from_u64(n: u64) -> Option<ScalarElem> {
        let mut s: B416 = Bytes::new_zero();
        extensions::u64_to_le_bytes(n, 8, |v| {
            for (sb, db) in v.iter().zip(s.as_mut_bytes().mut_iter()) {
                *db = *sb;
            }
        });
        ScalarElem::unpack(&s)
    }
}

impl Default for ScalarElem {
    /// Return the scalar value 0 as default.
    fn default() -> ScalarElem {
        ScalarElem::new_zero()
    }
}

impl Rand for ScalarElem {
    /// Generate a random `ScalarElem` between `[0, L-1]`, and its value
    /// is not clamped. Be sure to use a secure PRNG when calling this
    /// method. For instance `ScalarElem::new_rand()` uses urandom.
    fn rand<R: Rng>(rng: &mut R) -> ScalarElem {
        let b: B832 = Rand::rand(rng);
        ScalarElem::unpack(&b).unwrap()
    }
}

impl Show for ScalarElem {
    /// Format as hex-string.
    fn fmt(&self, f: &mut Formatter) -> Result {
        self.pack().fmt(f)
    }
}

impl ToHex for ScalarElem {
    fn to_hex(&self) -> String {
        self.pack().to_hex()
    }
}

impl Eq for ScalarElem {
}

impl PartialEq for ScalarElem {
    /// Constant-time equality comparison.
    fn eq(&self, other: &ScalarElem) -> bool {
        self.pack() == other.pack()
    }
}

impl Collection for ScalarElem {
    fn len(&self) -> uint {
        self.elem.len()
    }
}


#[cfg(test)]
mod tests {
    use bytes::{B416, B512, B832, Bytes};
    use sc::ScalarElem;


    #[test]
    fn test_ops_b416() {
        let n1: B416 = Bytes::new_rand();
        let a = ScalarElem::unpack(&n1).unwrap();

        let apa = a + a;
        let aaa1 = a * apa;
        let s1 = aaa1 - a;

        let aa = a * a;
        let aaa2 = aa + aa;
        let s2 = aaa2 - a;

        assert!(s1 == s2);
        assert!(s1 != aaa2);
    }

    #[test]
    fn test_ops_b512() {
        let n1: B512 = Bytes::new_rand();
        let a = ScalarElem::unpack(&n1).unwrap();

        let apa = a + a;
        let aaa1 = a * apa;
        let s1 = aaa1 - a;

        let aa = a * a;
        let aaa2 = aa + aa;
        let s2 = aaa2 - a;

        assert!(s1 == s2);
    }

    #[test]
    fn test_ops_b832() {
        let n1: B832 = Bytes::new_rand();
        let a = ScalarElem::unpack(&n1).unwrap();

        let apa = a + a;
        let aaa1 = a * apa;
        let s1 = aaa1 - a;

        let aa = a * a;
        let aaa2 = aa + aa;
        let s2 = aaa2 - a;

        assert!(s1 == s2);
    }

    #[test]
    fn test_ops_416_ref() {
        let n: [u8, ..52] = [
            0xf6, 0xf0, 0x53, 0xb3, 0x79, 0x46, 0x2d, 0x51,
            0xc9, 0xea, 0xcf, 0xef, 0x0e, 0x4d, 0xaa, 0xbe,
            0x17, 0xee, 0xfd, 0xf7, 0x46, 0x98, 0x1f, 0xde,
            0xbf, 0xf2, 0xe2, 0xb7, 0xdc, 0x04, 0xf5, 0xad,
            0xa5, 0x09, 0x32, 0x8d, 0x4a, 0x0a, 0x5d, 0x77,
            0x19, 0xa6, 0xce, 0xc6, 0xf0, 0x49, 0xa8, 0x00,
            0xde, 0x7d, 0x31, 0x73];
        let r: [u8, ..52] = [
            0xd9, 0x60, 0x53, 0xb2, 0x78, 0x38, 0xf6, 0x41,
            0x49, 0x6e, 0x35, 0x1f, 0xd5, 0xc5, 0x58, 0xee,
            0x43, 0x0b, 0xd0, 0xe5, 0x06, 0x08, 0xf7, 0xc8,
            0x5a, 0xc4, 0xaf, 0x84, 0x37, 0x97, 0x0a, 0x38,
            0x66, 0xd2, 0xbc, 0x17, 0xc4, 0xec, 0xf3, 0x14,
            0x48, 0x0c, 0x76, 0xf2, 0x8d, 0x9e, 0x46, 0x31,
            0x6d, 0x16, 0x07, 0x02];

        let nn: B416 = Bytes::from_bytes(n).unwrap();
        let rr: B416 = Bytes::from_bytes(r).unwrap();

        let a = ScalarElem::unpack(&nn).unwrap();

        let apa = a + a;
        let aaa1 = a * apa;
        let s = aaa1 - a;

        assert!(s.pack().unwrap() == rr);
    }

    #[test]
    fn test_ops_512_ref() {
        let n: [u8, ..64] = [
            0xf3, 0xa5, 0x35, 0x47, 0xec, 0xcf, 0xa6, 0x84,
            0x03, 0x7f, 0xaa, 0x34, 0x62, 0x7a, 0xb6, 0x2e,
            0x18, 0xa4, 0x5c, 0xdd, 0xd7, 0x54, 0x72, 0x0b,
            0x80, 0xe5, 0xcf, 0xd4, 0x5e, 0x8a, 0x3f, 0x8e,
            0x0f, 0x6f, 0xe1, 0xbe, 0x1b, 0x6c, 0xaf, 0x45,
            0x8d, 0x51, 0xcc, 0xef, 0x87, 0xa4, 0x0d, 0x2c,
            0x87, 0xb0, 0xdd, 0x07, 0x3a, 0xf7, 0xe3, 0x16,
            0x12, 0x8c, 0x3b, 0x8b, 0x86, 0x0f, 0x78, 0xbe];
        let r: [u8, ..52] = [
            0x8d, 0x44, 0xdd, 0xae, 0x17, 0xd2, 0x48, 0x44,
            0x37, 0x5a, 0x1d, 0xb7, 0x7e, 0xde, 0x28, 0xde,
            0xc6, 0x3d, 0xa6, 0xc8, 0x87, 0x9b, 0x0b, 0xf0,
            0x46, 0xba, 0xb3, 0xf8, 0x55, 0x76, 0xe5, 0xe7,
            0x2f, 0x61, 0x40, 0xb2, 0xda, 0x99, 0xf7, 0x12,
            0x9e, 0x28, 0x2f, 0xce, 0x0e, 0x34, 0xf9, 0xb2,
            0x91, 0xb3, 0x31, 0x06];

        let nn: B512 = Bytes::from_bytes(n).unwrap();
        let rr: B416 = Bytes::from_bytes(r).unwrap();

        let a = ScalarElem::unpack(&nn).unwrap();

        let apa = a + a;
        let aaa1 = a * apa;
        let s = aaa1 - a;

        assert!(s.pack().unwrap() == rr);
    }

    #[test]
    fn test_ops_832_ref() {
        let n: [u8, ..104] = [
            0x14, 0x48, 0x03, 0x95, 0x83, 0x87, 0x9a, 0x7d,
            0xb6, 0x36, 0x02, 0x97, 0xa0, 0x2c, 0x25, 0x2d,
            0xf1, 0xa1, 0xa0, 0x45, 0xa7, 0x9a, 0xef, 0x04,
            0xa9, 0x14, 0xf4, 0xb1, 0xfd, 0x24, 0x4c, 0x85,
            0x94, 0x4a, 0xd5, 0x02, 0xf8, 0x07, 0x94, 0xaf,
            0xba, 0xb9, 0x83, 0x38, 0xae, 0x59, 0xa6, 0xe3,
            0x22, 0xfa, 0xd6, 0x64, 0x8f, 0xa1, 0x92, 0x36,
            0x96, 0x29, 0xe2, 0x4e, 0x80, 0x62, 0x61, 0xda,
            0xed, 0xb2, 0x04, 0x53, 0x33, 0xca, 0xf1, 0x8f,
            0x11, 0x33, 0xed, 0x22, 0x75, 0x6a, 0x55, 0x4c,
            0x34, 0xce, 0x65, 0x94, 0xbb, 0x38, 0xe4, 0x62,
            0xe3, 0x55, 0xbb, 0x82, 0x53, 0x78, 0x87, 0x32,
            0x79, 0xbe, 0x9b, 0x23, 0x61, 0xf3, 0xf6, 0x19];
        let r: [u8, ..52] = [
            0x1c, 0x2c, 0x61, 0xb6, 0xc8, 0xda, 0x85, 0x77,
            0x2b, 0x70, 0x2a, 0x54, 0xb0, 0x83, 0x49, 0xfc,
            0xc1, 0x33, 0x91, 0x37, 0x63, 0x90, 0x00, 0x13,
            0x4b, 0xde, 0x0b, 0xd2, 0x06, 0x07, 0xac, 0x54,
            0x1e, 0x3f, 0x75, 0x9f, 0x82, 0x07, 0x74, 0xd4,
            0xf5, 0x8e, 0xd8, 0xc7, 0x66, 0xcc, 0x3c, 0x23,
            0xde, 0x63, 0x9d, 0x01];

        let nn: B832 = Bytes::from_bytes(n).unwrap();
        let rr: B416 = Bytes::from_bytes(r).unwrap();

        let a = ScalarElem::unpack(&nn).unwrap();

        let apa = a + a;
        let aaa1 = a * apa;
        let s = aaa1 - a;

        assert!(s.pack().unwrap() == rr);
    }

    #[test]
    fn test_modl_416_ref() {
        let n: [u8, ..52] = [
            0xf6, 0xf0, 0x53, 0xb3, 0x79, 0x46, 0x2d, 0x51,
            0xc9, 0xea, 0xcf, 0xef, 0x0e, 0x4d, 0xaa, 0xbe,
            0x17, 0xee, 0xfd, 0xf7, 0x46, 0x98, 0x1f, 0xde,
            0xbf, 0xf2, 0xe2, 0xb7, 0xdc, 0x04, 0xf5, 0xad,
            0xa5, 0x09, 0x32, 0x8d, 0x4a, 0x0a, 0x5d, 0x77,
            0x19, 0xa6, 0xce, 0xc6, 0xf0, 0x49, 0xa8, 0x00,
            0xde, 0x7d, 0x31, 0x73];
        let r: [u8, ..52] = [
            0x58, 0x58, 0xf6, 0x64, 0x67, 0x0f, 0x63, 0xd6,
            0x77, 0x97, 0x5a, 0x74, 0xf5, 0x1f, 0x22, 0xab,
            0x47, 0x08, 0xbc, 0xd2, 0xee, 0x74, 0x26, 0xde,
            0x6c, 0x15, 0xe4, 0xb7, 0xdc, 0x04, 0xf5, 0xad,
            0xa5, 0x09, 0x32, 0x8d, 0x4a, 0x0a, 0x5d, 0x77,
            0x19, 0xa6, 0xce, 0xc6, 0xf0, 0x49, 0xa8, 0x00,
            0xde, 0x7d, 0x31, 0x03];

        let nn: B416 = Bytes::from_bytes(n).unwrap();
        let rr: B416 = Bytes::from_bytes(r).unwrap();

        let s = ScalarElem::unpack(&nn).unwrap();
        assert!(s.pack().unwrap() == rr);
    }

    #[test]
    fn test_modl_512_ref() {
        let n: [u8, ..64] = [
            0xf3, 0xa5, 0x35, 0x47, 0xec, 0xcf, 0xa6, 0x84,
            0x03, 0x7f, 0xaa, 0x34, 0x62, 0x7a, 0xb6, 0x2e,
            0x18, 0xa4, 0x5c, 0xdd, 0xd7, 0x54, 0x72, 0x0b,
            0x80, 0xe5, 0xcf, 0xd4, 0x5e, 0x8a, 0x3f, 0x8e,
            0x0f, 0x6f, 0xe1, 0xbe, 0x1b, 0x6c, 0xaf, 0x45,
            0x8d, 0x51, 0xcc, 0xef, 0x87, 0xa4, 0x0d, 0x2c,
            0x87, 0xb0, 0xdd, 0x07, 0x3a, 0xf7, 0xe3, 0x16,
            0x12, 0x8c, 0x3b, 0x8b, 0x86, 0x0f, 0x78, 0xbe];
        let r: [u8, ..52] = [
            0xb3, 0x98, 0xa5, 0xa3, 0x1e, 0x89, 0x39, 0xaf,
            0x6c, 0xfe, 0x18, 0x6e, 0x6f, 0xaf, 0xef, 0xea,
            0x7a, 0x52, 0xac, 0xc9, 0x43, 0xe3, 0x61, 0xff,
            0xc1, 0x51, 0x11, 0xfb, 0xe0, 0x09, 0xc6, 0x5a,
            0xa6, 0x99, 0x4a, 0xae, 0x6f, 0x5a, 0xb1, 0x45,
            0x8d, 0x51, 0xcc, 0xef, 0x87, 0xa4, 0x0d, 0x2c,
            0x87, 0xb0, 0xdd, 0x07];

        let nn: B512 = Bytes::from_bytes(n).unwrap();
        let rr: B416 = Bytes::from_bytes(r).unwrap();

        let s = ScalarElem::unpack(&nn).unwrap();
        assert!(s.pack().unwrap() == rr);
    }

    #[test]
    fn test_modl_832_ref() {
        let n: [u8, ..104] = [
            0x14, 0x48, 0x03, 0x95, 0x83, 0x87, 0x9a, 0x7d,
            0xb6, 0x36, 0x02, 0x97, 0xa0, 0x2c, 0x25, 0x2d,
            0xf1, 0xa1, 0xa0, 0x45, 0xa7, 0x9a, 0xef, 0x04,
            0xa9, 0x14, 0xf4, 0xb1, 0xfd, 0x24, 0x4c, 0x85,
            0x94, 0x4a, 0xd5, 0x02, 0xf8, 0x07, 0x94, 0xaf,
            0xba, 0xb9, 0x83, 0x38, 0xae, 0x59, 0xa6, 0xe3,
            0x22, 0xfa, 0xd6, 0x64, 0x8f, 0xa1, 0x92, 0x36,
            0x96, 0x29, 0xe2, 0x4e, 0x80, 0x62, 0x61, 0xda,
            0xed, 0xb2, 0x04, 0x53, 0x33, 0xca, 0xf1, 0x8f,
            0x11, 0x33, 0xed, 0x22, 0x75, 0x6a, 0x55, 0x4c,
            0x34, 0xce, 0x65, 0x94, 0xbb, 0x38, 0xe4, 0x62,
            0xe3, 0x55, 0xbb, 0x82, 0x53, 0x78, 0x87, 0x32,
            0x79, 0xbe, 0x9b, 0x23, 0x61, 0xf3, 0xf6, 0x19];
        let r: [u8, ..52] = [
            0xe5, 0x43, 0x46, 0x3b, 0xb2, 0x52, 0x16, 0xc0,
            0x8a, 0xdb, 0x92, 0x72, 0xae, 0x59, 0xef, 0xaa,
            0x17, 0xb4, 0xa3, 0x3b, 0x8c, 0x88, 0xcc, 0xf6,
            0x39, 0x71, 0xc5, 0xe0, 0x1e, 0x0e, 0xe1, 0x6e,
            0x22, 0xe8, 0xf1, 0x9a, 0xf1, 0x4e, 0x0e, 0x00,
            0xd4, 0x42, 0x49, 0xcf, 0x33, 0x49, 0x07, 0xdf,
            0xb1, 0x3a, 0xee, 0x00];

        let nn: B832 = Bytes::from_bytes(n).unwrap();
        let rr: B416 = Bytes::from_bytes(r).unwrap();

        let s = ScalarElem::unpack(&nn).unwrap();
        assert!(s.pack().unwrap() == rr);
    }

    #[test]
    fn test_from_u64() {
        let n: u64 = 72623859790382856;
        let b: [u8, ..52] = [
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0];

        let bb: B416 = Bytes::from_bytes(b).unwrap();

        let s1 = ScalarElem::unpack(&bb).unwrap();
        let s2: ScalarElem = FromPrimitive::from_u64(n).unwrap();

        assert!(s1 == s2);
    }
}
