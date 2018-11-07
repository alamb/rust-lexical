//! Extended precision floating-point types.
//!
//! Also contains helpers to convert to and from native rust floats.
//! This representation stores the fraction as a 64-bit unsigned integer,
//! and the exponent as a 32-bit unsigned integer, allowed ~80 bits of
//! precision (only 16 bits of the 32-bit integer are used, u32 is used
//! for performance). Since there is no storage for the sign bit,
//! this only works for positive floats.
//!
//! For the relative performance, on an x86-64 Intel CPU, the float
//! multiply operations are ~4-7x as costly as an f64 multiply, and ~1.5-2x
//! as costly as an f32 multiply (due to f32 expansion and then shrinking).
//! Likewise, addition is ~3.25x as expensive as f32 and f64 operations,
//! and subtract is ~2.3x slower. Since this is less than 1 order of
//! magnitude different for massive precision wins, we consider this a
//! success.

// TODO(ahuszagh)
//  1. Need a fast powi()
//  2. Need to implement atof in terms of this precise functionality.

// SHIFTS

/// Shift left `shift` bytes.
macro_rules! shl {
    ($self:ident, $shift:expr) => ({
        $self.frac <<= $shift;
        $self.exp -= $shift;
    })
}

/// Lossless shift left `shift` bytes.
macro_rules! check_shl {
    ($self:ident, $base:expr, $shift:expr) => ({
        if $self.frac >> ($base - $shift) == 0 {
            shl!($self, $shift);
        }
    })
}

/// Shift right `shift` bytes.
macro_rules! shr {
    ($self:ident, $shift:expr) => ({
        $self.frac >>= $shift;
        $self.exp += $shift;
    })
}

/// Shift the FloatType fraction to the fraction bits in a native float.
///
/// Floating-point arithmetic uses round to nearest, ties to even,
/// which rounds to the nearest value, if the value is halfway in between,
/// round to an even value.
///
/// * `self`        - Floating point to shift bits in.
/// * `trunc_mask`  - Mask to extract all bits beyond `64 - mantissa size - 1` bits.
/// * `trunc_mid`   - Midway point for trunc mask, `trunc_mask/2 + 1`.
/// * `odd_mask`    - Mask to extract the `64 - mantissa size` bit.
/// * `carry_mask`  - Mask to extract the bit after the hidden bit, or `HIDDEN_BIT_MASK * 2`.
/// * `shift`       - Number of bits to shift, or `64 - mantissa size - 1`.
macro_rules! shift_to_float {
    ($self:ident, $trunc_mask:ident, $trunc_mid:ident, $odd_mask:ident, $carry_mask:ident, $shift:ident) => ({
        // Extract the truncate bits from the number (64 - mantissa size - 1 bits).
        // Calculate if the truncated bits are either above the mid-way
        // point, or equal to it.
        //
        // For example, for 4 truncated bytes, the midway point would be
        // b1000.
        let truncated_bits = $self.frac & $trunc_mask;
        let is_above = truncated_bits > $trunc_mid;
        let is_halfway = truncated_bits == $trunc_mid;

        // Extract the last non-truncated (41st) bit (and determine if it is odd).
        let is_odd = $self.frac & $odd_mask == $odd_mask;

        // Calculate if we need to roundup.
        let is_roundup = is_above || (is_odd && is_halfway);

        // Bitshift so the leading bit is in the hidden bit.
        shr!($self, $shift);

        // Roundup and then shift if a full carry occurs.
        $self.frac += is_roundup as u64;
        if $self.frac & $carry_mask == $carry_mask {
            // Roundup carried over to 1 past the hidden bit.
            shr!($self, 1);
        }
    })
}

/// Shift the fraction to the fraction bits in an f32.
macro_rules! shift_to_f32 {
    ($self:ident) => ({
        const TRUNC_MASK: u64 = 0xFFFFFFFFFF;
        const TRUNC_MID: u64 = 0x8000000000;
        const ODD_MASK: u64 = 0x10000000000;
        const CARRY_MASK: u64 = 0x1000000;
        const SHIFT: i32 = 40;
        shift_to_float!($self, TRUNC_MASK, TRUNC_MID, ODD_MASK, CARRY_MASK, SHIFT)
    })
}

/// Shift the fraction to the fraction bits in an f64.
macro_rules! shift_to_f64 {
    ($self:ident) => ({
        const TRUNC_MASK: u64 = 0x7FF;
        const TRUNC_MID: u64 = 0x400;
        const ODD_MASK: u64 = 0x800;
        const CARRY_MASK: u64 = 0x20000000000000;
        const SHIFT: i32 = 11;
        shift_to_float!($self, TRUNC_MASK, TRUNC_MID, ODD_MASK, CARRY_MASK, SHIFT)
    })
}

/// Avoid underflow for denormalized values.
macro_rules! avoid_underflow {
    ($self:ident, $denormal:ident) => ({
        // Calculate the difference to allow a single calculation
        // rather than a loop, to minimize the number of ops required.
        if $self.exp < $denormal {
            let diff = $denormal - $self.exp;
            if $self.frac >> diff != 0 {
                $self.frac >>= diff;
                $self.exp += diff;
            }
        }
    })
}

/// Avoid overflow for large values, shift left as needed.
///
/// Shift until a 1-bit is in the hidden bit
macro_rules! avoid_overflow {
    ($self:ident, $max:ident, $masks:ident) => ({
        // Calculate the difference to allow a single calculation
        // rather than a loop, using a precalculated bitmask table,
        // minimizing the number of ops required.
        if $self.exp >= $max {
            let diff = $self.exp - $max;
            let idx = diff as usize;
            if idx < $masks.len() {
                let mask = unsafe { *$masks.get_unchecked(idx) };
                if $self.frac & mask == 0 {
                    // If we have no 1-bit in the hidden-bit position,
                    // which is index 0, we need to shift 1.
                    let shift = diff + 1;
                    $self.frac <<= shift;
                    $self.exp -= shift;
                }
            }
        }
    })
}

/// Normalize a FloatType to a representation to export to f32.
macro_rules! normalize_to_f32 {
    ($self:ident, $denormal:ident, $max:ident, $masks:ident) => ({
        // Shift all the way left, to ensure a consistent representation.
        // The following right-shifts do not work for a non-normalized number.
        $self.normalize();
        shift_to_f32!($self);
        avoid_underflow!($self, $denormal);
        avoid_overflow!($self, $max, $masks)
    })
}

/// Normalize a FloatType to a representation to export to f64.
macro_rules! normalize_to_f64 {
    ($self:ident, $denormal:ident, $max:ident, $masks:ident) => ({
        // Shift all the way left, to ensure we have a valid start point.
        // The following right-shifts do not work for a non-normalized number.
        $self.normalize();
        shift_to_f64!($self);
        avoid_underflow!($self, $denormal);
        avoid_overflow!($self, $max, $masks)
    })
}

// FROM INT

/// Import FloatType from integer.
///
/// This works because we call normalize before any operation, which
/// allows us to convert the integer representation to the float one.
macro_rules! from_int {
    ($int:ident) => ({
        FloatType {
            frac: $int as u64,
            exp: 0,
        }
    })
}

// FROM FLOAT

/// Import FloatType from native float.
macro_rules! from_float {
    ($float:ident, $exponent:ident, $hidden:ident,
     $fraction:ident, $bias:ident, $sig_size:ident)
    => ({
        let bits = $float.to_bits() as u64;
        let mut fp = FloatType {
            frac: (bits & $fraction),
            exp: ((bits & $exponent) >> $sig_size) as i32,
        };

        if fp.exp != 0 {
            fp.frac += $hidden;
            fp.exp -= $bias;
        } else {
            fp.exp = -$bias + 1;
        }

        fp
    })
}

// AS FLOAT

