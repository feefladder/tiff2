#!/bin/bash

rustc core.rs \
  --target wasm32-unknown-unknown \
  -C link-arg=--import-memory \
  -O \
  -o core.wasm

rustc decoder.rs \
  --target wasm32-unknown-unknown \
  -C link-arg=--import-memory \
  -O \
  -o decoder.wasm

python3 -m http.server
