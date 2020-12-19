#!/bin/bash

yum -y update
yum -y groupinstall "Development Tools"
yum -y install elfutils-libelf-devel
yum -y install emacs
yum -y install epel-release
yum -y install make libconfuse-devel libnl3-devel libnl3-devel ncurses-devel

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

# Install bmon
git clone https://github.com/tgraf/bmon.git
cd bmon
./autogen.sh
./configure
make
make install