/// Export FloatType normalized to native float to native float.
macro_rules! as_float {
    ($self:ident, $float:tt, $int:ty, $denormal:ident, $hidden:ident,
     $fraction:ident, $bias:ident, $max:ident, $inf:ident, $sig_size:ident)
    => ({
        // Export floating-point number.
        if $self.frac == 0 || $self.exp < $denormal {
            // sub-denormal, underflow
            0.0
        } else if $self.exp >= $max {
            // overflow
            $float::from_bits($inf)
        } else {
            // calculate the exp and fraction bits, and return a float from bits.
            let exp: $int;
            if ($self.exp == $denormal) && ($self.frac & $hidden) == 0 {
                exp = 0;
            } else {
                exp = ($self.exp + $bias) as $int;
            }
            let exp = exp << $sig_size;
            let frac = $self.frac & $fraction;
            $float::from_bits(frac as $int | exp)
        }
    })
}

// OPERATIONS

/// Conditionally add x and y, where x has the higher exponent.
macro_rules! conditional_add {
    ($x:ident, $y:ident) => ({
        // Calculate the difference for the shift-right, use it conditionally.
        let diff = $x.exp - $y.exp;
        if diff < 64 {
            // Less than 64-bits, shr is safe.
            let mut y = *$y;
            shr!(y, diff);
            y.add_unchecked($x)
        } else {
            // More than 64-bits, self is insignificant.
            *$x
        }
    })
}

// HARD-CODED MULTIPLY
// We need this for atof functionality. We need to parse each digit,
// multiply the result the base quickly, and then add the digit to the
// running total. We therefore need fast multiplication for 2-36, both
// checked and unchecked.

/// Generate wrappers for an unchecked power of two multiply.
macro_rules! multiply_pow2_impl {
    ($self:ident, $exp:expr) => (FloatType {frac: $self.frac, exp: $self.exp + $exp})
}

/// Generate wrappers for an unchecked multiply that is not a power of 2.
///
/// * `factor`      - Multiplier for the value.
/// * `max`         - Minimum value that would lead to integer overflow.
/// * `shift`       - Number to shift right if overflow would occur.
macro_rules! multiply_not_pow2_impl {
    ($self:ident, $factor:expr, $shift:expr) => ({
        const MAX: u64 = u64::max_value() / $factor + 1;
        if $self.frac < MAX {
            FloatType {frac: $factor * $self.frac, exp: $self.exp}
        } else {
            let mut x = *$self;
            shr!(x, $shift);
            x.frac *= $factor;
            x
        }
    })
}

/// Generate the checked and unchecked wrappers for multiply.
macro_rules! multiply_n_impl {
    ($checked:ident, $unchecked:ident, $macro:ident $(, $e:expr)*) => (
        /// Fast, unchecked multiply-by-N algorithm.
        #[inline]
        pub unsafe fn $unchecked(&self) -> FloatType {
            $macro!(self $(, $e)*)
        }

        /// Fast multiply-by-N algorithm which checks for infinity.
        #[inline]
        pub fn $checked(&self) -> FloatType {
            // We don't have to be stringent here, since we use 32-bits than 16.
            if self.exp < Self::F64_MAX_EXPONENT {
                unsafe { self.$unchecked() }
            } else {
                *self
            }
        }
    )
}

// FLOAT TYPE

/// Extended precision floating-point type.
///
/// Contains conversions to and from f64.
#[repr(C)]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatType {
    /// Has ~80 bits of precision (~16 for exponent).
    /// Use the 32-bit type first, for a packed alignment.
    pub exp: i32,
    pub frac: u64,
}

impl FloatType {
    // MASKS
    // 32-bit
    // Bit-mask for the sign.
    pub const F32_SIGN_MASK: u64        = 0x0000000080000000;
    /// Bit-mask for the exponent, including the hidden bit.
    pub const F32_EXPONENT_MASK: u64    = 0x000000007F800000;
    /// Bit-mask for the hidden bit in exponent, which is use for the fraction.
    pub const F32_HIDDEN_BIT_MASK: u64  = 0x0000000000800000;
    /// Bit-mask for the mantissa (fraction), excluding the hidden bit.
    pub const F32_FRACTION_MASK: u64    = 0x00000000007FFFFF;
    // 64-bit
    /// Bit-mask for the sign.
    pub const F64_SIGN_MASK: u64        = 0x8000000000000000;
    /// Bit-mask for the exponent, including the hidden bit.
    pub const F64_EXPONENT_MASK: u64    = 0x7FF0000000000000;
    /// Bit-mask for the hidden bit in exponent, which is use for the fraction.
    pub const F64_HIDDEN_BIT_MASK: u64  = 0x0010000000000000;
    /// Bit-mask for the mantissa (fraction), excluding the hidden bit.
    pub const F64_FRACTION_MASK: u64    = 0x000FFFFFFFFFFFFF;

    // PROPERTIES
    // 32-bit
    pub const U32_INFINITY: u32         = 0x7F800000;
    /// Size of a 32-bit float.
    pub const F32_FLOAT_SIZE: i32 = 32;
    /// Size of the significand (mantissa) without the hidden bit.
    pub const F32_SIGNIFICAND_SIZE: i32 = 23;
    /// Size of the exponent including the hidden bit.
    pub const F32_EXPONENT_SIZE: i32 = Self::F32_FLOAT_SIZE - Self::F32_SIGNIFICAND_SIZE - 1;
    /// Bias of the exponent.
    pub const F32_EXPONENT_BIAS: i32 = 127 + Self::F32_SIGNIFICAND_SIZE;
    /// Exponent portion of a denormal float.
    pub const F32_DENORMAL_EXPONENT: i32 = -Self::F32_EXPONENT_BIAS + 1;
    /// Maximum exponent (2^(F32_EXPONENT_SIZE) - 1 + F32_EXPONENT_BIAS).
    pub const F32_MAX_EXPONENT: i32 = 0xFF - Self::F32_EXPONENT_BIAS;
    /// Minimum exponent.
    pub const F32_MIN_EXPONENT: i32 = -Self::F32_EXPONENT_BIAS;
    // 64-bit
    /// Positive infinity as bits.
    pub const U64_INFINITY: u64         = 0x7FF0000000000000;
    /// Size of a 64-bit float.
    pub const F64_FLOAT_SIZE: i32 = 64;
    /// Size of the significand (mantissa) without the hidden bit.
    pub const F64_SIGNIFICAND_SIZE: i32 = 52;
    /// Size of the exponent including the hidden bit.
    pub const F64_EXPONENT_SIZE: i32 = Self::F64_FLOAT_SIZE - Self::F64_SIGNIFICAND_SIZE - 1;
    /// Bias of the exponent.
    pub const F64_EXPONENT_BIAS: i32 = 1023 + Self::F64_SIGNIFICAND_SIZE;
    /// Exponent portion of a denormal float.
    pub const F64_DENORMAL_EXPONENT: i32 = -Self::F64_EXPONENT_BIAS + 1;
    /// Maximum exponent (2^(F64_EXPONENT_SIZE) - 1 + F64_EXPONENT_BIAS).
    pub const F64_MAX_EXPONENT: i32 = 0x7FF - Self::F64_EXPONENT_BIAS;
    /// Minimum exponent.
    pub const F64_MIN_EXPONENT: i32 = -Self::F64_EXPONENT_BIAS;

    // OPERATIONS

    /// Add two extended-precision floating point numbers, as if by `a+b`.
    #[inline]
    pub fn add(&self, b: &FloatType) -> FloatType {
        // Produces better code than doing a tri-way comparison.
        unsafe {
            if self.exp > b.exp {
                self.add_less_equal(b)
            } else {
                b.add_less_equal(self)
            }
        }
    }

    /// Fast add, where b.exp <= a.exp.
    #[inline]
    pub unsafe fn add_less_equal(&self, b: &FloatType) -> FloatType {
        if self.exp == b.exp {
            self.add_unchecked(b)
        } else {
            conditional_add!(self, b)
        }
    }

    /// Fast add from u64.
    #[inline]
    pub fn add_u64(&self, b: u64) -> FloatType {
        self.add(&FloatType::from_u64(b))
    }

