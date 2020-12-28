#!/bin/bash

apt update
apt install -y ethtool
apt install -y emacs
apt install -y git
apt install -y build-essential
apt install -y libelf-dev

# Install rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Download the repo
git clone https://github.com/DouglasGray/xsk-rs.git

# Go to the benchmark crate and compile
cd xsk-rs/bench
cargo build --release

# Mark scripts as execute
chmod +x scripts/*.
