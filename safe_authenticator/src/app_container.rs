// Copyright 2017 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement.  This, along with the Licenses can be
// found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

//! Routines to handle an apps dedicated containers

use {AuthError, AuthFuture};
use access_container;
use futures::Future;
use routing::{Action, EntryActions, PermissionSet, User};
use rust_sodium::crypto::sign;
use safe_core::{Client, DIR_TAG, FutureExt, MDataInfo, nfs};

/// Checks if an app's dedicated container is available and stored in the access container.
/// If no previously created container has been found, then it will be created.
pub fn fetch(
    client: &Client<()>,
    app_id: &str,
    app_sign_pk: sign::PublicKey,
) -> Box<AuthFuture<MDataInfo>> {
    let c2 = client.clone();
    let c3 = client.clone();
    let app_cont_name = app_container_name(app_id);

    access_container::authenticator_entry(client)
        .and_then(move |(ac_entry_version, mut ac_entries)| {
            match ac_entries.remove(&app_cont_name) {
                Some(mdata_info) => {
                    // Reuse the already existing app container and update
                    // permissions for it
                    let ps = PermissionSet::new()
                        .allow(Action::Insert)
                        .allow(Action::Update)
                        .allow(Action::Delete)
                        .allow(Action::ManagePermissions);

                    c2.get_mdata_version(mdata_info.name, mdata_info.type_tag)
                        .and_then(move |version| {
                            c2.set_mdata_user_permissions(
                                mdata_info.name,
                                mdata_info.type_tag,
                                User::Key(app_sign_pk),
                                ps,
                                version + 1,
                            ).map(move |_| mdata_info)
                        })
                        .map_err(From::from)
                        .into_box()
                }
                None => {
                    // If the container is not found, create it
                    create(&c2, app_sign_pk)
                        .and_then(move |md_info| {
                            let _ = ac_entries.insert(app_cont_name, md_info.clone());

                            access_container::put_authenticator_entry(
                                &c3,
                                &ac_entries,
                                ac_entry_version + 1,
                            ).map(move |()| md_info)
                        })
                        .into_box()
                }
            }
        })
        .into_box()
}

/// Removes an app's dedicated container if it's available and stored in the user's root dir.
/// Returns `true` if it was removed successfully and `false` if it wasn't found in the parent dir.
pub fn remove(client: Client<()>, app_id: &str) -> Box<AuthFuture<bool>> {
    let c2 = client.clone();
    let app_cont_name = app_container_name(app_id);

    access_container::authenticator_entry(&client)
        .and_then(move |(ac_entry_version, mut ac_entries)| {
            match ac_entries.remove(&app_cont_name) {
                None => {
                    // App container doesn't exist
                    ok!(false)
                }
                Some(mdata_info) => {
                    let c3 = c2.clone();

                    c2.list_mdata_entries(mdata_info.name, mdata_info.type_tag)
                        .and_then(move |entries| {
                            // Remove all entries in MData
                            let actions = entries.iter().fold(EntryActions::new(), |actions,
                             (entry_name, val)| {
                                actions.del(entry_name.clone(), val.entry_version + 1)
                            });

                            c3.mutate_mdata_entries(
                                mdata_info.name,
                                mdata_info.type_tag,
                                actions.into(),
                            )
                        })
                        .map_err(From::from)
                        .and_then(move |_| {
                            // Remove MDataInfo from the access container
                            access_container::put_authenticator_entry(
                                &client,
                                &ac_entries,
                                ac_entry_version + 1,
                            )

                            // TODO(nbaksalyar): when MData deletion is implemented properly,
                            // also delete the actual MutableData related to app
                        })
                        .map_err(From::from)
                        .map(move |_| true)
                        .into_box()
                }
            }
        })
        .into_box()
}

/// Creates a new app's dedicated container
fn create(client: &Client<()>, app_sign_pk: sign::PublicKey) -> Box<AuthFuture<MDataInfo>> {
    let dir = fry!(MDataInfo::random_private(DIR_TAG).map_err(AuthError::from));
    nfs::create_dir(
        client,
        &dir,
        btree_map![],
        btree_map![User::Key(app_sign_pk) => PermissionSet::new()
                .allow(Action::Insert)
                .allow(Action::Update)
                .allow(Action::Delete)
                .allow(Action::ManagePermissions)],
    ).map(move |()| dir).map_err(From::from)
        .into_box()
}

#[inline]
fn app_container_name(app_id: &str) -> String {
    format!("apps/{}", app_id)
}