    /// Fast add from u64, where b.exp <= a.exp.
    #[inline]
    pub unsafe fn add_less_equal_u64(&self, b: u64) -> FloatType {
        self.add_less_equal(&FloatType::from_u64(b))
    }

    /// Add two extended-precision floats with the same exponent.
    #[inline]
    pub unsafe fn add_unchecked(&self, b: &FloatType) -> FloatType {
        debug_assert!(self.exp == b.exp);
        let (x, y) = self.frac.overflowing_add(b.frac);
        match y {
            // Overflow, shift both values left 1, then add.
            true  => {
                let x = self.frac >> 1;
                let y = b.frac >> 1;
                FloatType {frac: x + y, exp: self.exp + 1}
            },
            // No overflow, return our value.
            false => FloatType {frac: x, exp: self.exp},
        }
    }

    /// Subtract two extended-precision floats with the same exponent.
    ///
    /// Subtraction isn't well-supported, since negative values aren't
    /// supported. Only unsafe function calls may be made.
    #[inline]
    pub unsafe fn sub_unchecked(&self, b: &FloatType) -> FloatType {
        debug_assert!(self.frac >= b.frac);
        debug_assert!(self.exp == b.exp);
        FloatType {frac: self.frac - b.frac, exp: self.exp}
    }

    /// Multiply two normalized extended-precision floats, as if by `a*b`.
    ///
    /// Fast multiply, does not work for numbers of different magnitudes.
    #[inline]
    pub unsafe fn fast_multiply(&self, b: &FloatType) -> FloatType
    {
        const LOMASK: u64 = 0x00000000FFFFFFFF;

        let ah_bl = (self.frac >> 32)    * (b.frac & LOMASK);
        let al_bh = (self.frac & LOMASK) * (b.frac >> 32);
        let al_bl = (self.frac & LOMASK) * (b.frac & LOMASK);
        let ah_bh = (self.frac >> 32)    * (b.frac >> 32);

        let mut tmp = (ah_bl & LOMASK) + (al_bh & LOMASK) + (al_bl >> 32);
        // round up
        tmp += 1 << 31;

        FloatType {
            frac: ah_bh + (ah_bl >> 32) + (al_bh >> 32) + (tmp >> 32),
            exp: self.exp + b.exp + 64
        }
    }

    // HARD-CODED MULTIPLY
    // Fast multiply operations using hard-coded values.
    // These are meant to avoid the `mul` opcode, allowing the compiler
    // to optimize with a known value. Powers of 2 are optimized by just
    // incrementing the exponent, while other values check for overflow,
    // and shift-right (lossy) if required, before multiplying the fraction
    // by the multiplier.

    // BASE
    multiply_n_impl!(mul_2, mul_2_unchecked, multiply_pow2_impl, 1);
    multiply_n_impl!(mul_3, mul_3_unchecked, multiply_not_pow2_impl, 3, 2);
    multiply_n_impl!(mul_4, mul_4_unchecked, multiply_pow2_impl, 2);
    multiply_n_impl!(mul_5, mul_5_unchecked, multiply_not_pow2_impl, 5, 3);
    multiply_n_impl!(mul_6, mul_6_unchecked, multiply_not_pow2_impl, 6, 3);
    multiply_n_impl!(mul_7, mul_7_unchecked, multiply_not_pow2_impl, 7, 3);
    multiply_n_impl!(mul_8, mul_8_unchecked, multiply_pow2_impl, 3);
    multiply_n_impl!(mul_9, mul_9_unchecked, multiply_not_pow2_impl, 9, 4);
    multiply_n_impl!(mul_10, mul_10_unchecked, multiply_not_pow2_impl, 10, 4);
    multiply_n_impl!(mul_11, mul_11_unchecked, multiply_not_pow2_impl, 11, 4);
    multiply_n_impl!(mul_12, mul_12_unchecked, multiply_not_pow2_impl, 12, 4);
    multiply_n_impl!(mul_13, mul_13_unchecked, multiply_not_pow2_impl, 13, 4);
    multiply_n_impl!(mul_14, mul_14_unchecked, multiply_not_pow2_impl, 14, 4);
    multiply_n_impl!(mul_15, mul_15_unchecked, multiply_not_pow2_impl, 15, 4);
    multiply_n_impl!(mul_16, mul_16_unchecked, multiply_pow2_impl, 4);
    multiply_n_impl!(mul_17, mul_17_unchecked, multiply_not_pow2_impl, 17, 5);
    multiply_n_impl!(mul_18, mul_18_unchecked, multiply_not_pow2_impl, 18, 5);
    multiply_n_impl!(mul_19, mul_19_unchecked, multiply_not_pow2_impl, 19, 5);
    multiply_n_impl!(mul_20, mul_20_unchecked, multiply_not_pow2_impl, 20, 5);
    multiply_n_impl!(mul_21, mul_21_unchecked, multiply_not_pow2_impl, 21, 5);
    multiply_n_impl!(mul_22, mul_22_unchecked, multiply_not_pow2_impl, 22, 5);
    multiply_n_impl!(mul_23, mul_23_unchecked, multiply_not_pow2_impl, 23, 5);
    multiply_n_impl!(mul_24, mul_24_unchecked, multiply_not_pow2_impl, 24, 5);
    multiply_n_impl!(mul_25, mul_25_unchecked, multiply_not_pow2_impl, 25, 5);
    multiply_n_impl!(mul_26, mul_26_unchecked, multiply_not_pow2_impl, 26, 5);
    multiply_n_impl!(mul_27, mul_27_unchecked, multiply_not_pow2_impl, 27, 5);
    multiply_n_impl!(mul_28, mul_28_unchecked, multiply_not_pow2_impl, 28, 5);
    multiply_n_impl!(mul_29, mul_29_unchecked, multiply_not_pow2_impl, 29, 5);
    multiply_n_impl!(mul_30, mul_30_unchecked, multiply_not_pow2_impl, 30, 5);
    multiply_n_impl!(mul_31, mul_31_unchecked, multiply_not_pow2_impl, 31, 5);
    multiply_n_impl!(mul_32, mul_32_unchecked, multiply_pow2_impl, 5);
    multiply_n_impl!(mul_33, mul_33_unchecked, multiply_not_pow2_impl, 33, 6);
    multiply_n_impl!(mul_34, mul_34_unchecked, multiply_not_pow2_impl, 34, 6);
    multiply_n_impl!(mul_35, mul_35_unchecked, multiply_not_pow2_impl, 35, 6);
    multiply_n_impl!(mul_36, mul_36_unchecked, multiply_not_pow2_impl, 36, 6);

    // TODO(ahuszagh) Need a fast powi...

    /// Multiply by N to simplify testing purposes.
    pub fn mul_n(&self, n: u64) -> FloatType {
        match n {
            2  => self.mul_2(),
            3  => self.mul_3(),
            4  => self.mul_4(),
            5  => self.mul_5(),
            6  => self.mul_6(),
            7  => self.mul_7(),
            8  => self.mul_8(),
            9  => self.mul_9(),
            10 => self.mul_10(),
            11 => self.mul_11(),
            12 => self.mul_12(),
            13 => self.mul_13(),
            14 => self.mul_14(),
            15 => self.mul_15(),
            16 => self.mul_16(),
            17 => self.mul_17(),
            18 => self.mul_18(),
            19 => self.mul_19(),
            20 => self.mul_20(),
            21 => self.mul_21(),
            22 => self.mul_22(),
            23 => self.mul_23(),
            24 => self.mul_24(),
            25 => self.mul_25(),
            26 => self.mul_26(),
            27 => self.mul_27(),
            28 => self.mul_28(),
            29 => self.mul_29(),
            30 => self.mul_30(),
            31 => self.mul_31(),
            32 => self.mul_32(),
            33 => self.mul_33(),
            34 => self.mul_34(),
            35 => self.mul_35(),
            36 => self.mul_36(),
            _  => unreachable!(),
        }
    }

