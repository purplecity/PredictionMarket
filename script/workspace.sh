#!/bin/bash
#cargo ws init . --resolver=2
cargo workspaces create --name "$1" --edition 2024 --"$2" "$1"
#升级依赖 cargo install cargo-edit
#检查依赖是否升级 cargo install -f cargo-upgrades
# 升级全部依赖 cargo upgrade --incompatible