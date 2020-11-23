#!/bin/bash

source ./vars.sh

../../target/release/recv -d -w -z --if-name=$RX_DEV --if-queue=$RX_Q
