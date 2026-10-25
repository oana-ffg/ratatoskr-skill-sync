//! Unix replacement policy: preserve numeric protections; refuse unrepresented ACLs.
#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Protection {
    mode: u32,
    uid: u32,
    gid: u32,
}

pub(crate) fn capture(path: &Path) -> Result<Option<Protection>> {
    crate::config::no_symlinks(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Check existing ancestors before creating directories or temporary files. An
        // inherited allow ACE can bypass 0600; an inherited deny ACE must not be lost.
        for ancestor in path.ancestors().skip(1) {
            if ancestor.try_exists()? {
                check_acl(ancestor, true)?;
            }
        }
        if !path.try_exists()? {
            return Ok(None);
        }
        check_acl(path, false)?;
        let metadata = fs::metadata(path)?;
        ensure!(
            metadata.is_file(),
            "Expected a regular file: {}",
            path.display()
        );
        let protection = Protection {
            mode: metadata.mode() & 0o7777,
            uid: metadata.uid(),
            gid: metadata.gid(),
        };
        protection.validate()?;
        protection.check_identity(path)?;
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt as MacMetadataExt;
            ensure!(
                metadata.st_flags() & (0x2 | 0x4 | 0x20000 | 0x40000) == 0,
                "Immutable/append-only file flags are unsupported: {}",
                path.display()
            );
        }
        Ok(Some(protection))
    }
    #[cfg(not(unix))]
    {
        Ok(None)
    }
}

pub(crate) fn private_temporary(path: &Path) -> Result<()> {
    if let Some(protection) = capture(path)? {
        ensure!(
            protection.mode & 0o077 == 0,
            "Filesystem did not create an owner-only temporary file; refusing to write content"
        );
    }
    Ok(())
}

impl Protection {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.mode & !0o777 == 0,
            "Unsupported special Unix mode bits"
        );
        Ok(())
    }

    pub(crate) fn for_restore(
        path: &Path,
        saved: Option<&Self>,
        executable: bool,
    ) -> Result<Option<Self>> {
        let current = capture(path)?;
        let selected = saved.cloned().or_else(|| {
            current.map(|mut protection| {
                // Historical payloads have unknown protections. The current
                // file's public mode may itself be damage from an old release.
                protection.mode &= 0o700;
                protection
            })
        });
        if let Some(protection) = &selected {
            protection.restore_check(path, Some(executable))?;
        }
        Ok(selected)
    }

    pub(crate) fn restore_check(&self, path: &Path, executable: Option<bool>) -> Result<()> {
        self.validate()?;
        #[cfg(unix)]
        {
            self.check_identity(path)?;
            self.replacement(capture(path)?.as_ref(), executable)
                .with_context(|| format!("Cannot safely restore {}", path.display()))?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = executable;
            anyhow::bail!(
                "Cannot restore Unix protection metadata on this platform: {}",
                path.display()
            );
        }
    }

    #[cfg(unix)]
    fn check_identity(&self, path: &Path) -> Result<()> {
        // SAFETY: identity queries have no pointer inputs. getgroups is given
        // exactly the writable capacity obtained by its size-only query.
        let uid = unsafe { libc::geteuid() };
        ensure!(
            self.uid == uid,
            "Cannot safely replace a file owned by another user"
        );
        let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
        ensure!(count >= 0, "Cannot inspect supplementary groups");
        let mut groups = vec![0; count as usize];
        let actual = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
        ensure!(actual >= 0, "Cannot inspect supplementary groups");
        groups.truncate(actual as usize);
        ensure!(
            self.gid == unsafe { libc::getegid() }
                || groups.contains(&self.gid)
                || self.gid == creation_group(path)?,
            "Cannot preserve a file group outside this user's groups"
        );
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn apply(
        current: Option<&Self>,
        saved: Option<&Self>,
        file: &fs::File,
        executable: Option<bool>,
    ) -> Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, fchown};
        let metadata = file.metadata()?;
        let selected = saved
            .or(current)
            .cloned()
            .unwrap_or(Self {
                mode: 0o600,
                uid: metadata.uid(),
                gid: metadata.gid(),
            })
            .replacement(current, executable)?;
        // chown may clear special bits. Never publish until both ownership and
        // permissions have been set and read back successfully.
        if metadata.uid() != selected.uid || metadata.gid() != selected.gid {
            fchown(file, Some(selected.uid), Some(selected.gid))
                .context("Cannot preserve file owner/group; refusing replacement")?;
        }
        file.set_permissions(fs::Permissions::from_mode(selected.mode))?;
        let actual = file.metadata()?;
        ensure!(
            actual.uid() == selected.uid
                && actual.gid() == selected.gid
                && actual.mode() & 0o7777 == selected.mode,
            "Filesystem did not preserve requested Unix protections"
        );
        Ok(())
    }

    #[cfg(unix)]
    fn replacement(&self, current: Option<&Self>, executable: Option<bool>) -> Result<Self> {
        let mut selected = self.clone();
        selected.validate()?;
        if let Some(current) = current {
            ensure!(
                selected.uid == current.uid && selected.gid == current.gid,
                "Owner/group changed since backup; refusing restore"
            );
            // Preserve per-class tightening, including execute-only restrictions.
            // The requested executable boolean below can still restore a deliberate
            // executable/non-executable transition without granting read access.
            selected.mode &= current.mode;
        }
        if let Some(executable) = executable {
            if !executable {
                selected.mode &= !0o111;
            } else if selected.mode & 0o111 == 0 {
                // Both files already being executable is not authorization to
                // regenerate bits that their per-class intersection removed.
                ensure!(
                    current.is_none_or(|protection| protection.mode & 0o111 == 0),
                    "No executable permission survives the saved/current restrictions; refusing restore"
                );
                selected.mode |= 0o100 | ((selected.mode & 0o444) >> 2);
            }
        }
        Ok(selected)
    }
}

