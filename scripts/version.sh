#!/bin/sh
# Prints the version from Cargo.toml.
sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "$0")/../Cargo.toml" | head -1
