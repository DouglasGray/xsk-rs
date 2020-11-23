#!/bin/bash

source ./vars.sh

ip link add dev $TX_DEV type veth peer name $RX_DEV &&
ip link set $TX_DEV up &&
ip link set $RX_DEV up &&
ip link set dev $TX_DEV address $TX_MAC &&
ip link set dev $RX_DEV address $RX_MAC &&
ip addr add $TX_IP dev $TX_DEV &&
ip addr add $RX_IP dev $RX_DEV