    /// Unchecked multiply by N to simplify testing purposes.
    pub unsafe fn mul_n_unchecked(&self, n: u64) -> FloatType {
        match n {
            2  => self.mul_2_unchecked(),
            3  => self.mul_3_unchecked(),
            4  => self.mul_4_unchecked(),
            5  => self.mul_5_unchecked(),
            6  => self.mul_6_unchecked(),
            7  => self.mul_7_unchecked(),
            8  => self.mul_8_unchecked(),
            9  => self.mul_9_unchecked(),
            10 => self.mul_10_unchecked(),
            11 => self.mul_11_unchecked(),
            12 => self.mul_12_unchecked(),
            13 => self.mul_13_unchecked(),
            14 => self.mul_14_unchecked(),
            15 => self.mul_15_unchecked(),
            16 => self.mul_16_unchecked(),
            17 => self.mul_17_unchecked(),
            18 => self.mul_18_unchecked(),
            19 => self.mul_19_unchecked(),
            20 => self.mul_20_unchecked(),
            21 => self.mul_21_unchecked(),
            22 => self.mul_22_unchecked(),
            23 => self.mul_23_unchecked(),
            24 => self.mul_24_unchecked(),
            25 => self.mul_25_unchecked(),
            26 => self.mul_26_unchecked(),
            27 => self.mul_27_unchecked(),
            28 => self.mul_28_unchecked(),
            29 => self.mul_29_unchecked(),
            30 => self.mul_30_unchecked(),
            31 => self.mul_31_unchecked(),
            32 => self.mul_32_unchecked(),
            33 => self.mul_33_unchecked(),
            34 => self.mul_34_unchecked(),
            35 => self.mul_35_unchecked(),
            36 => self.mul_36_unchecked(),
            _  => unreachable!(),
        }
    }

    // NORMALIZE

    /// Normalize float-point number.
    ///
    /// Let the integer component of the mantissa (significand) to be exactly
    /// 1 and the decimal fraction and exponent to be whatever.
    ///
    /// Adapted from Rust's `diy_float` class.
    ///     https://github.com/rust-lang/rust/blob/b7c6e8f1805cd8a4b0a1c1f22f17a89e9e2cea23/src/libcore/num/diy_float.rs#L49
    #[inline]
    pub fn normalize(&mut self)
    {
        check_shl!(self, 64, 32);
        check_shl!(self, 64, 16);
        check_shl!(self, 64, 8);
        check_shl!(self, 64, 4);
        check_shl!(self, 64, 2);
        check_shl!(self, 64, 1);
    }

    /// Get normalized boundaries for float.
    #[inline]
    pub fn normalized_boundaries(&self) -> (FloatType, FloatType) {
        let mut upper = FloatType {
            frac: (self.frac << 1) + 1,
            exp: self.exp - 1,
        };
        upper.normalize();

        let l_shift: i32 = if self.frac == Self::F64_HIDDEN_BIT_MASK { 2 } else { 1 };

        let mut lower = FloatType {
            frac: (self.frac << l_shift) - 1,
            exp: self.exp - l_shift,
        };
        lower.frac <<= lower.exp - upper.exp;
        lower.exp = upper.exp;

        (lower, upper)
    }

    /// Lossy normalize float-point number to f32 fraction boundaries.
    #[inline]
    fn normalize_to_f32(&mut self)
    {
        const DENORMAL: i32 = FloatType::F32_DENORMAL_EXPONENT;
        const MAX: i32 = FloatType::F32_MAX_EXPONENT;
        // Every mask from the hidden bit over, to see if we can
        // shift-left in 1 operation.
        const MASKS: [u64; 24] = [
            0x00800000, 0x00C00000, 0x00E00000, 0x00F00000, 0x00F80000, 0x00FC0000,
            0x00FE0000, 0x00FF0000, 0x00FF8000, 0x00FFC000, 0x00FFE000, 0x00FFF000,
            0x00FFF800, 0x00FFFC00, 0x00FFFE00, 0x00FFFF00, 0x00FFFF80, 0x00FFFFC0,
            0x00FFFFE0, 0x00FFFFF0, 0x00FFFFF8, 0x00FFFFFC, 0x00FFFFFE, 0x00FFFFFF
        ];

        normalize_to_f32!(self, DENORMAL, MAX, MASKS)
    }

    /// Lossy normalize float-point number to f64 fraction boundaries.
    #[inline]
    fn normalize_to_f64(&mut self)
    {
        const DENORMAL: i32 = FloatType::F64_DENORMAL_EXPONENT;
        const MAX: i32 = FloatType::F64_MAX_EXPONENT;
        // Every mask from the hidden bit over, to see if we can
        // shift-left in 1 operation.
        const MASKS: [u64; 53] = [
            0x0010000000000000, 0x0018000000000000, 0x001C000000000000,
            0x001E000000000000, 0x001F000000000000, 0x001F800000000000,
            0x001FC00000000000, 0x001FE00000000000, 0x001FF00000000000,
            0x001FF80000000000, 0x001FFC0000000000, 0x001FFE0000000000,
            0x001FFF0000000000, 0x001FFF8000000000, 0x001FFFC000000000,
            0x001FFFE000000000, 0x001FFFF000000000, 0x001FFFF800000000,
            0x001FFFFC00000000, 0x001FFFFE00000000, 0x001FFFFF00000000,
            0x001FFFFF80000000, 0x001FFFFFC0000000, 0x001FFFFFE0000000,
            0x001FFFFFF0000000, 0x001FFFFFF8000000, 0x001FFFFFFC000000,
            0x001FFFFFFE000000, 0x001FFFFFFF000000, 0x001FFFFFFF800000,
            0x001FFFFFFFC00000, 0x001FFFFFFFE00000, 0x001FFFFFFFF00000,
            0x001FFFFFFFF80000, 0x001FFFFFFFFC0000, 0x001FFFFFFFFE0000,
            0x001FFFFFFFFF0000, 0x001FFFFFFFFF8000, 0x001FFFFFFFFFC000,
            0x001FFFFFFFFFE000, 0x001FFFFFFFFFF000, 0x001FFFFFFFFFF800,
            0x001FFFFFFFFFFC00, 0x001FFFFFFFFFFE00, 0x001FFFFFFFFFFF00,
            0x001FFFFFFFFFFF80, 0x001FFFFFFFFFFFC0, 0x001FFFFFFFFFFFE0,
            0x001FFFFFFFFFFFF0, 0x001FFFFFFFFFFFF8, 0x001FFFFFFFFFFFFC,
            0x001FFFFFFFFFFFFE, 0x001FFFFFFFFFFFFF
        ];

        normalize_to_f64!(self, DENORMAL, MAX, MASKS)
    }

    /// Create extended float from 8-bit unsigned integer.
    #[inline]
    pub fn from_u8(i: u8) -> FloatType {
        from_int!(i)
    }

    /// Create extended float from 16-bit unsigned integer.
    #[inline]
    pub fn from_u16(i: u16) -> FloatType {
        from_int!(i)
    }

    /// Create extended float from 32-bit unsigned integer.
    #[inline]
    pub fn from_u32(i: u32) -> FloatType {
        from_int!(i)
    }

    /// Create extended float from 64-bit unsigned integer.
    #[inline]
    pub fn from_u64(i: u64) -> FloatType {
        from_int!(i)
    }

    /// Create extended float from 32-bit float.
    #[inline]
    pub fn from_f32(f: f32) -> FloatType {
        const EXPONENT: u64 = FloatType::F32_EXPONENT_MASK;
        const HIDDEN: u64 = FloatType::F32_HIDDEN_BIT_MASK;
        const FRACTION: u64 = FloatType::F32_FRACTION_MASK;
        const BIAS: i32 = FloatType::F32_EXPONENT_BIAS;
        const SIG_SIZE: i32 = FloatType::F32_SIGNIFICAND_SIZE;

        from_float!(f, EXPONENT, HIDDEN, FRACTION, BIAS, SIG_SIZE)
    }

