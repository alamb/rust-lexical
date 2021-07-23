//! Radix-generic, lexical integer-to-string conversion routines.
//!
//! These routines are highly optimized: they unroll 4 loops at a time,
//! using pre-computed base^2 tables.
//!
//! See [Algorithm.md](/Algorithm/md) for a more detailed description of
//! the algorithm choice here.

use crate::lib::ptr;
use crate::table::digit_to_char;
use lexical_util::assert::debug_assert_radix;
use lexical_util::div128::{u128_divisor, u128_divrem};
use lexical_util::num::{as_cast, UnsignedInteger};

// TODO(ahuszagh) Add more documentation...

/// Generic itoa algorithm.
///
/// This algorithm first writers 4, then 2 digits at a time, finally
/// the last 1 or 2 digits, using power reduction to speed up the
/// algorithm a lot.
///
/// # Safety
///
/// This is safe as long as the buffer is large enough to hold `T::MAX`
/// digits in radix `N`.
unsafe fn write_digits<T: UnsignedInteger>(
    mut value: T,
    radix: u32,
    table: &[u8],
    buffer: &mut [u8],
    mut index: usize,
) -> usize {
    debug_assert_radix(radix);

    // Pre-compute our powers of radix.
    let radix = as_cast(radix);
    let radix2 = radix * radix;
    let radix4 = radix2 * radix2;

    // SAFETY: All of these are safe for the buffer writes as long as
    // the buffer is large enough to hold `T::MAX` digits in radix `N`.

    // Decode 4 digits at a time.
    while value >= radix4 {
        let r = value % radix4;
        value /= radix4;
        let r1 = (T::TWO * (r / radix2)).as_usize();
        let r2 = (T::TWO * (r % radix2)).as_usize();

        // SAFETY: This is always safe, since the table is 2*radix^2, and
        // r1 and r2 must be in the range [0, 2*radix^2-1), since the maximum
        // value of r is `radix4-1`, which must have a div and r
        // in the range [0, radix^2-1).
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r2 + 1);
        }
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r2);
        }
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r1 + 1);
        }
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r1);
        }
    }

    // Decode 2 digits at a time.
    while value >= radix2 {
        let r = (T::TWO * (value % radix2)).as_usize();
        value /= radix2;

        // SAFETY: this is always safe, since the table is 2*radix^2, and
        // r must be in the range [0, 2*radix^2-1).
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r + 1);
        }
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r);
        }
    }

    // Decode last 2 digits.
    if value < radix {
        // SAFETY: this is always safe, since value < radix, so it must be < 36.
        // Digit must be < 36.
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = digit_to_char(value.as_usize());
        }
    } else {
        let r = (T::TWO * value).as_usize();
        // SAFETY: this is always safe, since the table is 2*radix^2, and
        // the value must <= radix^2, so rem must be in the range
        // [0, 2*radix^2-1).
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r + 1);
        }
        index -= 1;
        unsafe {
            *buffer.get_unchecked_mut(index) = *table.get_unchecked(r);
        }
    }

    index
}

/// Specialized digits writer for u128, since it writes at least step digits.
///
/// # Safety
///
/// This is safe as long as the buffer is large enough to hold `T::MAX`
/// digits in radix `N`.
unsafe fn write_step_digits<T: UnsignedInteger>(
    value: T,
    radix: u32,
    table: &[u8],
    buffer: &mut [u8],
    index: usize,
    step: usize,
) -> usize {
    let start = index;
    // SAFETY: safe as long as the call to write_step_digits is safe.
    let index = unsafe { write_digits(value, radix, table, buffer, index) };
    // Write the remaining 0 bytes.
    // SAFETY: this is always safe as long as end is less than the buffer length.
    let end = start.saturating_sub(step);
    unsafe {
        ptr::write_bytes(buffer.as_mut_ptr().add(end), b'0', index - end);
    }

    end
}

/// Optimized implementation for radix-N numbers.
///
/// # Safety
///
/// Safe as long as the buffer is large enough to hold as many digits
/// that can be in the largest value of `T`, in radix `N`.
#[inline]
pub unsafe fn algorithm<T>(value: T, radix: u32, table: &[u8], buffer: &mut [u8]) -> usize
where
    T: UnsignedInteger,
{
    // This is so that radix^4 does not overflow, since 36^4 overflows a u16.
    debug_assert!(T::BITS >= 32, "Must have at least 32 bits in the input.");
    debug_assert_radix(radix);

    // SAFETY: Both forms of unchecked indexing cannot overflow.
    // The table always has 2*radix^2 elements, so it must be a legal index.
    // The buffer is ensured to have at least `FORMATTED_SIZE` or
    // `FORMATTED_SIZE_DECIMAL` characters, which is the maximum number of
    // digits an integer of that size may write.
    unsafe { write_digits(value, radix, table, buffer, buffer.len()) }
}

/// Optimized implementation for radix-N 128-bit numbers.
///
/// # Safety
///
/// Safe as long as the buffer is large enough to hold as many digits
/// that can be in the largest value of `T`, in radix `N`.
#[inline]
pub unsafe fn algorithm_u128(value: u128, radix: u32, table: &[u8], buffer: &mut [u8]) -> usize {
    debug_assert_radix(radix);

    // Quick approximations to make the algorithm **a lot** faster.
    // If the value can be represented in a 64-bit integer, we can
    // do this as a native integer.
    if value <= u64::MAX as u128 {
        return unsafe { algorithm(value as u64, radix, table, buffer) };
    }

    // SAFETY: Both forms of unchecked indexing cannot overflow.
    // The table always has 2*radix^2 elements, so it must be a legal index.
    // The buffer is ensured to have at least `FORMATTED_SIZE` or
    // `FORMATTED_SIZE_DECIMAL` characters, which is the maximum number of
    // digits an integer of that size may write.

    // Use power-reduction to minimize the number of operations.
    // Idea taken from "3 Optimization Tips for C++".
    // Need to keep the steps, cause the lower values may
    // have internal 0s.
    let (divisor, step, d_ctlz) = u128_divisor(radix);

    // Decode 4-digits at a time.
    // To deal with internal 0 values or values with internal 0 digits set,
    // we store the starting index, and if not all digits are written,
    // we just skip down `digits` digits for the next value.
    let (value, low) = u128_divrem(value, divisor, d_ctlz);
    let mut index = buffer.len();
    unsafe {
        index = write_step_digits(low, radix, table, buffer, index, step);
    }
    if value <= u64::MAX as u128 {
        return unsafe { write_digits(value as u64, radix, table, buffer, index) };
    }

    // Value has to be greater than 1.8e38
    let (value, mid) = u128_divrem(value, divisor, d_ctlz);
    unsafe {
        index = write_step_digits(mid, radix, table, buffer, index, step);
    }
    if index != 0 {
        index = unsafe { write_digits(value as u64, radix, table, buffer, index) };
    }

    index
}
