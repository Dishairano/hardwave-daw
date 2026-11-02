//! The one sign-in every Hardwave program on this computer shares.
//!
//! The Suite, the plug-in windows and the DAW keep a single account token
//! in a single file, `hardwave/auth_token` in the user's data folder:
//! `%APPDATA%\hardwave\auth_token` on Windows,
//! `~/Library/Application Support/hardwave/auth_token` on a Mac and
//! `~/.local/share/hardwave/auth_token` on Linux. The file holds the token
//! and nothing else. Signing in anywhere writes it, so every program is
//! signed in; signing out anywhere deletes it.
//!
//! The token is the key to the account, so the file is readable by this
//! user only, and it is replaced in one step so no program ever reads half
//! of one. Nothing here puts a token in an error message.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Longest token taken from the file or from the site. The site's tokens
/// are a few hundred bytes; a bigger file is not a token.
pub const MAX_TOKEN_LEN: usize = 8 * 1024;

/// Where the shared token lives on this computer.
pub fn path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("hardwave").join("auth_token"))
}

/// The shared token, if this computer is signed in.
pub fn load() -> Option<String> {
    read(&path()?)
}

/// Whether `text` can be a token: the characters a JWT is made of and a
/// sane length. Anything else is never sent anywhere: in a request header
/// a stray line break would be a header of its own.
pub fn looks_like_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_TOKEN_LEN
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'='))
}

/// Read the token file at `path`. An empty file, one too long to be a
/// token, or one holding anything a token cannot hold counts as no token.
pub fn read(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_TOKEN_LEN as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_TOKEN_LEN {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let token = text.trim();
    looks_like_token(token).then(|| token.to_string())
}

/// Write `token` to the file at `path`, readable by this user only.
pub fn write(path: &Path, token: &str) -> Result<(), String> {
    if !looks_like_token(token) {
        return Err("That is not a sign-in this computer can keep.".into());
    }
    // A plug-in reading the file at the very moment it is replaced can
    // make Windows refuse the swap; a moment later it goes through.
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        match write_private(path, token) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(format!(
        "Could not save the sign-in on this computer ({}).",
        last.map(|e| e.kind().to_string()).unwrap_or_default()
    ))
}

/// Delete the file at `path`: signing out. Already gone is fine.
pub fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "Could not remove the sign-in from this computer ({}).",
            e.kind()
        )),
    }
}

/// Write through a temporary file in the same folder, restricted to this
/// user before it takes the old file's place.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the token file has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = write_and_replace(&tmp, path, contents);
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn write_and_replace(tmp: &Path, path: &Path, contents: &str) -> std::io::Result<()> {
    let mut file = open_private(tmp)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    drop(file);
    restrict_to_user(tmp)?;
    std::fs::rename(tmp, path)
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Read and write for the owner only, whatever the umask said.
#[cfg(unix)]
fn restrict_to_user(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Replace the file's access list with one entry: full access for the user
/// this program runs as, nothing inherited from the folder. Other accounts
/// on the computer cannot read the token. The same list the Suite writes.
#[cfg(windows)]
fn restrict_to_user(path: &Path) -> std::io::Result<()> {
    use std::io::Error;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, ERROR_SUCCESS, GENERIC_ALL, HANDLE,
    };
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut process_token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
    // closing, and `process_token` is a valid place for the opened handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut process_token) } == 0 {
        return Err(Error::last_os_error());
    }
    // TOKEN_USER and the SID it points to. A SID is at most 68 bytes, so
    // 256 bytes hold both; u64 elements keep the buffer aligned.
    let mut buf = [0u64; 32];
    let mut len = 0u32;
    // SAFETY: `process_token` is open with TOKEN_QUERY and `buf` is
    // writable for the byte length passed.
    let ok = unsafe {
        GetTokenInformation(
            process_token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            std::mem::size_of_val(&buf) as u32,
            &mut len,
        )
    };
    let query_error = Error::last_os_error();
    // SAFETY: opened above, closed exactly once.
    unsafe { CloseHandle(process_token) };
    if ok == 0 {
        return Err(query_error);
    }
    // SAFETY: the call succeeded, so `buf` starts with an aligned
    // TOKEN_USER whose SID points into `buf`, which outlives `sid`.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };

    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid.cast(),
        },
    };
    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: one complete EXPLICIT_ACCESS_W whose SID is valid for this
    // call, no old list to merge, and `acl` is a valid place for the list.
    let status = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
    if status != ERROR_SUCCESS {
        return Err(Error::from_raw_os_error(status as i32));
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is a NUL-terminated path, `acl` is the list built
    // above, and the null owner, group and audit list leave those alone.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    // SAFETY: SetEntriesInAclW allocated `acl` with LocalAlloc; this is
    // its last use and it is freed exactly once.
    unsafe { LocalFree(acl.cast()) };
    if status != ERROR_SUCCESS {
        return Err(Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn restrict_to_user(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hw-shared-token-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_written_token_reads_back_and_nothing_is_left_beside_it() {
        let dir = folder("roundtrip");
        let file = dir.join("hardwave").join("auth_token");
        write(&file, "aaa.bbb.ccc").expect("written");
        assert_eq!(read(&file).as_deref(), Some("aaa.bbb.ccc"));
        // Written again over the old one, as a renewal does.
        write(&file, "ddd.eee.fff").expect("replaced");
        assert_eq!(read(&file).as_deref(), Some("ddd.eee.fff"));
        let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["auth_token".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn only_this_user_can_read_the_file_even_one_an_older_program_left_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = folder("mode");
        let file = dir.join("auth_token");
        std::fs::create_dir_all(&dir).unwrap();
        // How plug-ins before the sign-in fix left it: readable by all.
        std::fs::write(&file, "old.token.x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        write(&file, "aaa.bbb.ccc").unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_the_other_programs_write_is_read_the_way_they_write_it() {
        let dir = folder("others");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("auth_token");
        // The plug-ins write the bare token; some editors add a line end.
        std::fs::write(&file, "aaa.bbb.ccc\r\n").unwrap();
        assert_eq!(read(&file).as_deref(), Some("aaa.bbb.ccc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_a_token_is_no_sign_in() {
        let dir = folder("junk");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("auth_token");
        for junk in [
            &b""[..],
            b"   \n",
            b"aaa.bbb\r\nX-Evil: 1",
            b"token with spaces",
            b"\xff\xfe",
        ] {
            std::fs::write(&file, junk).unwrap();
            assert_eq!(read(&file), None, "{junk:?}");
        }
        std::fs::write(&file, "a".repeat(MAX_TOKEN_LEN + 1)).unwrap();
        assert_eq!(read(&file), None);
        assert_eq!(read(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_token_that_cannot_be_one_is_never_written() {
        let dir = folder("refuse");
        let file = dir.join("auth_token");
        assert!(write(&file, "").is_err());
        assert!(write(&file, "a\nb").is_err());
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_out_deletes_the_file_and_twice_is_fine() {
        let dir = folder("remove");
        let file = dir.join("auth_token");
        write(&file, "aaa.bbb.ccc").unwrap();
        remove(&file).expect("removed");
        assert!(!file.exists());
        remove(&file).expect("already gone is fine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