    /// Create extended float from 64-bit float.
    #[inline]
    pub fn from_f64(f: f64) -> FloatType {
        const EXPONENT: u64 = FloatType::F64_EXPONENT_MASK;
        const HIDDEN: u64 = FloatType::F64_HIDDEN_BIT_MASK;
        const FRACTION: u64 = FloatType::F64_FRACTION_MASK;
        const BIAS: i32 = FloatType::F64_EXPONENT_BIAS;
        const SIG_SIZE: i32 = FloatType::F64_SIGNIFICAND_SIZE;

        from_float!(f, EXPONENT, HIDDEN, FRACTION, BIAS, SIG_SIZE)
    }

    /// Convert to lower-precision 32-bit float.
    #[inline]
    pub fn as_f32(&self) -> f32 {
        const DENORMAL: i32 = FloatType::F32_DENORMAL_EXPONENT;
        const HIDDEN: u64 = FloatType::F32_HIDDEN_BIT_MASK;
        const FRACTION: u64 = FloatType::F32_FRACTION_MASK;
        const BIAS: i32 = FloatType::F32_EXPONENT_BIAS;
        const MAX: i32 = FloatType::F32_MAX_EXPONENT;
        const INF: u32 = FloatType::U32_INFINITY;
        const SIG_SIZE: i32 = FloatType::F32_SIGNIFICAND_SIZE;

        // Create a normalized fraction for export.
        let mut x = *self;
        x.normalize_to_f32();
        as_float!(x, f32, u32, DENORMAL, HIDDEN, FRACTION, BIAS, MAX, INF, SIG_SIZE)
    }

    /// Convert to lower-precision 64-bit float.
    #[inline]
    pub fn as_f64(&self) -> f64 {
        const DENORMAL: i32 = FloatType::F64_DENORMAL_EXPONENT;
        const HIDDEN: u64 = FloatType::F64_HIDDEN_BIT_MASK;
        const FRACTION: u64 = FloatType::F64_FRACTION_MASK;
        const BIAS: i32 = FloatType::F64_EXPONENT_BIAS;
        const MAX: i32 = FloatType::F64_MAX_EXPONENT;
        const INF: u64 = FloatType::U64_INFINITY;
        const SIG_SIZE: i32 = FloatType::F64_SIGNIFICAND_SIZE;

        // Create a normalized fraction for export.
        let mut x = *self;
        x.normalize_to_f64();
        as_float!(x, f64, u64, DENORMAL, HIDDEN, FRACTION, BIAS, MAX, INF, SIG_SIZE)
    }
}

// CACHED POWERS

// FLOATING POINT CONSTANTS
const ONE_LOG_TEN: f64 = 0.30102999566398114;
const NPOWERS: i32 = 87;
const FIRSTPOWER: i32 = -348;       // 10 ^ -348
const STEPPOWERS: i32 = 8;
const EXPMAX: i32 = -32;
const EXPMIN: i32 = -60;

/// Find cached power of 10 from the exponent.
#[inline]
pub(crate) unsafe extern "C" fn cached_power(exp: i32, k: *mut i32)
    -> &'static FloatType
{
    let approx = -((exp + NPOWERS) as f64) * ONE_LOG_TEN;
    let approx = approx as i32;
    let mut idx = ((approx - FIRSTPOWER) / STEPPOWERS) as usize;

    loop {
        let power = POWERS_OF_TEN.get_unchecked(idx);
        let current = exp + power.exp + 64;
        if current < EXPMIN {
            idx += 1;
            continue;
        }

        if current > EXPMAX {
            idx -= 1;
            continue;
        }

        *k = FIRSTPOWER + (idx as i32) * STEPPOWERS;
        return power;
    }
}

/// Cached powers of ten as specified by the Grisu algorithm.
///
/// Cached powers of 10^k, calculated as if by:
/// `ceil((alpha-e+63) * ONE_LOG_TEN);`
pub(crate) const POWERS_OF_TEN: [FloatType; 87] = [
    FloatType { frac: 18054884314459144840, exp: -1220 },
    FloatType { frac: 13451937075301367670, exp: -1193 },
    FloatType { frac: 10022474136428063862, exp: -1166 },
    FloatType { frac: 14934650266808366570, exp: -1140 },
    FloatType { frac: 11127181549972568877, exp: -1113 },
    FloatType { frac: 16580792590934885855, exp: -1087 },
    FloatType { frac: 12353653155963782858, exp: -1060 },
    FloatType { frac: 18408377700990114895, exp: -1034 },
    FloatType { frac: 13715310171984221708, exp: -1007 },
    FloatType { frac: 10218702384817765436, exp: -980 },
    FloatType { frac: 15227053142812498563, exp: -954 },
    FloatType { frac: 11345038669416679861, exp: -927 },
    FloatType { frac: 16905424996341287883, exp: -901 },
    FloatType { frac: 12595523146049147757, exp: -874 },
    FloatType { frac: 9384396036005875287, exp: -847 },
    FloatType { frac: 13983839803942852151, exp: -821 },
    FloatType { frac: 10418772551374772303, exp: -794 },
    FloatType { frac: 15525180923007089351, exp: -768 },
    FloatType { frac: 11567161174868858868, exp: -741 },
    FloatType { frac: 17236413322193710309, exp: -715 },
    FloatType { frac: 12842128665889583758, exp: -688 },
    FloatType { frac: 9568131466127621947, exp: -661 },
    FloatType { frac: 14257626930069360058, exp: -635 },
    FloatType { frac: 10622759856335341974, exp: -608 },
    FloatType { frac: 15829145694278690180, exp: -582 },
    FloatType { frac: 11793632577567316726, exp: -555 },
    FloatType { frac: 17573882009934360870, exp: -529 },
    FloatType { frac: 13093562431584567480, exp: -502 },
    FloatType { frac: 9755464219737475723, exp: -475 },
    FloatType { frac: 14536774485912137811, exp: -449 },
    FloatType { frac: 10830740992659433045, exp: -422 },
    FloatType { frac: 16139061738043178685, exp: -396 },
    FloatType { frac: 12024538023802026127, exp: -369 },
    FloatType { frac: 17917957937422433684, exp: -343 },
    FloatType { frac: 13349918974505688015, exp: -316 },
    FloatType { frac: 9946464728195732843, exp: -289 },
    FloatType { frac: 14821387422376473014, exp: -263 },
    FloatType { frac: 11042794154864902060, exp: -236 },
    FloatType { frac: 16455045573212060422, exp: -210 },
    FloatType { frac: 12259964326927110867, exp: -183 },
    FloatType { frac: 18268770466636286478, exp: -157 },
    FloatType { frac: 13611294676837538539, exp: -130 },
    FloatType { frac: 10141204801825835212, exp: -103 },
    FloatType { frac: 15111572745182864684, exp: -77 },
    FloatType { frac: 11258999068426240000, exp: -50 },
    FloatType { frac: 16777216000000000000, exp: -24 },
    FloatType { frac: 12500000000000000000, exp:  3 },
    FloatType { frac: 9313225746154785156, exp:  30 },
    FloatType { frac: 13877787807814456755, exp: 56 },
    FloatType { frac: 10339757656912845936, exp: 83 },
    FloatType { frac: 15407439555097886824, exp: 109 },
    FloatType { frac: 11479437019748901445, exp: 136 },
    FloatType { frac: 17105694144590052135, exp: 162 },
    FloatType { frac: 12744735289059618216, exp: 189 },
    FloatType { frac: 9495567745759798747, exp: 216 },
    FloatType { frac: 14149498560666738074, exp: 242 },
    FloatType { frac: 10542197943230523224, exp: 269 },
    FloatType { frac: 15709099088952724970, exp: 295 },
    FloatType { frac: 11704190886730495818, exp: 322 },
    FloatType { frac: 17440603504673385349, exp: 348 },
    FloatType { frac: 12994262207056124023, exp: 375 },
    FloatType { frac: 9681479787123295682, exp: 402 },
    FloatType { frac: 14426529090290212157, exp: 428 },
    FloatType { frac: 10748601772107342003, exp: 455 },
    FloatType { frac: 16016664761464807395, exp: 481 },
    FloatType { frac: 11933345169920330789, exp: 508 },
    FloatType { frac: 17782069995880619868, exp: 534 },
    FloatType { frac: 13248674568444952270, exp: 561 },
    FloatType { frac: 9871031767461413346, exp: 588 },
    FloatType { frac: 14708983551653345445, exp: 614 },
    FloatType { frac: 10959046745042015199, exp: 641 },
    FloatType { frac: 16330252207878254650, exp: 667 },
    FloatType { frac: 12166986024289022870, exp: 694 },
    FloatType { frac: 18130221999122236476, exp: 720 },
    FloatType { frac: 13508068024458167312, exp: 747 },
    FloatType { frac: 10064294952495520794, exp: 774 },
    FloatType { frac: 14996968138956309548, exp: 800 },
    FloatType { frac: 11173611982879273257, exp: 827 },
    FloatType { frac: 16649979327439178909, exp: 853 },
    FloatType { frac: 12405201291620119593, exp: 880 },
    FloatType { frac: 9242595204427927429, exp: 907 },
    FloatType { frac: 13772540099066387757, exp: 933 },
    FloatType { frac: 10261342003245940623, exp: 960 },
    FloatType { frac: 15290591125556738113, exp: 986 },
    FloatType { frac: 11392378155556871081, exp: 1013 },
    FloatType { frac: 16975966327722178521, exp: 1039 },
    FloatType { frac: 12648080533535911531, exp: 1066 }
];

