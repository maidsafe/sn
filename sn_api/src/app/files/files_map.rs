// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use super::{
    file_system::{normalise_path_separator, upload_file_to_net},
    metadata::FileMeta,
    ProcessedFiles, RealPath,
};
use crate::{app::consts::*, Error, Result, Safe};
use log::{debug, info};
use std::{collections::BTreeMap, fs, path::Path};

// To use for mapping files names (with path in a flattened hierarchy) to FileInfos
pub type FilesMap = BTreeMap<String, FileInfo>;

// Each FileInfo contains file metadata and the link to the file's Blob XOR-URL
pub type FileInfo = BTreeMap<String, String>;

// A trait to get an key attr and return an API Result
pub trait GetAttr {
    fn getattr(&self, key: &str) -> Result<&str>;
}

impl GetAttr for FileInfo {
    // Makes it more readable to conditionally get an attribute from a FileInfo
    // because we can call it in API funcs like fileitem.getattr("key")?;
    fn getattr(&self, key: &str) -> Result<&str> {
        match self.get(key) {
            Some(v) => Ok(v),
            None => Err(Error::EntryNotFound(format!("key not found: {}", key))),
        }
    }
}

// Helper function to add or update a FileInfo in a FilesMap
#[allow(clippy::too_many_arguments)]
pub(crate) async fn add_or_update_file_item(
    safe: &mut Safe,
    file_name: &str,
    file_name_for_map: &str,
    file_path: &Path,
    file_meta: &FileMeta,
    file_link: Option<&str>,
    name_exists: bool,
    dry_run: bool,
    files_map: &mut FilesMap,
    processed_files: &mut ProcessedFiles,
) -> bool {
    // We need to add a new FileInfo, let's generate the FileInfo first
    match gen_new_file_item(safe, file_path, file_meta, file_link, dry_run).await {
        Ok(new_file_item) => {
            let content_added_sign = if name_exists {
                CONTENT_UPDATED_SIGN.to_string()
            } else {
                CONTENT_ADDED_SIGN.to_string()
            };

            debug!("New FileInfo item: {:?}", new_file_item);
            debug!("New FileInfo item inserted as {:?}", file_name);
            files_map.insert(file_name_for_map.to_string(), new_file_item.clone());

            processed_files.insert(
                file_name.to_string(),
                (
                    content_added_sign,
                    // note: files have link property, dirs and symlinks do not
                    new_file_item
                        .get(PREDICATE_LINK)
                        .unwrap_or(&String::default())
                        .to_string(),
                ),
            );

            true
        }
        Err(err) => {
            processed_files.insert(
                file_name.to_string(),
                (CONTENT_ERROR_SIGN.to_string(), format!("<{}>", err)),
            );
            info!("Skipping file \"{}\": {:?}", file_link.unwrap_or(""), err);

            false
        }
    }
}

// Generate a FileInfo for a file which can then be added to a FilesMap
async fn gen_new_file_item(
    safe: &mut Safe,
    file_path: &Path,
    file_meta: &FileMeta,
    link: Option<&str>, // must be symlink target or None if FileMeta::is_symlink() is true.
    dry_run: bool,
) -> Result<FileInfo> {
    let mut file_item = file_meta.to_file_item();
    if file_meta.is_file() {
        let xorurl = match link {
            None => upload_file_to_net(safe, file_path, dry_run).await?,
            Some(link) => link.to_string(),
        };
        file_item.insert(PREDICATE_LINK.to_string(), xorurl);
    } else if file_meta.is_symlink() {
        // get metadata, with any symlinks resolved.
        let result = fs::metadata(&file_path);
        let symlink_target_type = match result {
            Ok(meta) => {
                if meta.is_dir() {
                    "dir"
                } else {
                    "file"
                }
            }
            Err(_) => "unknown", // this occurs for a broken link.  on windows, this would be fixed by: https://github.com/rust-lang/rust/pull/47956
                                 // on unix, there is no way to know if broken link points to file or dir, though we could guess, based on if it has an extension or not.
        };
        let target_path = match link {
            Some(target) => target.to_string(),
            None => {
                let target_path = fs::read_link(&file_path).map_err(|e| {
                    Error::FileSystemError(format!(
                        "Unable to read link: {}.  {:#?}",
                        file_path.display(),
                        e
                    ))
                })?;
                normalise_path_separator(&target_path.display().to_string())
            }
        };
        file_item.insert("symlink_target".to_string(), target_path);
        // This is a hint for windows-platform clients to be able to call
        //   symlink_dir() or symlink_file().  on unix, there's no need.
        file_item.insert(
            "symlink_target_type".to_string(),
            symlink_target_type.to_string(),
        );
    }

    Ok(file_item)
}

