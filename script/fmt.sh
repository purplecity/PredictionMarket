#! /bin/bash

cargo clippy --fix --allow-dirty --allow-staged
rustup run nightly cargo fmt
