#!/bin/bash

# The intention of this is to exit with a non-zero status code if any of
# the tests fail, but to allow all of them the opportunity to run, since
# they're all independent of one another.
exit=0

export RUST_BACKTRACE=full

cargo run --package sn_cli --release -- keys create --for-cli || ((exit++))
cargo test --release --test cli_node || ((exit++))
cargo test --release --test cli_cat -- --test-threads=1 || ((exit++))
cargo test --release --test cli_dog -- --test-threads=1 || ((exit++))
cargo test --release --test cli_files -- --test-threads=1 || ((exit++))
cargo test --release --test cli_files_get -- --test-threads=1 || ((exit++))
cargo test --release --test cli_keys || ((exit++))
cargo test --release --test cli_nrs -- --test-threads=1 || ((exit++))

exit $exit
