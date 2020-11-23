#!/bin/bash

source ./vars.sh

../../target/release/recv --if-name=$RX_DEV --if-queue=$RX_Q
