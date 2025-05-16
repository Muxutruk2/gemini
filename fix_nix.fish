#!/usr/bin/env fish

set -x PKG_CONFIG_PATH (nix eval --raw nixpkgs#openssl.dev)/lib/pkgconfig/