/// Returns a new files_map at the given path if the given path is a dir.
pub(crate) fn file_map_for_path(files_map: FilesMap, path: &str) -> Result<FilesMap> {
    let realpath = files_map.realpath(path)?;

    // evict symlinks or files
    if let Some(file_info) = files_map.get(&realpath) {
        let file_type = file_info.get("type").ok_or_else(|| {
            Error::ContentError(format!(
                "corrupt FileInfo: missing a \"type\" property at: {}",
                path
            ))
        })?;
        if FileMeta::filetype_is_symlink(file_type) {
            return Err(Error::ContentError(format!(
                "symlink should not be present in resolved real path: {}",
                realpath
            )));
        } else if FileMeta::filetype_is_file(file_type) {
            return Ok(files_map);
        }
        // else must be a directory, managed below
    }

    // chroot
    let chrooted_file_map = filesmap_chroot(&realpath, &files_map)?;
    Ok(chrooted_file_map)
}

/// If a file is found at path, returns its "link" (xorurl to the blob)
/// along with its metadata enriched with the name of the file
/// Else returns (None, None)
pub(crate) fn get_file_link_and_metadata(
    files_map: &FilesMap,
    path: &str,
) -> Result<(Option<String>, Option<FileInfo>)> {
    if path.is_empty() {
        return Ok((None, None));
    }

    let realpath = files_map.realpath(path)?;

    if let Some(file_info) = files_map.get(&realpath) {
        let file_type = file_info.get("type").ok_or_else(|| {
            Error::ContentError(format!(
                "corrupt FileInfo: missing a \"type\" property at: {}",
                path
            ))
        })?;

        if FileMeta::filetype_is_file(file_type) {
            // get link
            let link = file_info.get("link").ok_or_else(|| {
                Error::ContentError(format!(
                    "corrupt FileInfo: missing a \"link\" property at path: {}",
                    path
                ))
            })?;

            // get FileInfo and enrich it with filename
            let mut enriched_file_info = (*file_info).clone();
            if let Some(filename) = Path::new(&path).file_name() {
                if let Some(name) = filename.to_str() {
                    enriched_file_info.insert("name".to_string(), name.to_owned());
                }
            }

            return Ok((Some(link.to_owned()), Some(enriched_file_info)));
        }
    }
    Ok((None, None))
}

fn filesmap_chroot(urlpath: &str, files_map: &FilesMap) -> Result<FilesMap> {
    let mut filtered_filesmap = FilesMap::default();
    let folder_path = if !urlpath.ends_with('/') {
        format!("{}/", urlpath)
    } else {
        urlpath.to_string()
    };
    files_map.iter().for_each(|(filepath, fileitem)| {
        if filepath.starts_with(&folder_path) {
            let mut new_path = filepath.clone();
            new_path.replace_range(..folder_path.len(), "");
            filtered_filesmap.insert(new_path, fileitem.clone());
        }
    });

    if filtered_filesmap.is_empty() {
        Err(Error::ContentError(format!(
            "no data found for path: {}",
            folder_path
        )))
    } else {
        Ok(filtered_filesmap)
    }
}
