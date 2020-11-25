#!/bin/bash

yum -y update
rpm --import https://www.elrepo.org/RPM-GPG-KEY-elrepo.org
dnf -y install https://www.elrepo.org/elrepo-release-8.0-2.el8.elrepo.noarch.rpm
dnf --enablerepo=elrepo-kernel install kernel-ml
