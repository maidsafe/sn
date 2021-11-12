#!/usr/bin/env bash

version=$1
if [[ -z "$version" ]]; then
    echo "You must supply a version number for sn_cli."
    exit 1
fi

# The single quotes around EOF is to stop attempted variable and backtick expansion.
read -r -d '' release_description << 'EOF'
Command line interface for the Safe Network. Refer to [Safe CLI User Guide](https://github.com/maidsafe/sn_cli/blob/master/README.md) for detailed instructions.

## Safe Network Changelog

__SN_CHANGELOG_TEXT__

## SHA-256 checksums for sn_node binaries:
```
Linux
zip: SN_ZIP_LINUX_CHECKSUM
tar.gz: SN_TAR_LINUX_CHECKSUM

macOS
zip: SN_ZIP_MACOS_CHECKSUM
tar.gz: SN_TAR_MACOS_CHECKSUM

Windows
zip: SN_ZIP_WIN_CHECKSUM
tar.gz: SN_TAR_WIN_CHECKSUM

ARM
zip: SN_ZIP_ARM_CHECKSUM
tar.gz: SN_TAR_ARM_CHECKSUM

ARMv7
zip: SN_ZIP_ARMv7_CHECKSUM
tar.gz: SN_TAR_ARMv7_CHECKSUM

Aarch64
zip: SN_ZIP_AARCH64_CHECKSUM
tar.gz: SN_TAR_AARCH64_CHECKSUM
```
EOF

sn_zip_linux_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-x86_64-unknown-linux-musl.zip" | \
    awk '{ print $1 }')
sn_zip_macos_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-x86_64-apple-darwin.zip" | \
    awk '{ print $1 }')
sn_zip_win_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-x86_64-pc-windows-msvc.zip" | \
    awk '{ print $1 }')
sn_zip_arm_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-arm-unknown-linux-musleabi.zip" | \
    awk '{ print $1 }')
sn_zip_armv7_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-armv7-unknown-linux-musleabihf.zip" | \
    awk '{ print $1 }')
sn_zip_aarch64_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-aarch64-unknown-linux-musl.zip" | \
    awk '{ print $1 }')
sn_tar_linux_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-x86_64-unknown-linux-musl.tar.gz" | \
    awk '{ print $1 }')
sn_tar_macos_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-x86_64-apple-darwin.tar.gz" | \
    awk '{ print $1 }')
sn_tar_win_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-x86_64-pc-windows-msvc.tar.gz" | \
    awk '{ print $1 }')
sn_tar_arm_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-arm-unknown-linux-musleabi.tar.gz" | \
    awk '{ print $1 }')
sn_tar_armv7_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-armv7-unknown-linux-musleabihf.tar.gz" | \
    awk '{ print $1 }')
sn_tar_aarch64_checksum=$(sha256sum \
    "./deploy/prod/sn_node-$version-aarch64-unknown-linux-musl.tar.gz" | \
    awk '{ print $1 }')

release_description=$(sed "s/SN_ZIP_LINUX_CHECKSUM/$sn_zip_linux_checksum/g" <<< "$release_description")
release_description=$(sed "s/SN_ZIP_MACOS_CHECKSUM/$sn_zip_macos_checksum/g" <<< "$release_description")
release_description=$(sed "s/SN_ZIP_WIN_CHECKSUM/$sn_zip_win_checksum/g" <<< "$release_description")
release_description=$(sed "s=SN_ZIP_ARM_CHECKSUM=$sn_zip_arm_checksum=g" <<< "$release_description")
release_description=$(sed "s=SN_ZIP_ARMv7_CHECKSUM=$sn_zip_armv7_checksum=g" <<< "$release_description")
release_description=$(sed "s=SN_ZIP_AARCH64_CHECKSUM=$sn_zip_aarch64_checksum=g" <<< "$release_description")
release_description=$(sed "s/SN_TAR_LINUX_CHECKSUM/$sn_tar_linux_checksum/g" <<< "$release_description")
release_description=$(sed "s/SN_TAR_MACOS_CHECKSUM/$sn_tar_macos_checksum/g" <<< "$release_description")
release_description=$(sed "s/SN_TAR_WIN_CHECKSUM/$sn_tar_win_checksum/g" <<< "$release_description")
release_description=$(sed "s=SN_TAR_ARM_CHECKSUM=$sn_tar_arm_checksum=g" <<< "$release_description")
release_description=$(sed "s=SN_TAR_ARMv7_CHECKSUM=$sn_tar_armv7_checksum=g" <<< "$release_description")
release_description=$(sed "s=SN_TAR_AARCH64_CHECKSUM=$sn_tar_aarch64_checksum=g" <<< "$release_description")

echo "$release_description"
