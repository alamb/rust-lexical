#!/bin/bash
# shellcheck disable=SC2086,SC2236
# Run main test suite.

set -ex

# Change to our project home.
script_dir=$(dirname "${BASH_SOURCE[0]}")
script_home=$(realpath "${script_dir}")
home=$(dirname "${script_home}")
version="${CARGO_VERSION}"
cd "${home}"

# Print our cargo version, for debugging.
cargo ${version} --version

# Ensure we have all our benchmark data files
git submodule update --init lexical-benchmark/data

# Force default tests to disable default feature on NO_STD.
if [ ! -z $NO_STD ]; then
    DEFAULT_FEATURES="--no-default-features"
    REQUIRED_FEATURES="parse-floats,parse-integers,write-floats,write-integers"
    DOCTESTS="--tests"
fi

# Test various feature combinations.
FEATURES=(
    "compact"
    "format"
    "power-of-two"
    "radix"
    "compact,format"
    "compact,radix"
    "format,power-of-two"
    "format,radix"
)

check_error() {
    local feature=$1
    if 2>/dev/null cargo ${version} check --no-default-features --features="${feature}" ; then
        >&2 echo "The feature ${feature} did not error..."
        exit 1
    fi
}

# Don't build the target, but ensure the syntax is correct.
check() {
    if [ ! -z $NO_FEATURES ]; then
        return
    fi

    # Need to test a few permutations just to ensure everything compiles.
    for features in "${FEATURES[@]}"; do
        check_features="$DEFAULT_FEATURES --features=$REQUIRED_FEATURES,$features"
        cargo ${version} check --tests $check_features
    done

    # Check each of our sub-crates compiles.
    cd lexical-parse-float
    cargo ${version} check --tests

    cd ../lexical-parse-integer
    cargo ${version} check --tests

    cd ../lexical-write-float
    cargo ${version} check --tests

    cd ../lexical-write-integer
    cargo ${version} check --tests

    # ensure our partial features aren't allowed, as are unsupported features
    cd ../lexical-core
    partial=(parse write floats integers)
    for feature in "${partial[@]}"; do
        check_error "${feature}"
    done

    cd ../lexical
    for feature in "${partial[@]}"; do
        check_error "${feature}"
    done

    cd ..
}

# Build target.
build() {
    build_features="$DEFAULT_FEATURES --features=$REQUIRED_FEATURES"
    cargo ${version} build $build_features
    cargo ${version} build $build_features --release
}

# Test target.
test() {
    if [ ! -z $DISABLE_TESTS ]; then
        return
    fi
    if [ ! -z $NO_STD ]; then
        return
    fi

    # Default tests.
    test_features="$DEFAULT_FEATURES --features=$REQUIRED_FEATURES"
    cargo ${version} test $test_features $DOCTESTS
    cargo ${version} test $test_features $DOCTESTS --release
    cargo ${version} test --features=radix,format,compact $DOCTESTS --release
    # NOTE: This tests a regressions, related to #96.
    cargo ${version} test --features=format $DOCTESTS

    # Ensure we test radix without the compact feature
    # See #169
    cargo ${version} test --features=radix,format --release

    # this fixes an issue where the lexical and lexical-core tests weren't being run
    cd lexical-core
    cargo ${version} test $test_features,format
    cargo ${version} test $test_features,radix
    cargo ${version} test $test_features,format,radix
    cd ..

    # this fixes an issue where the lexical and lexical-core tests weren't being run
    cd lexical
    cargo ${version} test $test_features,format
    cargo ${version} test $test_features,radix
    cargo ${version} test $test_features,format,radix
    cd ..
}

# Dry-run bench target
bench() {
    if [ ! -z $DISABLE_TESTS ]; then
        return
    fi
    if [ ! -z $DISABLE_BENCHES ]; then
        return
    fi
    if [ ! -z $NO_STD ]; then
        return
    fi
    if [ ! -z $NO_FEATURES ]; then
        # Benches are extremely slow, so disable them unless features are enabled.
        return
    fi

    cd lexical-benchmark
    bench_features="$DEFAULT_FEATURES --features=$REQUIRED_FEATURES"
    cargo ${version} test $bench_features --bench '*'
    cd ..
}

main() {
    check
    build
    test
    bench

    if [ ! -z $NIGHTLY ]; then
        scripts/check.sh
        RUSTFLAGS="--deny warnings" cargo +nightly build --features=lint
    fi
}

main
