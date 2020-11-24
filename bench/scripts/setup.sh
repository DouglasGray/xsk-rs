#!/bin/bash

source ./vars.sh

ethtool -N $RX_DEV flow-type ether dst $RX_MAC src $TX_MAC action $RX_Q
