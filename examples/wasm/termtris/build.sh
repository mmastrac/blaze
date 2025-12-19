#!/bin/bash
set -euxo pipefail

cd "$(dirname "$0")"

rm -rf build 2> /dev/null || true
mkdir -p build
emcc -O3 -sSTRICT --no-entry \
    -o build/termtris.wasm -I termtris/src ./wasm.c termtris/src/*.c
