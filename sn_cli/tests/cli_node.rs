// Copyright 2022 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use assert_cmd::Command;
use assert_fs::prelude::*;
use color_eyre::Result;
use predicates::prelude::*;
use sn_cmd_test_utilities::util::get_sn_node_latest_released_version;

#[cfg(not(target_os = "windows"))]
pub(crate) const SN_NODE_BIN_NAME: &str = "sn_node";

#[cfg(target_os = "windows")]
pub(crate) const SN_NODE_BIN_NAME: &str = "sn_node.exe";

#[test]
#[ignore = "this test gets subject to rate limiting so even with retries it's susceptible to failure"]
fn node_install_should_install_the_latest_version() -> Result<()> {
    let temp_dir = assert_fs::TempDir::new()?;
    let safe_dir = temp_dir.child(".safe");
    safe_dir.create_dir_all()?;
    let node_bin_path = safe_dir.child(format!("node/{}", SN_NODE_BIN_NAME));
    let latest_version = get_sn_node_latest_released_version()?;

    let mut cmd = Command::cargo_bin("safe")?;
    cmd.env("SN_CLI_CONFIG_PATH", safe_dir.path())
        .arg("node")
        .arg("install")
        .assert()
        .success()
        .stdout(predicate::str::is_match(format!(
            "Downloading sn_node version:.*{}",
            latest_version
        ))?);
    node_bin_path.assert(predicate::path::is_file());
    Ok(())
}

#[test]
#[ignore = "reenable when aws setup once more"]
fn node_install_should_install_a_specific_version() -> Result<()> {
    let temp_dir = assert_fs::TempDir::new()?;
    let safe_dir = temp_dir.child(".safe");
    safe_dir.create_dir_all()?;
    let node_bin_path = safe_dir.child(format!("node/{}", SN_NODE_BIN_NAME));
    let version = "0.51.6";

    let mut cmd = Command::cargo_bin("safe")?;
    cmd.env("SN_CLI_CONFIG_PATH", safe_dir.path())
        .arg("node")
        .arg("install")
        .arg("--version")
        .arg(version)
        .assert()
        .success()
        .stdout(predicate::str::is_match(format!(
            "Downloading sn_node version:.*{}",
            version
        ))?);
    node_bin_path.assert(predicate::path::is_file());
    Ok(())
}

#[test]
#[ignore = "reenable when aws setup once more"]
fn node_install_should_install_to_a_specific_location() -> Result<()> {
    let temp_dir = assert_fs::TempDir::new()?;
    let safe_dir = temp_dir.child(".safe");
    safe_dir.create_dir_all()?;
    let node_dir_path = safe_dir.child("node");
    let node_bin_path = node_dir_path.child(SN_NODE_BIN_NAME);
    let version = "0.51.6";

    let mut cmd = Command::cargo_bin("safe")?;
    cmd.arg("node")
        .arg("install")
        .arg("--node-path")
        .arg(&node_dir_path.path().display().to_string())
        .arg("--version")
        .arg(version)
        .assert()
        .success()
        .stdout(predicate::str::is_match(format!(
            "Downloading sn_node version:.*{}",
            version
        ))?);
    node_bin_path.assert(predicate::path::is_file());
    Ok(())
}