#[cfg(test)]
mod tests {
    use super::*;
    use util::*;

    // Sample of interesting numbers to check during standard test builds.
    const INTEGERS: [u64; 32] = [
        0,                      // 0x0
        1,                      // 0x1
        7,                      // 0x7
        15,                     // 0xF
        112,                    // 0x70
        119,                    // 0x77
        127,                    // 0x7F
        240,                    // 0xF0
        247,                    // 0xF7
        255,                    // 0xFF
        2032,                   // 0x7F0
        2039,                   // 0x7F7
        2047,                   // 0x7FF
        4080,                   // 0xFF0
        4087,                   // 0xFF7
        4095,                   // 0xFFF
        65520,                  // 0xFFF0
        65527,                  // 0xFFF7
        65535,                  // 0xFFFF
        1048560,                // 0xFFFF0
        1048567,                // 0xFFFF7
        1048575,                // 0xFFFFF
        16777200,               // 0xFFFFF0
        16777207,               // 0xFFFFF7
        16777215,               // 0xFFFFFF
        268435440,              // 0xFFFFFF0
        268435447,              // 0xFFFFFF7
        268435455,              // 0xFFFFFFF
        4294967280,             // 0xFFFFFFF0
        4294967287,             // 0xFFFFFFF7
        4294967295,             // 0xFFFFFFFF
        18446744073709551615,   // 0xFFFFFFFFFFFFFFFF
    ];

    // FROM

    #[test]
    fn from_int_test() {
        // 0
        assert_eq!(FloatType::from_u8(0), FloatType {frac: 0, exp: 0});
        assert_eq!(FloatType::from_u16(0), FloatType {frac: 0, exp: 0});
        assert_eq!(FloatType::from_u32(0), FloatType {frac: 0, exp: 0});
        assert_eq!(FloatType::from_u64(0), FloatType {frac: 0, exp: 0});

        // 1
        assert_eq!(FloatType::from_u8(1), FloatType {frac: 1, exp: 0});
        assert_eq!(FloatType::from_u16(1), FloatType {frac: 1, exp: 0});
        assert_eq!(FloatType::from_u32(1), FloatType {frac: 1, exp: 0});
        assert_eq!(FloatType::from_u64(1), FloatType {frac: 1, exp: 0});

        // (2^8-1) 255
        assert_eq!(FloatType::from_u8(255), FloatType {frac: 255, exp: 0});
        assert_eq!(FloatType::from_u16(255), FloatType {frac: 255, exp: 0});
        assert_eq!(FloatType::from_u32(255), FloatType {frac: 255, exp: 0});
        assert_eq!(FloatType::from_u64(255), FloatType {frac: 255, exp: 0});

        // (2^16-1) 65535
        assert_eq!(FloatType::from_u16(65535), FloatType {frac: 65535, exp: 0});
        assert_eq!(FloatType::from_u32(65535), FloatType {frac: 65535, exp: 0});
        assert_eq!(FloatType::from_u64(65535), FloatType {frac: 65535, exp: 0});

        // (2^32-1) 4294967295
        assert_eq!(FloatType::from_u32(4294967295), FloatType {frac: 4294967295, exp: 0});
        assert_eq!(FloatType::from_u64(4294967295), FloatType {frac: 4294967295, exp: 0});

        // (2^64-1) 18446744073709551615
        assert_eq!(FloatType::from_u64(18446744073709551615), FloatType {frac: 18446744073709551615, exp: 0});
    }

    #[test]
    fn from_f32_test() {
        assert_eq!(FloatType::from_f32(0.), FloatType {frac: 0, exp: -149});
        assert_eq!(FloatType::from_f32(-0.), FloatType {frac: 0, exp: -149});

        assert_eq!(FloatType::from_f32(1e-45), FloatType {frac: 1, exp: -149});
        assert_eq!(FloatType::from_f32(1e-40), FloatType {frac: 71362, exp: -149});
        assert_eq!(FloatType::from_f32(2e-40), FloatType {frac: 142725, exp: -149});
        assert_eq!(FloatType::from_f32(1e-20), FloatType {frac: 12379400, exp: -90});
        assert_eq!(FloatType::from_f32(2e-20), FloatType {frac: 12379400, exp: -89});
        assert_eq!(FloatType::from_f32(1.0), FloatType {frac: 8388608, exp: -23});
        assert_eq!(FloatType::from_f32(2.0), FloatType {frac: 8388608, exp: -22});
        assert_eq!(FloatType::from_f32(1e20), FloatType {frac: 11368684, exp: 43});
        assert_eq!(FloatType::from_f32(2e20), FloatType {frac: 11368684, exp: 44});
        assert_eq!(FloatType::from_f32(3.402823e38), FloatType {frac: 16777213, exp: 104});
    }

    #[test]
    fn from_f64_test() {
        assert_eq!(FloatType::from_f64(0.), FloatType {frac: 0, exp: -1074});
        assert_eq!(FloatType::from_f64(-0.), FloatType {frac: 0, exp: -1074});
        assert_eq!(FloatType::from_f64(5e-324), FloatType {frac: 1, exp: -1074});
        assert_eq!(FloatType::from_f64(1e-250), FloatType {frac: 6448907850777164, exp: -883});
        assert_eq!(FloatType::from_f64(1e-150), FloatType {frac: 7371020360979573, exp: -551});
        assert_eq!(FloatType::from_f64(1e-45), FloatType {frac: 6427752177035961, exp: -202});
        assert_eq!(FloatType::from_f64(1e-40), FloatType {frac: 4903985730770844, exp: -185});
        assert_eq!(FloatType::from_f64(2e-40), FloatType {frac: 4903985730770844, exp: -184});
        assert_eq!(FloatType::from_f64(1e-20), FloatType {frac: 6646139978924579, exp: -119});
        assert_eq!(FloatType::from_f64(2e-20), FloatType {frac: 6646139978924579, exp: -118});
        assert_eq!(FloatType::from_f64(1.0), FloatType {frac: 4503599627370496, exp: -52});
        assert_eq!(FloatType::from_f64(2.0), FloatType {frac: 4503599627370496, exp: -51});
        assert_eq!(FloatType::from_f64(1e20), FloatType {frac: 6103515625000000, exp: 14});
        assert_eq!(FloatType::from_f64(2e20), FloatType {frac: 6103515625000000, exp: 15});
        assert_eq!(FloatType::from_f64(1e40), FloatType {frac: 8271806125530277, exp: 80});
        assert_eq!(FloatType::from_f64(2e40), FloatType {frac: 8271806125530277, exp: 81});
        assert_eq!(FloatType::from_f64(1e150), FloatType {frac: 5503284107318959, exp: 446});
        assert_eq!(FloatType::from_f64(1e250), FloatType {frac: 6290184345309700, exp: 778});
        assert_eq!(FloatType::from_f64(1.7976931348623157e308), FloatType {frac: 9007199254740991, exp: 971});
    }

