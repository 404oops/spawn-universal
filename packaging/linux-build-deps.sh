#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The system packages a Linux build needs, in one place so the workflows and
# anyone building by hand cannot disagree about them.
#
#   packaging/linux-build-deps.sh
#
# Debian and Ubuntu. Other distributions ship the same libraries under their
# own names; the comments say what each one is for.

set -euo pipefail

sudo apt-get update
sudo apt-get install -y \
    pkg-config make cmake clang mold libstdc++-12-dev \
    libudev-dev \
    libasound2-dev \
    libwayland-dev \
    libx11-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libvulkan-dev \
    libfontconfig1-dev libfreetype-dev \
    libssl-dev libzstd-dev libgit2-dev

# What each group is for:
#   udev          reading the keyboard through hidraw
#   asound        gpui's audio, which it links whether or not this app uses it
#   wayland, x11  the two display backends gpui builds on Linux
#   vulkan        gpui's renderer
#   fontconfig    font-kit, which finds and loads system fonts; without its
#     freetype    development package the build fails on a missing
#                 `fontconfig.pc` before compiling any of this project
