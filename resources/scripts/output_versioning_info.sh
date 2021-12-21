#!/usr/bin/env bash

function build_release_name() {
    gh_release_name="Safe Network v$sn_version/"
    gh_release_name="${gh_release_name}Safe API v$sn_api_version/"
    gh_release_name="${gh_release_name}Safe CLI v$sn_cli_version"
}

function build_release_tag_name() {
    gh_release_tag_name="v$sn_version-"
    gh_release_tag_name="v$sn_api_version-"
    gh_release_tag_name="v$sn_cli_version"
}

function output_version_info() {
    echo "::set-output name=sn_version::$sn_version"
    echo "::set-output name=sn_api_version::$sn_api_version"
    echo "::set-output name=sn_cli_version::$sn_cli_version"
    echo "::set-output name=gh_release_name::$gh_release_name"
    echo "::set-output name=gh_release_tag_name::$gh_release_tag_name"
}

gh_release_name=""
gh_release_tag_name=""
commit_message=$(git log --oneline --pretty=format:%s | head -n 1)
sn_version=$(grep "^version" < sn/Cargo.toml | head -n 1 | awk '{ print $3 }' | sed 's/\"//g')
sn_api_version=$(grep "^version" < sn_api/Cargo.toml | head -n 1 | awk '{ print $3 }' | sed 's/\"//g')
sn_cli_version=$(grep "^version" < sn_cli/Cargo.toml | head -n 1 | awk '{ print $3 }' | sed 's/\"//g')
build_release_name
build_release_tag_name
output_version_info
