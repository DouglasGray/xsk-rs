#!/bin/bash

find ./target/debug/deps/ -maxdepth 1 -perm -111 -type f -regextype egrep -regex "(.*tests.*|.*xsk_rs.*)" -exec {} \;
