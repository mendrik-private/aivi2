use std::hash::{Hash, Hasher};

use num_bigint::{BigInt, Sign};
use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, PartialOrd)]
pub struct RuntimeFloat(f64);

impl RuntimeFloat {
    pub fn new(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self(value))
    }

    pub fn parse_literal(raw: &str) -> Option<Self> {
        let value = raw.parse::<f64>().ok()?;
        Self::new(value)
    }

    pub const fn to_f64(self) -> f64 {
        self.0
    }
}

impl PartialEq for RuntimeFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for RuntimeFloat {}

// Safety: `RuntimeFloat::new` rejects NaN and infinities, so every stored
// value is a finite f64 whose bit pattern is a stable, canonical identifier.
impl Hash for RuntimeFloat {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl std::fmt::Display for RuntimeFloat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut rendered = self.0.to_string();
        if !rendered.contains(['.', 'e', 'E']) {
            rendered.push_str(".0");
        }
        f.write_str(&rendered)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeDecimal(Decimal);

impl RuntimeDecimal {
    pub(crate) const ENCODED_BYTES: usize = 20;

    pub fn parse_literal(raw: &str) -> Option<Self> {
        let digits = raw.strip_suffix('d')?;
        let value = digits.parse::<Decimal>().ok()?;
        Some(Self(value))
    }

    pub(crate) fn encode_constant_bytes(&self) -> Box<[u8]> {
        let mut bytes = Vec::with_capacity(Self::ENCODED_BYTES);
        bytes.extend_from_slice(&self.0.mantissa().to_le_bytes());
        bytes.extend_from_slice(&self.0.scale().to_le_bytes());
        bytes.into_boxed_slice()
    }

    pub(crate) fn from_constant_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::ENCODED_BYTES {
            return None;
        }
        let mantissa = i128::from_le_bytes(bytes[..16].try_into().ok()?);
        let scale = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
        Some(Self(Decimal::from_i128_with_scale(mantissa, scale)))
    }

    pub(crate) fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl std::fmt::Display for RuntimeDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}d", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeBigInt(BigInt);

impl RuntimeBigInt {
    pub(crate) const HEADER_BYTES: usize = 16;

    pub fn parse_literal(raw: &str) -> Option<Self> {
        let digits = raw.strip_suffix('n')?;
        let value = digits.parse::<BigInt>().ok()?;
        Some(Self(value))
    }

    pub(crate) fn encode_constant_bytes(&self) -> Box<[u8]> {
        let (sign, magnitude) = self.0.to_bytes_le();
        let mut bytes = Vec::with_capacity(16 + magnitude.len());
        bytes.push(match sign {
            Sign::NoSign => 0,
            Sign::Plus => 1,
            Sign::Minus => 2,
        });
        bytes.extend_from_slice(&[0; 7]);
        bytes.extend_from_slice(&(magnitude.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&magnitude);
        bytes.into_boxed_slice()
    }

    pub(crate) fn from_constant_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::HEADER_BYTES {
            return None;
        }
        let sign = match bytes[0] {
            0 => Sign::NoSign,
            1 => Sign::Plus,
            2 => Sign::Minus,
            _ => return None,
        };
        let magnitude_len = u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize;
        let magnitude = bytes.get(Self::HEADER_BYTES..Self::HEADER_BYTES + magnitude_len)?;
        Some(Self(BigInt::from_bytes_le(sign, magnitude)))
    }

    pub(crate) fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }

    pub(crate) fn from_i64(n: i64) -> Self {
        Self(BigInt::from(n))
    }

    pub(crate) fn to_i64(&self) -> Option<i64> {
        use num_traits::ToPrimitive;
        self.0.to_i64()
    }

    pub(crate) fn from_decimal_str(s: &str) -> Option<Self> {
        s.trim().parse::<BigInt>().ok().map(Self)
    }

    pub(crate) fn to_decimal_str(&self) -> Box<str> {
        self.0.to_string().into_boxed_str()
    }

    pub(crate) fn bigint_add(&self, other: &Self) -> Self {
        Self(&self.0 + &other.0)
    }

    pub(crate) fn bigint_sub(&self, other: &Self) -> Self {
        Self(&self.0 - &other.0)
    }

    pub(crate) fn bigint_mul(&self, other: &Self) -> Self {
        Self(&self.0 * &other.0)
    }

    pub(crate) fn bigint_div(&self, other: &Self) -> Option<Self> {
        use num_traits::Zero;
        if other.0.is_zero() {
            None
        } else {
            Some(Self(&self.0 / &other.0))
        }
    }

    pub(crate) fn bigint_rem(&self, other: &Self) -> Option<Self> {
        use num_traits::Zero;
        if other.0.is_zero() {
            None
        } else {
            Some(Self(&self.0 % &other.0))
        }
    }

    pub(crate) fn bigint_pow(&self, exp: u32) -> Self {
        if exp == 0 {
            return Self(BigInt::from(1u32));
        }
        let mut result = BigInt::from(1u32);
        let mut base = self.0.clone();
        let mut n = exp;
        while n > 0 {
            if n & 1 == 1 {
                result *= &base;
            }
            base = &base * &base;
            n >>= 1;
        }
        Self(result)
    }

    pub(crate) fn bigint_neg(&self) -> Self {
        Self(-self.0.clone())
    }

    pub(crate) fn bigint_abs(&self) -> Self {
        use num_traits::Signed;
        Self(self.0.abs())
    }
}

impl std::fmt::Display for RuntimeBigInt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}n", self.0)
    }
}
