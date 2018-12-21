//! Adaptation of the V8 ftoa algorithm with a custom radix.
//!
//! This algorithm is adapted from the V8 codebase,
//! and may be found [here](https://github.com/v8/v8).

use itoa;
use util::*;

// FTOA BASEN
// ----------

/// Calculate the naive exponent from a minimal value.
///
/// Don't export this for float, since it's specialized for radix.
#[inline]
pub(crate) fn naive_exponent(d: f64, radix: u32) -> i32
{
    // floor returns the minimal value, which is our
    // desired exponent
    // ln(1.1e-5) -> -4.95 -> -5
    // ln(1.1e5) -> -5.04 -> 5
    (d.ln() / (radix as f64).ln()).floor() as i32
}

/// Naive algorithm for converting a floating point to a custom radix.
///
/// `d` must be non-special (NaN or infinite), non-negative,
/// and non-zero.
///
/// Adapted from the V8 implementation.
fn ftoa_naive<'a>(value: f64, radix: u32, bytes: &'a mut [u8])
    -> &'a mut [u8]
{
    debug_assert_radix!(radix);

    // Assert no special cases remain, no non-zero values,
    // and no negative numbers.
    debug_assert!(!value.is_special());
    debug_assert!(value != 0.0);
    debug_assert!(value > 0.0);

    // Store the first digit and up to `BUFFER_SIZE - 20` digits
    // that occur from left-to-right in the decimal representation.
    // For example, for the number 123.45, store the first digit `1`
    // and `2345` as the remaining values. Then, decide on-the-fly
    // if we need scientific or regular formatting.
    //
    //   BUFFER_SIZE
    // - 1      # first digit
    // - 1      # period
    // - 1      # +/- sign
    // - 2      # e and +/- sign
    // - 9      # max exp is 308, in radix2 is 9
    // - 1      # null terminator
    // = 15 characters of formatting required
    // Just pad it a bit, we don't want memory corruption.
    const MAX_NONDIGIT_LENGTH: usize = 25;
    const MAX_DIGIT_LENGTH: usize = BUFFER_SIZE - MAX_NONDIGIT_LENGTH;

    // Temporary buffer for the result. We start with the decimal point in the
    // middle and write to the left for the integer part and to the right for the
    // fractional part. 1024 characters for the exponent and 52 for the mantissa
    // either way, with additional space for sign, decimal point and string
    // termination should be sufficient.
    const SIZE: usize = 2200;
    let mut buffer: [u8; SIZE] = explicit_uninitialized();
    //let buffer = buffer.as_mut_ptr();
    let initial_position: usize = SIZE / 2;
    let mut integer_cursor = initial_position;
    let mut fraction_cursor = initial_position;
    let base = radix as f64;

    // Split the value into an integer part and a fractional part.
    let mut integer = value.floor();
    let mut fraction = value - integer;

    // We only compute fractional digits up to the input double's precision.
    let mut delta = 0.5 * (value.next_positive() - value);
    delta = 0.0.next_positive().max_finite(delta);
    debug_assert!(delta > 0.0);

    if fraction > delta {
        loop {
            // Shift up by one digit.
            fraction *= base;
            delta *= base;
            // Write digit.
            let digit = fraction as i32;
            buffer[fraction_cursor] = digit_to_char(digit);
            fraction_cursor += 1;
            // Calculate remainder.
            fraction -= digit as f64;
            // Round to even.
            if fraction > 0.5 || (fraction == 0.5 && (digit & 1) != 0) {
                if fraction + delta > 1.0 {
                    // We need to back trace already written digits in case of carry-over.
                    loop {
                        fraction_cursor -= 1;
                        if fraction_cursor == initial_position-1 {
                            // Carry over to the integer part.
                            integer += 1.0;
                            break;
                        }
                        // Reconstruct digit.
                        let c = buffer[fraction_cursor];
                        let digit = char_to_digit(c) as i32;
                        if digit <= radix as i32 {
                            let idx = (digit + 1) as usize;
                            buffer[fraction_cursor] = digit_to_char(idx);
                            fraction_cursor += 1;
                            break;
                        }
                    }
                    break;
                }
            }

            if delta >= fraction {
                break;
            }
        }
    }

    // Compute integer digits. Fill unrepresented digits with zero.
    while (integer / base).exponent() > 0 {
        integer /= base;
        integer_cursor -= 1;
        buffer[integer_cursor] = b'0';
    }

    loop {
        let remainder = integer % base;
        integer_cursor -= 1;
        let idx = remainder as usize;
        buffer[integer_cursor] = digit_to_char(idx);
        integer = (integer - remainder) / base;

        if integer <= 0.0 {
            break;
        }
    };

    if value <= 1e-5 || value >= 1e9 {
        // write scientific notation with negative exponent
        let exponent = naive_exponent(value, radix);

        // Non-exponent portion.
        // 1.   Get as many digits as possible, up to `MAX_DIGIT_LENGTH+1`
        //      (since we are ignoring the digit for the first digit),
        //      or the number of written digits
        let start: usize;
        let end: usize;
        if value <= 1e-5 {
            start = ((initial_position as i32) - exponent - 1) as usize;
            end = fraction_cursor.min(start + MAX_DIGIT_LENGTH + 1);
        } else {
            start = integer_cursor;
            end = fraction_cursor.min(start + MAX_DIGIT_LENGTH + 1);
        }
        let buffer = &buffer[start..end];

        // 2.   Remove any trailing 0s in the selected range.
        let buffer = rtrim_char_slice(buffer, b'0').0;

        // 3.   Write the fraction component
        bytes[0] = buffer[0];
        bytes[1] = b'.';
        let bytes = copy_to_dst(&mut bytes[2..], &buffer[1..]);

        // write the exponent component
        bytes[0] = exponent_notation_char(radix);
        // Handle negative exponents.
        let exp: u32;
        if exponent < 0 {
            bytes[1] = b'-';
            exp = exponent.wrapping_neg() as u32;
            itoa::forward(exp, radix, &mut bytes[2..])
        } else {
            exp = exponent as u32;
            itoa::forward(exp, radix, &mut bytes[1..])
        }
    } else {
        // get component lengths
        let integer_length = initial_position - integer_cursor;
        let fraction_length = (fraction_cursor - initial_position).min(MAX_DIGIT_LENGTH - integer_length);

        // write integer component
        let bytes = copy_to_dst(bytes, &buffer[integer_cursor..integer_cursor+integer_length]);

        // write fraction component
        if fraction_length > 0 {
            // fraction exists, write it
            bytes[0] = b'.';
            let src = &buffer[initial_position..initial_position+fraction_length];
            copy_to_dst(&mut bytes[1..], src)
        } else {
            // no fraction, write decimal place
            copy_to_dst(bytes, ".0")
        }
    }
}

// F32

/// Forward to double_radix.
///
/// `f` must be non-special (NaN or infinite), non-negative,
/// and non-zero.
#[inline(always)]
pub(crate) fn float_radix<'a>(f: f32, radix: u32, bytes: &'a mut [u8])
    -> &'a mut [u8]
{
    double_radix(f as f64, radix, bytes)
}

// F64

/// Algorithm for non-decimal string representations.
///
/// `d` must be non-special (NaN or infinite), non-negative,
/// and non-zero.
#[inline(always)]
pub(crate) fn double_radix<'a>(value: f64, radix: u32, bytes: &'a mut [u8])
    -> &'a mut [u8]
{
    ftoa_naive(value, radix, bytes)
}
