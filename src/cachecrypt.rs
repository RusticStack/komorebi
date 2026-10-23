//! The encrypted mail cache (opt-in, "Encrypt the mail cache on this
//! computer"; design and measurements in `docs/PERF.md` section 10).
//!
//! A SQLite VFS shim over the default Windows VFS. Every database page and
//! every page in the write-ahead log is sealed with AES-256-GCM through
//! Windows CNG (`bcrypt.dll`): the last [`RESERVE`] bytes of each page,
//! which SQLite is told to leave unused (`SQLITE_FCNTL_RESERVE_BYTES`), hold
//! a random 96-bit nonce and the 128-bit tag. The additional data binds a
//! page to its file kind and position, so pages cannot be swapped. The key
//! is 32 random bytes per cache, kept beside it in `<cache>.key` protected
//! by DPAPI for the current Windows user.
//!
//! What stays in the clear: the 32-byte WAL header and each 24-byte frame
//! header (page numbers, salts, checksums), the `-shm` index (page numbers
//! and hash tables only), and file sizes. Rollback journals, statement
//! journals and temporary files are refused (`temp_store = MEMORY`, WAL
//! only), so no other file can carry page content to disk. The drafts'
//! attachment copies (`<cache>.drafts`) are files outside SQLite and are not
//! covered (TODO.md).

use rusqlite::ffi;
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Page size of an encrypted cache (SQLite's default).
pub const PAGE: usize = 4096;
/// Bytes at the end of every page: nonce then tag.
pub const RESERVE: usize = 28;
const NONCE: usize = 12;
const TAG: usize = 16;
const SEALED: usize = PAGE - RESERVE;
const WAL_HDR: u64 = 32;
const FRAME_HDR: u64 = 24;
const FRAME: u64 = FRAME_HDR + PAGE as u64;
/// The VFS name (`?vfs=` and `open_with_flags_and_vfs`).
pub const VFS: &str = "komorebi-aes";
const PLAIN_MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// What an encrypted cache starts with on disk in place of SQLite's
/// (constant) header string, so it is told apart from a damaged file
/// without its key. The rest of page 1 is sealed.
const ENC_MAGIC: &[u8; 16] = b"Komorebi AES256\0";

/// What went wrong with the cache key. The UI words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The cache is encrypted and its `.key` file is missing.
    Missing,
    /// The key file is there but this Windows user cannot unlock it
    /// (another user, another computer, a reset password), or it is not a
    /// key at all.
    Locked,
    /// Windows refused to make or store a key.
    Failed(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "the encrypted cache's key file is missing"),
            Self::Locked => write!(f, "the encrypted cache's key cannot be unlocked"),
            Self::Failed(e) => write!(f, "cache key: {e}"),
        }
    }
}
impl std::error::Error for KeyError {}

/// `<cache>.key`, the DPAPI-protected key beside the cache.
pub fn key_path(db: &Path) -> PathBuf {
    PathBuf::from(format!("{}.key", db.display()))
}

/// Whether the file at `db` is an encrypted cache: it exists, is not
/// empty and does not start with SQLite's plaintext header. A missing or
/// empty file counts as encrypted when a key file sits beside it (a new
/// encrypted cache).
pub fn is_encrypted(db: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    match std::fs::File::open(db).and_then(|mut f| f.read(&mut head)) {
        Ok(16) => &head == ENC_MAGIC,
        Ok(0) | Err(_) => key_path(db).exists(),
        Ok(_) => false,
    }
}

/// `<cache>.encrypt`: the user's choice in Settings > Privacy, applied at
/// the next start (`1` encrypt, `0` decrypt) and then removed. A file beside
/// the cache, not a setting inside it, so it is read before the cache is
/// opened.
pub fn intent_path(db: &Path) -> PathBuf {
    PathBuf::from(format!("{}.encrypt", db.display()))
}

/// The choice waiting for the next start, if any.
pub fn intent(db: &Path) -> Option<bool> {
    match std::fs::read(intent_path(db)).ok()?.as_slice() {
        b"1" => Some(true),
        b"0" => Some(false),
        _ => None,
    }
}