    fn assert_normalized_eq(mut x: FloatType, mut y: FloatType) {
        x.normalize();
        y.normalize();
        assert_eq!(x, y);
    }

    #[test]
    fn from_float() {
        let values: [f32; 26] = [
            1e-40,
            2e-40,
            1e-35,
            2e-35,
            1e-30,
            2e-30,
            1e-25,
            2e-25,
            1e-20,
            2e-20,
            1e-15,
            2e-15,
            1e-10,
            2e-10,
            1e-5,
            2e-5,
            1.0,
            2.0,
            1e5,
            2e5,
            1e10,
            2e10,
            1e15,
            2e15,
            1e20,
            2e20,
        ];
        for value in values.iter() {
            assert_normalized_eq(FloatType::from_f32(*value), FloatType::from_f64(*value as f64));
        }
    }

    // NORMALIZE

    #[test]
    fn normalize_test() {
        // F32
        // min value
        let mut x = FloatType {frac: 1, exp: -149};
        x.normalize();
        assert_eq!(x, FloatType {frac: 9223372036854775808, exp: -212});

        // 1.0e-40
        let mut x = FloatType {frac: 71362, exp: -149};
        x.normalize();
        assert_eq!(x, FloatType {frac: 10043308644012916736, exp: -196});

        // 1.0e-20
        let mut x = FloatType {frac: 12379400, exp: -90};
        x.normalize();
        assert_eq!(x, FloatType {frac: 13611294244890214400, exp: -130});

        // 1.0
        let mut x = FloatType {frac: 8388608, exp: -23};
        x.normalize();
        assert_eq!(x, FloatType {frac: 9223372036854775808, exp: -63});

        // 1e20
        let mut x = FloatType {frac: 11368684, exp: 43};
        x.normalize();
        assert_eq!(x, FloatType {frac: 12500000250510966784, exp: 3});

        // max value
        let mut x = FloatType {frac: 16777213, exp: 104};
        x.normalize();
        assert_eq!(x, FloatType {frac: 18446740775174668288, exp: 64});

        // F64

        // min value
        let mut x = FloatType {frac: 1, exp: -1074};
        x.normalize();
        assert_eq!(x, FloatType {frac: 9223372036854775808, exp: -1137});

        // 1.0e-250
        let mut x = FloatType {frac: 6448907850777164, exp: -883};
        x.normalize();
        assert_eq!(x, FloatType {frac: 13207363278391631872, exp: -894});

        // 1.0e-150
        let mut x = FloatType {frac: 7371020360979573, exp: -551};
        x.normalize();
        assert_eq!(x, FloatType {frac: 15095849699286165504, exp: -562});

        // 1.0e-45
        let mut x = FloatType {frac: 6427752177035961, exp: -202};
        x.normalize();
        assert_eq!(x, FloatType {frac: 13164036458569648128, exp: -213});

        // 1.0e-40
        let mut x = FloatType {frac: 4903985730770844, exp: -185};
        x.normalize();
        assert_eq!(x, FloatType {frac: 10043362776618688512, exp: -196});

        // 1.0e-20
        let mut x = FloatType {frac: 6646139978924579, exp: -119};
        x.normalize();
        assert_eq!(x, FloatType {frac: 13611294676837537792, exp: -130});

        // 1.0
        let mut x = FloatType {frac: 4503599627370496, exp: -52};
        x.normalize();
        assert_eq!(x, FloatType {frac: 9223372036854775808, exp: -63});

        // 1e20
        let mut x = FloatType {frac: 6103515625000000, exp: 14};
        x.normalize();
        assert_eq!(x, FloatType {frac: 12500000000000000000, exp: 3});

        // 1e40
        let mut x = FloatType {frac: 8271806125530277, exp: 80};
        x.normalize();
        assert_eq!(x, FloatType {frac: 16940658945086007296, exp: 69});

        // 1e150
        let mut x = FloatType {frac: 5503284107318959, exp: 446};
        x.normalize();
        assert_eq!(x, FloatType {frac: 11270725851789228032, exp: 435});

        // 1e250
        let mut x = FloatType {frac: 6290184345309700, exp: 778};
        x.normalize();
        assert_eq!(x, FloatType {frac: 12882297539194265600, exp: 767});

        // max value
        let mut x = FloatType {frac: 9007199254740991, exp: 971};
        x.normalize();
        assert_eq!(x, FloatType {frac: 18446744073709549568, exp: 960});
    }

