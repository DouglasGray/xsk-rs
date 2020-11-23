#!/bin/bash

apt update
apt install -y ethtool
apt install -y emacs
apt install -y build-essential
apt install -y libelf-dev

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
