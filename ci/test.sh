set -e

cargo test --verbose --features libjvm

if [[ ${TRAVIS_RUST_VERSION} == "nightly" ]]; then
	(cd examples/java-lib && ./test.sh)
	(cd generator && cargo test --verbose)
fi
