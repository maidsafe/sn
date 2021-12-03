#!/usr/bin/env bash

echo "Waiting for a healthy network to be detected, as per the 'split_network_assert_health_check' test in sn/src/lib.rs "
until cargo test  -p safe_network --lib --release --features=always-joinable,test-utils -- --ignored split_network_assert_health_check || $( sleep 15 && false ); do :; done
