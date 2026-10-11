#!/bin/sh
# Development only: cargo-about 0.9.2 must be installed explicitly.
set -eu
cargo about generate --locked -o THIRD_PARTY_NOTICES.txt about.hbs
# Normalize upstream line endings/whitespace without changing license words.
perl -pi -e 's/\r$//; s/[ \t]+$//' THIRD_PARTY_NOTICES.txt
