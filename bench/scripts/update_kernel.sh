#!/bin/bash

KTYP=generic

wget -P kernel-5.9-$KTYP https://kernel.ubuntu.com/~kernel-ppa/mainline/v5.9/amd64/linux-headers-5.9.0-050900-"$KTYP"_5.9.0-050900.202010112230_amd64.deb
wget -P kernel-5.9-$KTYP https://kernel.ubuntu.com/~kernel-ppa/mainline/v5.9/amd64/linux-headers-5.9.0-050900_5.9.0-050900.202010112230_all.deb
wget -P kernel-5.9-$KTYP https://kernel.ubuntu.com/~kernel-ppa/mainline/v5.9/amd64/linux-image-unsigned-5.9.0-050900-"$KTYP"_5.9.0-050900.202010112230_amd64.deb
wget -P kernel-5.9-$KTYP https://kernel.ubuntu.com/~kernel-ppa/mainline/v5.9/amd64/linux-modules-5.9.0-050900-"$KTYP"_5.9.0-050900.202010112230_amd64.deb

dpkg -i linux-*.deb 
shutdown -r now