/// Ask for the cache to be encrypted (or not) at the next start; asking for
/// what it already is withdraws the request.
pub fn set_intent(db: &Path, encrypt: bool) -> std::io::Result<()> {
    if is_encrypted(db) == encrypt {
        match std::fs::remove_file(intent_path(db)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        std::fs::write(intent_path(db), if encrypt { b"1" } else { b"0" })
    }
}

/// At start, before anything opens the cache: carry out a waiting choice
/// with the crash-safe [`migrate`]. The request is removed first, so a
/// conversion that fails is not retried at every start; the cache then
/// stays as it was (the migration leaves a whole cache either way).
pub fn apply_intent(db: &Path) -> anyhow::Result<()> {
    let Some(want) = intent(db) else {
        return Ok(());
    };
    std::fs::remove_file(intent_path(db)).ok();
    if db.exists() && is_encrypted(db) != want {
        migrate(db, want)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ keys

/// A fresh random key.
pub fn new_key() -> Result<[u8; 32], KeyError> {
    let mut k = [0u8; 32];
    cng::random(&mut k).map_err(KeyError::Failed)?;
    Ok(k)
}

/// Write `key` protected for the current user, durably (temp file, flush,
/// rename).
pub fn store_key(db: &Path, key: &[u8; 32]) -> Result<(), KeyError> {
    let blob = dpapi::protect(key).map_err(KeyError::Failed)?;
    let path = key_path(db);
    let tmp = PathBuf::from(format!("{}.new", path.display()));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&blob)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| KeyError::Failed(e.to_string()))
}

/// Read and unlock the key for the cache at `db`.
pub fn load_key(db: &Path) -> Result<[u8; 32], KeyError> {
    let blob = match std::fs::read(key_path(db)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(KeyError::Missing),
        Err(e) => return Err(KeyError::Failed(e.to_string())),
    };
    let plain = dpapi::unprotect(&blob).ok_or(KeyError::Locked)?;
    <[u8; 32]>::try_from(plain.as_slice()).map_err(|_| KeyError::Locked)
}

// ------------------------------------------------------------- opening

static KEYS: Mutex<Vec<[u8; 32]>> = Mutex::new(Vec::new());

/// Register the VFS (once) and the key; returns the `kid` to name it by.
fn register(key: &[u8; 32]) -> Result<usize, KeyError> {
    vfs::install().map_err(KeyError::Failed)?;
    let mut keys = KEYS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(i) = keys.iter().position(|k| k == key) {
        return Ok(i);
    }
    keys.push(*key);
    Ok(keys.len() - 1)
}

/// The URI that opens `db` through the VFS with key number `kid`.
fn uri(db: &Path, kid: usize) -> String {
    uri_query(db, &format!("vfs={VFS}&kid={kid}"))
}

fn uri_query(db: &Path, query: &str) -> String {
    let mut s = String::from("file:");
    let p = db.to_string_lossy().replace('\\', "/");
    if p.starts_with(|c: char| c.is_ascii_alphabetic()) && p.get(1..2) == Some(":") {
        s.push_str("///");
    }
    for c in p.chars() {
        match c {
            '%' => s.push_str("%25"),
            '?' => s.push_str("%3f"),
            '#' => s.push_str("%23"),
            c => s.push(c),
        }
    }
    s.push('?');
    s.push_str(query);
    s
}

/// Copy every table, index, trigger and view of the cache at `src` (a URI
/// naming its VFS) into the empty `dest`, with `user_version`, in one
/// transaction. Rows go in before indexes and triggers.
fn copy_into(dest: &rusqlite::Connection, src: &str) -> anyhow::Result<()> {
    dest.execute("ATTACH ?1 AS old", [src])?;
    let objects = {
        let mut s = dest.prepare(
            "SELECT type, name, sql FROM old.sqlite_master WHERE sql IS NOT NULL
             AND name NOT LIKE 'sqlite_%' ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, rowid",
        )?;
        s.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let version: i64 = dest.query_row("PRAGMA old.user_version", [], |r| r.get(0))?;
    let tx = dest.unchecked_transaction()?;
    for (kind, name, sql) in &objects {
        tx.execute_batch(sql)?;
        if kind == "table" {
            let name = name.replace('"', "\"\"");
            tx.execute_batch(&format!(
                "INSERT INTO main.\"{name}\" SELECT * FROM old.\"{name}\""
            ))?;
        }
    }
    let seq: bool = tx.query_row(
        "SELECT count(*) > 0 FROM old.sqlite_master WHERE name = 'sqlite_sequence'",
        [],
        |r| r.get(0),
    )?;
    if seq {
        tx.execute_batch("DELETE FROM main.sqlite_sequence; INSERT INTO main.sqlite_sequence SELECT * FROM old.sqlite_sequence;")?;
    }
    tx.execute_batch(&format!("PRAGMA main.user_version = {version};"))?;
    tx.commit()?;
    Ok(())
}

/// Open the encrypted cache at `db` with `key`. A new file is created
/// with the reserve bytes the VFS needs; the connection is set to WAL and
/// in-memory temporary storage before anything else runs.
pub fn open(
    db: &Path,
    key: &[u8; 32],
    flags: rusqlite::OpenFlags,
) -> anyhow::Result<rusqlite::Connection> {
    let kid = register(key)?;
    let conn = rusqlite::Connection::open_with_flags(
        uri(db, kid),
        flags | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    set_reserve(&conn, RESERVE as c_int)?;
    conn.execute_batch("PRAGMA temp_store = MEMORY;")?;
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        anyhow::bail!("the encrypted cache needs WAL mode, got {mode}");
    }
    Ok(conn)
}

/// Ask SQLite to keep `n` bytes free at the end of every page of `main`
/// (applies to a new file, and to a `VACUUM INTO` copy).
fn set_reserve(conn: &rusqlite::Connection, n: c_int) -> anyhow::Result<()> {
    let mut v = n;
    // SAFETY: the handle is live for `conn`'s lifetime; the argument is an int.
    let rc = unsafe {
        ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_RESERVE_BYTES,
            (&mut v as *mut c_int).cast(),
        )
    };
    if rc != ffi::SQLITE_OK {
        anyhow::bail!("setting reserve bytes failed ({rc})");
    }
    Ok(())
}

// ----------------------------------------------------------- migration

fn with(db: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", db.display()))
}

/// A crash point for the migration tests: return early after this step.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Never,
    AfterKey,
    AfterCopy,
    AfterAside,
}