#[cfg(unix)]
fn creation_group(path: &Path) -> Result<u32> {
    use std::os::unix::fs::MetadataExt;
    let parent = path
        .ancestors()
        .skip(1)
        .find(|parent| parent.is_dir())
        .context("No existing parent directory for replacement")?;
    let metadata = fs::metadata(parent)?;
    // macOS always inherits the parent group. Linux does so for setgid
    // directories; otherwise it uses the process's effective group. Newly
    // created intermediate directories retain those same inheritance rules.
    if cfg!(target_os = "macos") || metadata.mode() & 0o2000 != 0 {
        Ok(metadata.gid())
    } else {
        // SAFETY: getegid has no pointer inputs or failure mode.
        Ok(unsafe { libc::getegid() })
    }
}

#[cfg(target_os = "macos")]
fn check_acl(path: &Path, directory: bool) -> Result<()> {
    use std::{
        ffi::{CString, c_char, c_int, c_void},
        os::unix::ffi::OsStrExt,
        ptr,
    };
    // libSystem's extended ACL API (sys/acl.h). libc exposes no Apple ACL bindings.
    unsafe extern "C" {
        fn acl_get_file(path: *const c_char, kind: c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, index: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_get_flagset_np(entry: *mut c_void, flags: *mut *mut c_void) -> c_int;
        fn acl_get_flag_np(flags: *mut c_void, flag: c_int) -> c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
        fn acl_get_permset_mask_np(entry: *mut c_void, mask: *mut u64) -> c_int;
        fn acl_free(acl: *mut c_void) -> c_int;
    }
    struct Acl(*mut c_void);
    impl Drop for Acl {
        fn drop(&mut self) {
            // SAFETY: this pointer was returned by acl_get_file and is owned here.
            unsafe {
                acl_free(self.0);
            }
        }
    }
    let name = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: name is NUL-terminated; 0x100 is ACL_TYPE_EXTENDED on macOS.
    let raw = unsafe { acl_get_file(name.as_ptr(), 0x100) };
    if raw.is_null() {
        let error = std::io::Error::last_os_error();
        // macOS reports ENOENT for an existing object with no extended ACL.
        if error.kind() == std::io::ErrorKind::NotFound && path.try_exists()? {
            return Ok(());
        }
        return Err(error).with_context(|| format!("Inspect native ACL: {}", path.display()));
    }
    let acl = Acl(raw);
    let mut index = 0; // ACL_FIRST_ENTRY; subsequent calls use ACL_NEXT_ENTRY (-1).
    loop {
        let mut entry = ptr::null_mut();
        // SAFETY: acl is live and entry points to writable pointer storage.
        if unsafe { acl_get_entry(acl.0, index, &mut entry) } != 0 {
            let error = std::io::Error::last_os_error();
            // Apple uses EINVAL to signal the end of this valid ACL's entries.
            if error.raw_os_error() == Some(22) {
                break;
            }
            return Err(error).context("Read native ACL entry");
        }
        ensure!(
            directory,
            "Native file ACLs are unsupported; refusing to replace, back up or delete {}",
            path.display()
        );
        let mut flags = ptr::null_mut();
        let mut tag = 0;
        let mut permissions = 0;
        // SAFETY: entry belongs to the live ACL; all outputs have the ABI's types.
        ensure!(
            unsafe { acl_get_flagset_np(entry, &mut flags) } == 0
                && unsafe { acl_get_tag_type(entry, &mut tag) } == 0
                && unsafe { acl_get_permset_mask_np(entry, &mut permissions) } == 0,
            "Cannot inspect directory ACL: {}",
            path.display()
        );
        for flag in [1 << 5, 1 << 6] {
            // FILE_INHERIT, DIRECTORY_INHERIT
            // SAFETY: flags belongs to the live ACL entry.
            let inherited = unsafe { acl_get_flag_np(flags, flag) };
            ensure!(
                inherited == 0,
                "Inheritable directory ACLs are unsupported; refusing writes under {}",
                path.display()
            );
        }
        // Non-inheritable directory ACLs stay on the directory. Refuse deny
        // add_file/add_subdirectory/delete_child before creating a tempfile that
        // the same ACL could prevent us cleaning up. HOME's deny-delete is safe.
        ensure!(
            tag != 2 || permissions & ((1 << 2) | (1 << 5) | (1 << 6)) == 0,
            "Directory ACL prevents safe replacement/cleanup: {}",
            path.display()
        );
        index = -1;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn check_acl(path: &Path, directory: bool) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, ptr};
    let path_name = CString::new(path.as_os_str().as_bytes())?;
    for attribute in [c"system.posix_acl_access", c"system.posix_acl_default"] {
        if directory != (attribute == c"system.posix_acl_default") {
            continue;
        }
        // SAFETY: both strings are terminated; a null buffer requests size only.
        let size =
            unsafe { libc::getxattr(path_name.as_ptr(), attribute.as_ptr(), ptr::null_mut(), 0) };
        if size < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENODATA | libc::EOPNOTSUPP)) {
                continue;
            }
            return Err(error).context("Inspect POSIX ACL");
        }
        ensure!(
            size == 0,
            "POSIX ACLs are unsupported; refusing writes at {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn check_acl(path: &Path, _directory: bool) -> Result<()> {
    anyhow::bail!(
        "Cannot verify native file protections on this platform: {}",
        path.display()
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn temporary_privacy_is_verified_before_content_can_be_written() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().canonicalize().unwrap();
        private_temporary(&path).unwrap();
        // Model a filesystem that grants group/other access despite the
        // creation request. The guard must refuse before receiving any bytes.
        for mode in [0o640, 0o604] {
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(mode))
                .unwrap();
            assert!(private_temporary(&path).is_err());
            assert_eq!(file.as_file().metadata().unwrap().len(), 0);
        }
    }
}
