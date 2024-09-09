'''
    step
    ====

    Generate the step size for each type. This is the maximum
    number of digits that can be processed without overflowing
    a type of a given size.
'''

import math

WIDTHS = [8, 16, 32, 64, 128]
# Note that we use the actual max + 1.
# This is because the first digit cannot be the radix,
# for example, a binary integer of 64-digits cannot be 2**64.
UNSIGNED_MAX = [2**i for i in WIDTHS]
SIGNED_MAX = [2**(i - 1) for i in WIDTHS]


def is_pow2(radix):
    '''Determine if the value is an exact power-of-two.'''
    return radix == 2**int(math.log2(radix))


def find_power(max_value, radix):
    '''Find the power of the divisor.'''

    # Normally we'd use a log, but the log can be inaccurate.
    # We use it as a guiding point, but we take the floor - 1.
    power = int(math.floor(math.log(max_value, radix))) - 1
    while radix**power <= max_value:
        power += 1
    power -= 1
    if radix**power < max_value:
        # Not fully divisible
        return (power, power + 1)
    return (power, power)


def print_comment():
    '''Print the auto-generated comment'''

    print('''// AUTO-GENERATED
// These functions were auto-generated by `etc/step.py`.
// Do not edit them unless there is a good reason to.
// Preferably, edit the source code to generate the constants.
//
// NOTE: For the fallthrough value for types (in case of adding short
// or wider type support in the future), use 1 so it doesn't infinitely
// recurse. Under normal circumstances, this will never be called.
''')


def print_power(radix):
    '''Print the minimum and maximum powers.'''

    unsigned = [find_power(i, radix) for i in UNSIGNED_MAX]
    signed = [find_power(i, radix) for i in SIGNED_MAX]

    print('#[inline(always)]')
    print(f'const fn max_step_{radix}(bits: usize, is_signed: bool) -> usize {{')
    print('    match bits {')
    for index in range(len(WIDTHS)):
        print(f'        {WIDTHS[index]} if is_signed => {signed[index][1]},')
        print(f'        {WIDTHS[index]} if !is_signed => {unsigned[index][1]},')
    print('        _ => 1,')
    print('    }')
    print('}')
    print('')

    print('#[inline(always)]')
    print(f'const fn min_step_{radix}(bits: usize, is_signed: bool) -> usize {{')
    print('    match bits {')
    for index in range(len(WIDTHS)):
        print(f'        {WIDTHS[index]} if is_signed => {signed[index][0]},')
        print(f'        {WIDTHS[index]} if !is_signed => {unsigned[index][0]},')
    print('        _ => 1,')
    print('    }')
    print('}')
    print('')


def main():
    '''Generate all the step sizes for given radixes.'''

    print_comment()
    for radix in range(2, 37):
        print_power(radix)


if __name__ == '__main__':
    main()