/// Count the rows of every table, for the copy's check.
fn counts(conn: &rusqlite::Connection) -> anyhow::Result<Vec<(String, i64)>> {
    let mut s = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names: Vec<String> = s
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    names
        .into_iter()
        .map(|n| {
            let c: i64 =
                conn.query_row(&format!("SELECT count(*) FROM \"{n}\""), [], |r| r.get(0))?;
            Ok((n, c))
        })
        .collect()
}

/// Encrypt (`to_encrypted`) or decrypt the cache at `db` in place,
/// crash-safe: the copy is written to `<db>.migrate`, checked
/// (`quick_check` and every table's row count), then the old file is
/// renamed to `<db>.previous`, the copy renamed into place, and the old
/// file and its `-wal` and `-shm` removed. [`recover`] finishes or undoes
/// an interrupted run. Nothing else may have the cache open.
pub fn migrate(db: &Path, to_encrypted: bool) -> anyhow::Result<()> {
    migrate_until(db, to_encrypted, Stop::Never)
}

#[doc(hidden)]
pub fn migrate_until(db: &Path, to_encrypted: bool, stop: Stop) -> anyhow::Result<()> {
    recover(db)?;
    if is_encrypted(db) == to_encrypted && db.exists() {
        return Ok(());
    }
    let new = with(db, ".migrate");
    let _ = std::fs::remove_file(&new);
    let (src, dest) = if to_encrypted {
        let key = match load_key(db) {
            Ok(k) => k,
            Err(KeyError::Missing) => {
                let k = new_key()?;
                store_key(db, &k)?;
                k
            }
            Err(e) => return Err(e.into()),
        };
        if stop == Stop::AfterKey {
            return Ok(());
        }
        let plain = vfs::default_name().map_err(anyhow::Error::msg)?;
        (
            uri_query(db, &format!("vfs={plain}")),
            open(&new, &key, rusqlite::OpenFlags::default())?,
        )
    } else {
        let key = load_key(db)?;
        let kid = register(&key)?;
        let dest = rusqlite::Connection::open(&new)?;
        dest.execute_batch("PRAGMA journal_mode = WAL;")?;
        (uri_query(db, &format!("vfs={VFS}&kid={kid}")), dest)
    };
    copy_into(&dest, &src)?;
    let src = {
        dest.execute_batch("DETACH old")?;
        if to_encrypted {
            rusqlite::Connection::open(db)?
        } else {
            open(db, &load_key(db)?, rusqlite::OpenFlags::default())?
        }
    };
    drop(dest);
    // Check the copy before the original is touched.
    let check = if to_encrypted {
        let key = load_key(db)?;
        open(&new, &key, rusqlite::OpenFlags::default())?
    } else {
        rusqlite::Connection::open(&new)?
    };
    let ok: String = check.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if ok != "ok" || counts(&check)? != counts(&src)? {
        drop(check);
        let _ = std::fs::remove_file(&new);
        anyhow::bail!("the converted cache did not match; the original is unchanged");
    }
    drop(check);
    drop(src);
    for s in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(with(&new, s));
    }
    if stop == Stop::AfterCopy {
        return Ok(());
    }
    let previous = with(db, ".previous");
    std::fs::rename(db, &previous)?;
    if stop == Stop::AfterAside {
        return Ok(());
    }
    std::fs::rename(&new, db)?;
    finish(db, to_encrypted);
    Ok(())
}

/// Remove what the old form left: its file, `-wal` and `-shm`, and the key
/// after a decryption.
fn finish(db: &Path, encrypted: bool) {
    let previous = with(db, ".previous");
    for s in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(with(&previous, s));
        let _ = std::fs::remove_file(with(db, s));
    }
    let _ = std::fs::remove_file(&previous);
    if !encrypted {
        let _ = std::fs::remove_file(key_path(db));
    }
}

/// Finish or undo a migration a crash interrupted. A checked copy only
/// replaces the cache in the rename step, so: no cache but a `.previous`
/// means the copy (if any) was checked: move it in, else put the original
/// back; a `.migrate` beside a cache is an unfinished copy: delete it.
pub fn recover(db: &Path) -> anyhow::Result<()> {
    let new = with(db, ".migrate");
    let previous = with(db, ".previous");
    if !db.exists() && previous.exists() {
        if new.exists() {
            std::fs::rename(&new, db)?;
            let enc = is_encrypted(db);
            finish(db, enc);
        } else {
            std::fs::rename(&previous, db)?;
        }
    }
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(with(&new, s));
    }
    if db.exists() && previous.exists() {
        let _ = std::fs::remove_file(&previous);
    }
    Ok(())
}

