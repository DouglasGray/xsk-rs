#!/bin/bash

source ./vars.sh

../../target/release/send \
    --if-name=$TX_DEV \
    --if-queue=$TX_Q \
    --src-mac=$TX_MAC \
    --dst-mac=$RX_MAC \
    --src-ip=$TX_IP \
    --dst-ip=$RX_IP