    #[test]
    fn normalize_to_f32_test() {
        // This is lossy, so some of these values are **slightly** rounded.

        // underflow
        let mut x = FloatType {frac: 9223372036854775808, exp: -213};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 8388608, exp: -173});

        // min value
        let mut x = FloatType {frac: 9223372036854775808, exp: -212};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 1, exp: -149});

        // 1.0e-40
        let mut x = FloatType {frac: 10043308644012916736, exp: -196};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 71362, exp: -149});

        // 1.0e-20
        let mut x = FloatType {frac: 13611294244890214400, exp: -130};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 12379400, exp: -90});

        // 1.0
        let mut x = FloatType {frac: 9223372036854775808, exp: -63};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 8388608, exp: -23});

        // 1e20
        let mut x = FloatType {frac: 12500000250510966784, exp: 3};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 11368684, exp: 43});

        // max value
        let mut x = FloatType {frac: 18446740775174668288, exp: 64};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 16777213, exp: 104});

        // overflow
        let mut x = FloatType {frac: 18446740775174668288, exp: 65};
        x.normalize_to_f32();
        assert_eq!(x, FloatType {frac: 16777213, exp: 105});
    }

    #[test]
    fn normalize_to_f64_test() {
        // This is lossy, so some of these values are **slightly** rounded.

        // underflow
        let mut x = FloatType {frac: 9223372036854775808, exp: -1138};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 4503599627370496, exp: -1127});

        // min value
        let mut x = FloatType {frac: 9223372036854775808, exp: -1137};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 1, exp: -1074});

        // 1.0e-250
        let mut x = FloatType {frac: 13207363278391631872, exp: -894};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 6448907850777164, exp: -883});

        // 1.0e-150
        let mut x = FloatType {frac: 15095849699286165504, exp: -562};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 7371020360979573, exp: -551});

        // 1.0e-45
        let mut x = FloatType {frac: 13164036458569648128, exp: -213};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 6427752177035961, exp: -202});

        // 1.0e-40
        let mut x = FloatType {frac: 10043362776618688512, exp: -196};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 4903985730770844, exp: -185});

        // 1.0e-20
        let mut x = FloatType {frac: 13611294676837537792, exp: -130};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 6646139978924579, exp: -119});

        // 1.0
        let mut x = FloatType {frac: 9223372036854775808, exp: -63};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 4503599627370496, exp: -52});

        // 1e20
        let mut x = FloatType {frac: 12500000000000000000, exp: 3};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 6103515625000000, exp: 14});

        // 1e40
        let mut x = FloatType {frac: 16940658945086007296, exp: 69};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 8271806125530277, exp: 80});

        // 1e150
        let mut x = FloatType {frac: 11270725851789228032, exp: 435};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 5503284107318959, exp: 446});

        // 1e250
        let mut x = FloatType {frac: 12882297539194265600, exp: 767};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 6290184345309700, exp: 778});

        // max value
        let mut x = FloatType {frac: 18446744073709549568, exp: 960};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 9007199254740991, exp: 971});

        // overflow
        let mut x = FloatType {frac: 18446744073709549568, exp: 961};
        x.normalize_to_f64();
        assert_eq!(x, FloatType {frac: 9007199254740991, exp: 972});
    }

    // TO

    #[test]
    fn to_f32_test() {
        // underflow
        let x = FloatType {frac: 9223372036854775808, exp: -213};
        assert_eq!(x.as_f32(), 0.0);

        // min value
        let x = FloatType {frac: 9223372036854775808, exp: -212};
        assert_eq!(x.as_f32(), 1e-45);

        // 1.0e-40
        let x = FloatType {frac: 10043308644012916736, exp: -196};
        assert_eq!(x.as_f32(), 1e-40);

        // 1.0e-20
        let x = FloatType {frac: 13611294244890214400, exp: -130};
        assert_eq!(x.as_f32(), 1e-20);

        // 1.0
        let x = FloatType {frac: 9223372036854775808, exp: -63};
        assert_eq!(x.as_f32(), 1.0);

        // 1e20
        let x = FloatType {frac: 12500000250510966784, exp: 3};
        assert_eq!(x.as_f32(), 1e20);

        // max value
        let x = FloatType {frac: 18446740775174668288, exp: 64};
        assert_eq!(x.as_f32(), 3.402823e38);

        // almost max, high exp
        let x = FloatType {frac: 1048575, exp: 108};
        assert_eq!(x.as_f32(), 3.4028204e38);

        // max value + 1
        let x = FloatType {frac: 16777216, exp: 104};
        assert_eq!(x.as_f32(), F32_INFINITY);

        // max value + 1
        let x = FloatType {frac: 1048576, exp: 108};
        assert_eq!(x.as_f32(), F32_INFINITY);

        // 1e40
        let x = FloatType {frac: 16940658945086007296, exp: 69};
        assert_eq!(x.as_f32(), F32_INFINITY);

        // Integers.
        for int in INTEGERS.iter() {
            let fp = FloatType {frac: *int, exp: 0};
            assert_eq!(fp.as_f32(), *int as f32, "{:?} as f32", *int);
        }
    }

    #[test]
    fn to_f64_test() {
        // underflow
        let x = FloatType {frac: 9223372036854775808, exp: -1138};
        assert_relative_eq!(x.as_f64(), 0.0);

        // min value
        let x = FloatType {frac: 9223372036854775808, exp: -1137};
        assert_relative_eq!(x.as_f64(), 5e-324);

        // 1.0e-250
        let x = FloatType {frac: 13207363278391631872, exp: -894};
        assert_relative_eq!(x.as_f64(), 1e-250);

        // 1.0e-150
        let x = FloatType {frac: 15095849699286165504, exp: -562};
        assert_relative_eq!(x.as_f64(), 1e-150);

        // 1.0e-45
        let x = FloatType {frac: 13164036458569648128, exp: -213};
        assert_relative_eq!(x.as_f64(), 1e-45);

        // 1.0e-40
        let x = FloatType {frac: 10043362776618688512, exp: -196};
        assert_relative_eq!(x.as_f64(), 1e-40);

        // 1.0e-20
        let x = FloatType {frac: 13611294676837537792, exp: -130};
        assert_relative_eq!(x.as_f64(), 1e-20);

        // 1.0
        let x = FloatType {frac: 9223372036854775808, exp: -63};
        assert_relative_eq!(x.as_f64(), 1.0);

        // 1e20
        let x = FloatType {frac: 12500000000000000000, exp: 3};
        assert_relative_eq!(x.as_f64(), 1e20);

        // 1e40
        let x = FloatType {frac: 16940658945086007296, exp: 69};
        assert_relative_eq!(x.as_f64(), 1e40);

        // 1e150
        let x = FloatType {frac: 11270725851789228032, exp: 435};
        assert_relative_eq!(x.as_f64(), 1e150);

        // 1e250
        let x = FloatType {frac: 12882297539194265600, exp: 767};
        assert_relative_eq!(x.as_f64(), 1e250);

        // max value
        let x = FloatType {frac: 9007199254740991, exp: 971};
        assert_relative_eq!(x.as_f64(), 1.7976931348623157e308);

        // max value
        let x = FloatType {frac: 18446744073709549568, exp: 960};
        assert_relative_eq!(x.as_f64(), 1.7976931348623157e308);

        // overflow
        let x = FloatType {frac: 9007199254740992, exp: 971};
        assert_relative_eq!(x.as_f64(), F64_INFINITY);

        // overflow
        let x = FloatType {frac: 18446744073709549568, exp: 961};
        assert_relative_eq!(x.as_f64(), F64_INFINITY);

        // Integers.
        for int in INTEGERS.iter() {
            let fp = FloatType {frac: *int, exp: 0};
            assert_eq!(fp.as_f64(), *int as f64, "{:?} as f64", *int);
        }
    }

    #[test]
    #[ignore]
    fn to_f32_full_test() {
        // Use exhaustive search to ensure both lossy and unlossy items are checked.
        // 23-bits of precision, so go from 0-32.
        for int in 0..u32::max_value() {
            let fp = FloatType {frac: int as u64, exp: 0};
            assert_eq!(fp.as_f32(), int as f32, "{:?} as f32", int);
        }
    }

    #[test]
    #[ignore]
    fn to_f64_full_test() {
        // Use exhaustive search to ensure both lossy and unlossy items are checked.
        const U32_MAX: u64 = u32::max_value() as u64;
        const POW2_52: u64 = 4503599627370496;
        const START: u64 = POW2_52 - U32_MAX / 2;
        const END: u64 = START + U32_MAX;
        for int in START..END {
            let fp = FloatType {frac: int, exp: 0};
            assert_eq!(fp.as_f64(), int as f64, "{:?} as f64", int);
        }
    }

    // OPERATIONS

    #[test]
    fn add_test() {
        // Simple
        let x = FloatType::from_u8(1);
        let y = FloatType::from_u8(2);
        let z = unsafe { x.add_unchecked(&y) };
        assert_eq!(z.as_f64(), 3.0);

        // Different exponents
        let x = FloatType::from_f32(1.0);
        let y = FloatType::from_f64(2.0);
        let z = x.add(&y);
        assert_eq!(z.as_f64(), 3.0);

        let x = FloatType::from_u8(1);
        let y = FloatType::from_f64(2.0);
        let z = x.add(&y);
        assert_eq!(z.as_f64(), 3.0);

        // Normally underflowing
        let x = FloatType::from_f32(1e-45);
        let y = FloatType::from_f64(2.0);
        let z = x.add(&y);
        assert_eq!(z.as_f64(), 2.0);

        let x = FloatType::from_f64(2.0);
        let y = FloatType::from_f32(1e-45);
        let z = x.add(&y);
        assert_eq!(z.as_f64(), 2.0);

        // Normally overflowing
        let x = FloatType::from_u64(10000000000000000000);
        let y = FloatType::from_u64(10000000000000000000);
        let z = x.add(&y);
        assert_eq!(z.as_f64(), 2e19);

        // Actually overflowing
        let x = FloatType {frac: u64::max_value(), exp: FloatType::F64_MAX_EXPONENT};
        let y = x;
        let z = x.add(&y);
        assert_eq!(z.as_f64(), F64_INFINITY);
    }

    #[test]
    fn subtract_test() {
        // Simple
        let x = FloatType::from_u8(5);
        let y = FloatType::from_u8(3);
        let z = unsafe { x.sub_unchecked(&y) };
        assert_eq!(z.as_f64(), 2.0);
    }

    #[test]
    fn multiply_test() {
        let x = FloatType::from_u8(1);
        let y = FloatType::from_u64(10000000000000000000);
        let z = FloatType::from_f64(1.7976931348623157e308);

        for i in 2..36 {
            let f = i as f64;
            assert_eq!(x.mul_n(i).as_f64(), f);
            assert_eq!(y.mul_n(i).as_f64(), f * 1e19);
            assert_eq!(z.mul_n(i).as_f64(), F64_INFINITY);

            unsafe {
                assert_eq!(x.mul_n_unchecked(i).as_f64(), f);
            }
        }
    }
}