// ----------------------------------------------------------------- CNG

mod cng {
    use super::*;
    use windows_sys::Win32::Security::Cryptography::*;

    struct Alg(BCRYPT_ALG_HANDLE);
    // SAFETY: CNG algorithm handles may be used from any thread.
    unsafe impl Send for Alg {}
    unsafe impl Sync for Alg {}

    fn alg() -> Result<BCRYPT_ALG_HANDLE, String> {
        static ALG: OnceLock<Result<Alg, String>> = OnceLock::new();
        ALG.get_or_init(|| {
            let mut h: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
            // SAFETY: valid out pointer and static strings.
            let st = unsafe {
                BCryptOpenAlgorithmProvider(&mut h, BCRYPT_AES_ALGORITHM, std::ptr::null(), 0)
            };
            if st != 0 {
                return Err(format!("BCryptOpenAlgorithmProvider {st:#x}"));
            }
            let mode = BCRYPT_CHAIN_MODE_GCM;
            // SAFETY: the mode string is a static NUL-terminated wide string.
            let len = unsafe { (0..).take_while(|&i| *mode.add(i) != 0).count() + 1 } * 2;
            // SAFETY: as above; length in bytes includes the terminator.
            let st =
                unsafe { BCryptSetProperty(h, BCRYPT_CHAINING_MODE, mode.cast(), len as u32, 0) };
            if st != 0 {
                return Err(format!("BCryptSetProperty {st:#x}"));
            }
            Ok(Alg(h))
        })
        .as_ref()
        .map(|a| a.0)
        .map_err(Clone::clone)
    }

    pub fn random(buf: &mut [u8]) -> Result<(), String> {
        // SAFETY: valid buffer and length.
        let st = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if st == 0 {
            Ok(())
        } else {
            Err(format!("BCryptGenRandom {st:#x}"))
        }
    }

    /// An AES-256-GCM key handle, used by one file (one connection) only.
    pub struct Key(BCRYPT_KEY_HANDLE);

    impl Key {
        pub fn new(secret: &[u8; 32]) -> Result<Self, String> {
            let mut k: BCRYPT_KEY_HANDLE = std::ptr::null_mut();
            // SAFETY: CNG allocates the key object itself (null buffer).
            let st = unsafe {
                BCryptGenerateSymmetricKey(
                    alg()?,
                    &mut k,
                    std::ptr::null_mut(),
                    0,
                    secret.as_ptr(),
                    32,
                    0,
                )
            };
            if st == 0 {
                Ok(Self(k))
            } else {
                Err(format!("BCryptGenerateSymmetricKey {st:#x}"))
            }
        }

        fn info(
            nonce: &mut [u8; NONCE],
            aad: &mut [u8; 9],
            tag: &mut [u8; TAG],
        ) -> BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
            BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
                cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
                dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
                pbNonce: nonce.as_mut_ptr(),
                cbNonce: NONCE as u32,
                pbAuthData: aad.as_mut_ptr(),
                cbAuthData: 9,
                pbTag: tag.as_mut_ptr(),
                cbTag: TAG as u32,
                pbMacContext: std::ptr::null_mut(),
                cbMacContext: 0,
                cbAAD: 0,
                cbData: 0,
                dwFlags: 0,
            }
        }

        /// Seal `page` in place: the first `SEALED` bytes are encrypted,
        /// the reserve gets nonce and tag.
        pub fn seal(&self, page: &mut [u8], skip: usize, mut aad: [u8; 9]) -> Result<(), String> {
            let (body, tail) = page.split_at_mut(SEALED);
            let body = &mut body[skip..];
            let mut nonce = [0u8; NONCE];
            random(&mut nonce)?;
            let mut tag = [0u8; TAG];
            let info = Self::info(&mut nonce, &mut aad, &mut tag);
            let mut out = 0u32;
            // SAFETY: GCM encrypts in place (input and output may be the same buffer).
            let st = unsafe {
                BCryptEncrypt(
                    self.0,
                    body.as_ptr(),
                    body.len() as u32,
                    (&info as *const BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO).cast(),
                    std::ptr::null_mut(),
                    0,
                    body.as_mut_ptr(),
                    body.len() as u32,
                    &mut out,
                    0,
                )
            };
            if st != 0 || out as usize != body.len() {
                return Err(format!("BCryptEncrypt {st:#x}"));
            }
            tail[..NONCE].copy_from_slice(&nonce);
            tail[NONCE..].copy_from_slice(&tag);
            Ok(())
        }

        /// Open a sealed page in place; false when the tag does not match
        /// (wrong key or changed bytes). The reserve reads back as zeros.
        pub fn open(&self, page: &mut [u8], skip: usize, mut aad: [u8; 9]) -> bool {
            let (body, tail) = page.split_at_mut(SEALED);
            let body = &mut body[skip..];
            let mut nonce = [0u8; NONCE];
            nonce.copy_from_slice(&tail[..NONCE]);
            let mut tag = [0u8; TAG];
            tag.copy_from_slice(&tail[NONCE..]);
            let info = Self::info(&mut nonce, &mut aad, &mut tag);
            let mut out = 0u32;
            // SAFETY: in-place GCM decryption of a buffer we own.
            let st = unsafe {
                BCryptDecrypt(
                    self.0,
                    body.as_ptr(),
                    body.len() as u32,
                    (&info as *const BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO).cast(),
                    std::ptr::null_mut(),
                    0,
                    body.as_mut_ptr(),
                    body.len() as u32,
                    &mut out,
                    0,
                )
            };
            tail.fill(0);
            st == 0 && out as usize == body.len()
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from BCryptGenerateSymmetricKey.
            unsafe { BCryptDestroyKey(self.0) };
        }
    }
}

// --------------------------------------------------------------- DPAPI

mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::*;

    const ENTROPY: &[u8] = b"komorebi mail cache key";

    fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: b.len() as u32,
            pbData: b.as_ptr().cast_mut(),
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI returned a LocalAlloc'd buffer of cbData bytes.
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: freed once, with the allocator DPAPI used.
        unsafe { LocalFree(out.pbData.cast()) };
        v
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: input blobs borrow live slices; out is written by the call.
        let ok = unsafe {
            CryptProtectData(
                &blob(data),
                std::ptr::null(),
                &blob(ENTROPY),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(format!(
                "CryptProtectData failed ({})",
                std::io::Error::last_os_error()
            ));
        }
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: as in `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &blob(data),
                std::ptr::null_mut(),
                &blob(ENTROPY),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        (ok != 0).then(|| take(out))
    }
}

// ----------------------------------------------------------------- VFS

mod vfs {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Db = 1,
        Wal = 2,
        Journal = 3,
    }

    /// Our file: SQLite's header, then the inner (Windows VFS) file.
    #[repr(C)]
    struct File {
        base: ffi::sqlite3_file,
        kind: Kind,
        key: Option<cng::Key>,
        inner: *mut ffi::sqlite3_file,
    }

    struct Ptr(*mut ffi::sqlite3_vfs);
    // SAFETY: the VFS objects live for the process and are read-only after setup.
    unsafe impl Send for Ptr {}
    unsafe impl Sync for Ptr {}

    static INNER: OnceLock<Ptr> = OnceLock::new();
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();

    fn head() -> usize {
        std::mem::size_of::<File>().next_multiple_of(16)
    }

    pub fn install() -> Result<(), String> {
        INSTALLED
            .get_or_init(|| {
                // SAFETY: SQLite returns its default VFS, which lives forever.
                let inner = unsafe { ffi::sqlite3_vfs_find(std::ptr::null()) };
                if inner.is_null() {
                    return Err("no default SQLite VFS".into());
                }
                let _ = INNER.set(Ptr(inner));
                // SAFETY: a plain copy of the default VFS struct.
                let mut v: ffi::sqlite3_vfs = unsafe { std::ptr::read(inner) };
                v.pNext = std::ptr::null_mut();
                // SAFETY: szOsFile of the inner VFS is a small positive int.
                v.szOsFile = (head() as c_int).saturating_add(unsafe { (*inner).szOsFile });
                v.zName = CString::new(VFS).map_err(|e| e.to_string())?.into_raw();
                v.xOpen = Some(x_open);
                let v = Box::into_raw(Box::new(v));
                // SAFETY: the struct is leaked, so it outlives SQLite's use of it.
                let rc = unsafe { ffi::sqlite3_vfs_register(v, 0) };
                if rc == ffi::SQLITE_OK {
                    Ok(())
                } else {
                    Err(format!("sqlite3_vfs_register {rc}"))
                }
            })
            .clone()
    }

    /// The default VFS's name (`win32`), to open a plaintext file by URI.
    pub fn default_name() -> Result<String, String> {
        install()?;
        let v = inner_vfs();
        if v.is_null() {
            return Err("no default SQLite VFS".into());
        }
        // SAFETY: the default VFS's name is a static C string.
        Ok(unsafe { std::ffi::CStr::from_ptr((*v).zName) }
            .to_string_lossy()
            .into_owned())
    }

    fn inner_vfs() -> *mut ffi::sqlite3_vfs {
        INNER.get().map_or(std::ptr::null_mut(), |p| p.0)
    }

    static METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
        // 2: no xFetch/xUnfetch, so SQLite never memory-maps ciphertext.
        iVersion: 2,
        xClose: Some(x_close),
        xRead: Some(x_read),
        xWrite: Some(x_write),
        xTruncate: Some(x_truncate),
        xSync: Some(x_sync),
        xFileSize: Some(x_file_size),
        xLock: Some(x_lock),
        xUnlock: Some(x_unlock),
        xCheckReservedLock: Some(x_check_reserved),
        xFileControl: Some(x_file_control),
        xSectorSize: Some(x_sector_size),
        xDeviceCharacteristics: Some(x_device),
        xShmMap: Some(x_shm_map),
        xShmLock: Some(x_shm_lock),
        xShmBarrier: Some(x_shm_barrier),
        xShmUnmap: Some(x_shm_unmap),
        xFetch: None,
        xUnfetch: None,
    };

    unsafe extern "C" fn x_open(
        _vfs: *mut ffi::sqlite3_vfs,
        name: *const c_char,
        file: *mut ffi::sqlite3_file,
        flags: c_int,
        out_flags: *mut c_int,
    ) -> c_int {
        let f = file.cast::<File>();
        // SAFETY: SQLite gives szOsFile bytes; mark the file unopened first.
        unsafe { (*f).base.pMethods = std::ptr::null() };
        let kind = if flags & ffi::SQLITE_OPEN_MAIN_DB != 0 {
            Kind::Db
        } else if flags & ffi::SQLITE_OPEN_WAL != 0 {
            Kind::Wal
        } else if flags & ffi::SQLITE_OPEN_MAIN_JOURNAL != 0 {
            // Only the switch of a new, empty file to WAL makes one; it
            // holds no page, and a page written to it is refused.
            Kind::Journal
        } else {
            // Statement journals and temporary files could hold plaintext
            // pages (`temp_store = MEMORY` keeps them off disk).
            return ffi::SQLITE_CANTOPEN;
        };
        if name.is_null() {
            return ffi::SQLITE_CANTOPEN;
        }
        // SAFETY: works on main and WAL names alike (SQLite >= 3.31).
        let kid = unsafe { ffi::sqlite3_uri_int64(name, c"kid".as_ptr(), -1) };
        let secret = usize::try_from(kid).ok().and_then(|i| {
            KEYS.lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(i)
                .copied()
        });
        let Some(secret) = secret else {
            return ffi::SQLITE_CANTOPEN;
        };
        let Ok(key) = cng::Key::new(&secret) else {
            return ffi::SQLITE_CANTOPEN;
        };
        let inner_vfs = inner_vfs();
        // SAFETY: the inner file lives right after our header in SQLite's buffer.
        let inner = unsafe { file.cast::<u8>().add(head()).cast::<ffi::sqlite3_file>() };
        // SAFETY: forwarding to the default VFS with its own struct.
        let rc = unsafe {
            match (*inner_vfs).xOpen {
                Some(open) => open(inner_vfs, name, inner, flags, out_flags),
                None => ffi::SQLITE_CANTOPEN,
            }
        };
        if rc != ffi::SQLITE_OK {
            return rc;
        }
        // SAFETY: initialising our own header in place.
        unsafe {
            std::ptr::write(
                f,
                File {
                    base: ffi::sqlite3_file { pMethods: &METHODS },
                    kind,
                    key: Some(key),
                    inner,
                },
            );
        }
        ffi::SQLITE_OK
    }

    fn me<'a>(file: *mut ffi::sqlite3_file) -> &'a mut File {
        // SAFETY: SQLite passes back the file it opened through x_open.
        unsafe { &mut *file.cast::<File>() }
    }

    fn methods(inner: *mut ffi::sqlite3_file) -> &'static ffi::sqlite3_io_methods {
        // SAFETY: an opened inner file has methods for its lifetime.
        unsafe { &*(*inner).pMethods }
    }

    unsafe extern "C" fn x_close(file: *mut ffi::sqlite3_file) -> c_int {
        let f = me(file);
        let rc = methods(f.inner)
            .xClose
            .map_or(ffi::SQLITE_OK, |c| unsafe { c(f.inner) });
        f.key = None;
        rc
    }

    /// A stretch of the file: in the clear, or one sealed page.
    #[derive(Clone, Copy)]
    struct Region {
        start: u64,
        end: u64,
        page: Option<u64>,
    }

    fn region(kind: Kind, pos: u64) -> Region {
        let p = PAGE as u64;
        match kind {
            Kind::Db => {
                let i = pos / p;
                Region {
                    start: i * p,
                    end: i * p + p,
                    page: Some(i),
                }
            }
            Kind::Journal => Region {
                start: pos,
                end: u64::MAX,
                page: None,
            },
            Kind::Wal if pos < WAL_HDR => Region {
                start: 0,
                end: WAL_HDR,
                page: None,
            },
            Kind::Wal => {
                let i = (pos - WAL_HDR) / FRAME;
                let fs = WAL_HDR + i * FRAME;
                if pos < fs + FRAME_HDR {
                    Region {
                        start: fs,
                        end: fs + FRAME_HDR,
                        page: None,
                    }
                } else {
                    Region {
                        start: fs + FRAME_HDR,
                        end: fs + FRAME,
                        page: Some(i),
                    }
                }
            }
        }
    }

    fn aad(kind: Kind, index: u64) -> [u8; 9] {
        let mut a = [0u8; 9];
        a[0] = kind as u8;
        a[1..].copy_from_slice(&index.to_le_bytes());
        a
    }

    /// The span to read or write so that every sealed page is whole.
    fn span(kind: Kind, off: u64, amt: u64) -> (u64, u64) {
        let first = region(kind, off);
        let last = region(kind, off + amt - 1);
        let start = if first.page.is_some() {
            first.start
        } else {
            off
        };
        let end = if last.page.is_some() {
            last.end
        } else {
            off + amt
        };
        (start, end)
    }

    fn pages(kind: Kind, start: u64, end: u64) -> impl Iterator<Item = Region> {
        let mut pos = start;
        std::iter::from_fn(move || {
            if pos >= end {
                return None;
            }
            let r = region(kind, pos);
            pos = r.end;
            Some(r)
        })
        .filter(|r| r.page.is_some())
    }

    /// Read `start..end` from the inner file and open every sealed page.
    /// `lenient`: a page that fails reads as zeros (the WAL recovery read,
    /// whose frame checksum then rejects it, as for a torn frame).
    fn read_open(f: &File, start: u64, end: u64, lenient: bool) -> (c_int, Vec<u8>) {
        let mut buf = vec![0u8; (end - start) as usize];
        let Some(read) = methods(f.inner).xRead else {
            return (ffi::SQLITE_IOERR_READ, buf);
        };
        // SAFETY: buffer of the stated length.
        let rc = unsafe {
            read(
                f.inner,
                buf.as_mut_ptr().cast(),
                buf.len() as c_int,
                start as i64,
            )
        };
        if rc != ffi::SQLITE_OK && rc != ffi::SQLITE_IOERR_SHORT_READ {
            return (rc, buf);
        }
        let Some(key) = &f.key else {
            return (ffi::SQLITE_IOERR_READ, buf);
        };
        // Where the file really ends: only a page wholly at or past it may
        // read as zeros (the inner VFS fills a short read with them). SQLite
        // never leaves a hole inside the file: the cache grows by appending
        // pages in order (a checkpoint writes the WAL's frames in ascending
        // page order, and no size hint or chunk size pre-extends the file
        // here), and the WAL is written frame after frame. So an all-zero
        // page inside the file was zeroed by someone else: tampering, refused
        // like a changed byte (the lenient WAL frame read excepted, whose
        // checksum drops it as a torn frame).
        let Some(size_of) = methods(f.inner).xFileSize else {
            return (ffi::SQLITE_IOERR_FSTAT, buf);
        };
        let mut size: i64 = 0;
        // SAFETY: the inner file's own method, with a valid out pointer.
        let st = unsafe { size_of(f.inner, &mut size) };
        if st != ffi::SQLITE_OK {
            return (st, buf);
        }
        let size = u64::try_from(size).unwrap_or(0);
        for r in pages(f.kind, start, end) {
            let page = &mut buf[(r.start - start) as usize..(r.end - start) as usize];
            if r.start >= size {
                page.fill(0);
                continue; // past the end of the file
            }
            if page.iter().all(|&b| b == 0) && lenient {
                continue; // a WAL frame never written: its checksum drops it
            }
            // Page 1 of the cache keeps a fixed marker in place of
            // SQLite's header string (see `ENC_MAGIC`).
            let first = f.kind == Kind::Db && r.page == Some(0);
            let skip = if first { PLAIN_MAGIC.len() } else { 0 };
            if (!first || page.starts_with(ENC_MAGIC))
                && key.open(page, skip, aad(f.kind, r.page.unwrap_or(0)))
            {
                if first {
                    page[..skip].copy_from_slice(PLAIN_MAGIC);
                }
            } else {
                if lenient {
                    page.fill(0);
                } else {
                    return (ffi::SQLITE_IOERR_DATA, buf);
                }
            }
        }
        (rc, buf)
    }

    unsafe extern "C" fn x_read(
        file: *mut ffi::sqlite3_file,
        out: *mut c_void,
        amt: c_int,
        off: i64,
    ) -> c_int {
        let f = me(file);
        let (Ok(off), Ok(amt)) = (u64::try_from(off), u64::try_from(amt)) else {
            return ffi::SQLITE_IOERR_READ;
        };
        if amt == 0 {
            return ffi::SQLITE_OK;
        }
        let (start, end) = span(f.kind, off, amt);
        let lenient = f.kind == Kind::Wal && amt == FRAME;
        let (rc, buf) = read_open(f, start, end, lenient);
        if rc != ffi::SQLITE_OK && rc != ffi::SQLITE_IOERR_SHORT_READ {
            return rc;
        }
        let at = (off - start) as usize;
        // SAFETY: SQLite's buffer holds `amt` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(buf[at..].as_ptr(), out.cast::<u8>(), amt as usize)
        };
        rc
    }

    unsafe extern "C" fn x_write(
        file: *mut ffi::sqlite3_file,
        data: *const c_void,
        amt: c_int,
        off: i64,
    ) -> c_int {
        let f = me(file);
        let (Ok(off), Ok(amt)) = (u64::try_from(off), u64::try_from(amt)) else {
            return ffi::SQLITE_IOERR_WRITE;
        };
        if amt == 0 {
            return ffi::SQLITE_OK;
        }
        // SAFETY: SQLite's buffer holds `amt` bytes; we never write into it.
        let data = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), amt as usize) };
        if f.kind == Kind::Journal {
            if amt as usize >= SEALED {
                return ffi::SQLITE_IOERR_WRITE;
            }
            let Some(write) = methods(f.inner).xWrite else {
                return ffi::SQLITE_IOERR_WRITE;
            };
            // SAFETY: forwarding SQLite's own buffer.
            return unsafe { write(f.inner, data.as_ptr().cast(), amt as c_int, off as i64) };
        }
        if f.kind == Kind::Wal && off == 0 && amt >= 12 {
            // The WAL header's page size must be ours.
            let sz = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
            if sz as usize != PAGE {
                return ffi::SQLITE_IOERR_WRITE;
            }
        }
        let (start, end) = span(f.kind, off, amt);
        let mut buf = if start == off && end == off + amt {
            data.to_vec()
        } else {
            let (rc, mut buf) = read_open(f, start, end, false);
            if rc != ffi::SQLITE_OK && rc != ffi::SQLITE_IOERR_SHORT_READ {
                return rc;
            }
            let at = (off - start) as usize;
            buf[at..at + amt as usize].copy_from_slice(data);
            buf
        };
        let Some(key) = &f.key else {
            return ffi::SQLITE_IOERR_WRITE;
        };
        for r in pages(f.kind, start, end) {
            let page = &mut buf[(r.start - start) as usize..(r.end - start) as usize];
            // SQLite leaves the reserve zeroed; anything else would not read
            // back as written (and WAL checksums cover it).
            if page[SEALED..].iter().any(|&b| b != 0) {
                return ffi::SQLITE_IOERR_WRITE;
            }
            if f.kind == Kind::Db && r.page == Some(0) && page.starts_with(PLAIN_MAGIC) {
                let size = u16::from_be_bytes([page[16], page[17]]) as usize;
                if size != PAGE || page[20] as usize != RESERVE {
                    return ffi::SQLITE_IOERR_WRITE;
                }
            }
            let first = f.kind == Kind::Db && r.page == Some(0);
            if first && !page.starts_with(PLAIN_MAGIC) {
                return ffi::SQLITE_IOERR_WRITE;
            }
            let skip = if first { PLAIN_MAGIC.len() } else { 0 };
            if key
                .seal(page, skip, aad(f.kind, r.page.unwrap_or(0)))
                .is_err()
            {
                return ffi::SQLITE_IOERR_WRITE;
            }
            if first {
                page[..skip].copy_from_slice(ENC_MAGIC);
            }
        }
        let Some(write) = methods(f.inner).xWrite else {
            return ffi::SQLITE_IOERR_WRITE;
        };
        // SAFETY: our buffer of the stated length.
        unsafe {
            write(
                f.inner,
                buf.as_ptr().cast(),
                buf.len() as c_int,
                start as i64,
            )
        }
    }

    macro_rules! forward {
        ($name:ident, $m:ident, ($($a:ident: $t:ty),*), $ret:ty, $default:expr) => {
            unsafe extern "C" fn $name(file: *mut ffi::sqlite3_file $(, $a: $t)*) -> $ret {
                let f = me(file);
                // SAFETY: forwarding to the inner file's own method.
                methods(f.inner).$m.map_or($default, |m| unsafe { m(f.inner $(, $a)*) })
            }
        };
    }
    forward!(x_truncate, xTruncate, (size: i64), c_int, ffi::SQLITE_IOERR_TRUNCATE);
    forward!(x_sync, xSync, (flags: c_int), c_int, ffi::SQLITE_IOERR_FSYNC);
    forward!(x_file_size, xFileSize, (size: *mut i64), c_int, ffi::SQLITE_IOERR_FSTAT);
    forward!(x_lock, xLock, (l: c_int), c_int, ffi::SQLITE_IOERR_LOCK);
    forward!(x_unlock, xUnlock, (l: c_int), c_int, ffi::SQLITE_IOERR_UNLOCK);
    forward!(x_check_reserved, xCheckReservedLock, (out: *mut c_int), c_int, ffi::SQLITE_IOERR_CHECKRESERVEDLOCK);
    forward!(x_file_control, xFileControl, (op: c_int, arg: *mut c_void), c_int, ffi::SQLITE_NOTFOUND);
    forward!(x_sector_size, xSectorSize, (), c_int, 4096);
    forward!(x_device, xDeviceCharacteristics, (), c_int, 0);
    forward!(x_shm_map, xShmMap, (pg: c_int, sz: c_int, extend: c_int, out: *mut *mut c_void), c_int, ffi::SQLITE_IOERR_SHMMAP);
    forward!(x_shm_lock, xShmLock, (off: c_int, n: c_int, flags: c_int), c_int, ffi::SQLITE_IOERR_SHMLOCK);
    forward!(x_shm_unmap, xShmUnmap, (del: c_int), c_int, ffi::SQLITE_OK);

    unsafe extern "C" fn x_shm_barrier(file: *mut ffi::sqlite3_file) {
        let f = me(file);
        if let Some(m) = methods(f.inner).xShmBarrier {
            // SAFETY: forwarding to the inner file.
            unsafe { m(f.inner) }
        }
    }
}

#[cfg(test)]
mod tests;
