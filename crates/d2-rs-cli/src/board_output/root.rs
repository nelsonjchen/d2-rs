// SPDX-License-Identifier: MPL-2.0
//! Directory-handle operations used by d2cli's boardOutputRoot.
use std::fs::{File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

/// Identity is taken from the same open handle as the metadata, never from
/// size/timestamp guesses when deciding whether rollback still owns a file.
pub(super) struct Info {
    metadata: Metadata,
    #[cfg(windows)]
    identity: WindowsFileId,
}
#[cfg(windows)]
#[derive(PartialEq, Eq)]
#[repr(C)]
struct WindowsFileId {
    volume: u64,
    id: [u8; 16],
}
impl std::ops::Deref for Info {
    type Target = Metadata;
    fn deref(&self) -> &Metadata {
        &self.metadata
    }
}
impl Info {
    fn from_file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        #[cfg(windows)]
        let identity = {
            use std::os::windows::io::AsRawHandle;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetFileInformationByHandleEx(
                    handle: *mut std::ffi::c_void,
                    class: i32,
                    information: *mut std::ffi::c_void,
                    size: u32,
                ) -> i32;
            }
            let mut identity = std::mem::MaybeUninit::<WindowsFileId>::uninit();
            // FileIdInfo (18): volume serial and complete 128-bit file ID.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    18,
                    identity.as_mut_ptr().cast(),
                    std::mem::size_of::<WindowsFileId>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            unsafe { identity.assume_init() }
        };
        Ok(Self {
            metadata,
            #[cfg(windows)]
            identity,
        })
    }
    pub(super) fn same_file(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            self.dev() == other.dev() && self.ino() == other.ino()
        }
        #[cfg(windows)]
        {
            self.identity == other.identity
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = other;
            false
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::{CStr, CString, OsString};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    pub(super) struct Handle(File);
    fn name(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    }
    fn opened(fd: libc::c_int) -> io::Result<File> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: successful openat returns a new owned descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn syscall(result: libc::c_int) -> io::Result<()> {
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    impl Handle {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            let before = std::fs::symlink_metadata(path)?;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            let after = file.metadata()?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                return Err(io::Error::other("directory changed while opening it"));
            }
            Ok(Self(file))
        }
        // Resolve parents from the retained root descriptor. Never use an
        // absolute destination after preflight: it could have been replaced.
        fn parent(&self, rel: &Path) -> io::Result<(File, CString)> {
            use std::collections::VecDeque;
            use std::path::Component;
            let mut pending = rel
                .components()
                .map(|part| part.as_os_str().to_owned())
                .collect::<VecDeque<_>>();
            let mut directories = vec![self.0.try_clone()?];
            let mut symlinks = 0;
            while let Some(part) = pending.pop_front() {
                let path = Path::new(&part);
                match path.components().next() {
                    Some(Component::CurDir) => continue,
                    Some(Component::ParentDir) => {
                        if directories.len() == 1 {
                            return Err(io::Error::other("path escapes from parent"));
                        }
                        directories.pop();
                        continue;
                    }
                    Some(Component::Normal(_)) => {}
                    _ => return Err(io::Error::other("path escapes from parent")),
                }
                let component = name(path)?;
                if pending.is_empty() {
                    return Ok((directories.pop().unwrap(), component));
                }
                let directory = directories.last().unwrap();
                // Open each ancestor relative to the retained directory. If
                // it is a symlink, resolve its relative target under this same
                // descriptor stack; absolute targets and root escapes fail.
                let fd = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        component.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                match opened(fd) {
                    Ok(next) => directories.push(next),
                    Err(error) => {
                        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
                        let code = unsafe {
                            libc::fstatat(
                                directory.as_raw_fd(),
                                component.as_ptr(),
                                info.as_mut_ptr(),
                                libc::AT_SYMLINK_NOFOLLOW,
                            )
                        };
                        if code != 0
                            || unsafe { info.assume_init() }.st_mode as u32 & libc::S_IFMT as u32
                                != libc::S_IFLNK as u32
                        {
                            return Err(error);
                        }
                        symlinks += 1;
                        if symlinks > 8 {
                            return Err(io::Error::from_raw_os_error(libc::ELOOP));
                        }
                        let mut bytes = vec![0u8; 1024];
                        loop {
                            let count = unsafe {
                                libc::readlinkat(
                                    directory.as_raw_fd(),
                                    component.as_ptr(),
                                    bytes.as_mut_ptr().cast(),
                                    bytes.len(),
                                )
                            };
                            if count < 0 {
                                return Err(io::Error::last_os_error());
                            }
                            if (count as usize) < bytes.len() {
                                bytes.truncate(count as usize);
                                break;
                            }
                            if bytes.len() >= 1 << 20 {
                                return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
                            }
                            bytes.resize(bytes.len() * 2, 0);
                        }
                        let target = PathBuf::from(OsString::from_vec(bytes));
                        if target.is_absolute() {
                            return Err(io::Error::other("path escapes from parent"));
                        }
                        for part in target.components().rev() {
                            pending.push_front(part.as_os_str().to_owned());
                        }
                    }
                }
            }
            Ok((directories.pop().unwrap(), CString::new(".").unwrap()))
        }
        pub(super) fn metadata(&self, rel: &Path) -> io::Result<Info> {
            let (directory, component) = self.parent(rel)?;
            // Open only a regular file/directory without following the final
            // link. Metadata for symlinks uses fstatat below in kind().
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            Info::from_file(&opened(fd)?)
        }
        pub(super) fn kind(&self, rel: &Path) -> io::Result<u32> {
            let (directory, component) = self.parent(rel)?;
            let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: fstatat initializes info on success; pointers are valid.
            syscall(unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    info.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            })?;
            Ok(unsafe { info.assume_init() }.st_mode as u32 & libc::S_IFMT as u32)
        }
        pub(super) fn read(&self, rel: &Path, expected: &Info) -> io::Result<Vec<u8>> {
            use std::io::Read;
            let (directory, component) = self.parent(rel)?;
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            let mut file = opened(fd)?;
            let info = file.metadata()?;
            if !info.is_file() || info.dev() != expected.dev() || info.ino() != expected.ino() {
                return Err(io::Error::other("file changed while opening it"));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        pub(super) fn mkdir(&self, rel: &Path, mode: u32) -> io::Result<()> {
            let (directory, component) = self.parent(rel)?;
            syscall(unsafe {
                libc::mkdirat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    mode as libc::mode_t,
                )
            })
        }
        pub(super) fn remove(&self, rel: &Path, directory_entry: bool) -> io::Result<()> {
            let (directory, component) = self.parent(rel)?;
            syscall(unsafe {
                libc::unlinkat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    if directory_entry {
                        libc::AT_REMOVEDIR
                    } else {
                        0
                    },
                )
            })
        }
        pub(super) fn create(&self, rel: &Path) -> io::Result<File> {
            let (directory, component) = self.parent(rel)?;
            opened(unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                    0o600,
                )
            })
        }
        pub(super) fn install(
            &self,
            source: &Path,
            destination: &Path,
            replacing: bool,
        ) -> io::Result<()> {
            let (from_dir, from) = self.parent(source)?;
            let (to_dir, to) = self.parent(destination)?;
            if replacing {
                syscall(unsafe {
                    libc::renameat(
                        from_dir.as_raw_fd(),
                        from.as_ptr(),
                        to_dir.as_raw_fd(),
                        to.as_ptr(),
                    )
                })
            } else {
                syscall(unsafe {
                    libc::linkat(
                        from_dir.as_raw_fd(),
                        from.as_ptr(),
                        to_dir.as_raw_fd(),
                        to.as_ptr(),
                        0,
                    )
                })
            }
        }
        pub(super) fn entries(&self, rel: &Path) -> io::Result<Vec<OsString>> {
            let (directory, component) = self.parent(rel)?;
            let file = opened(unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })?;
            use std::os::fd::IntoRawFd;
            let fd = file.into_raw_fd();
            let stream = unsafe { libc::fdopendir(fd) };
            if stream.is_null() {
                unsafe {
                    libc::close(fd);
                }
                return Err(io::Error::last_os_error());
            }
            struct Stream(*mut libc::DIR);
            impl Drop for Stream {
                fn drop(&mut self) {
                    unsafe {
                        libc::closedir(self.0);
                    }
                }
            }
            let stream = Stream(stream);
            let mut names = Vec::new();
            loop {
                // readdir uses errno to distinguish exhaustion from failure.
                #[cfg(target_os = "macos")]
                unsafe {
                    *libc::__error() = 0;
                }
                #[cfg(not(target_os = "macos"))]
                unsafe {
                    *libc::__errno_location() = 0;
                }
                let entry = unsafe { libc::readdir(stream.0) };
                if entry.is_null() {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(0) {
                        return Err(error);
                    }
                    break;
                }
                let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if bytes != b"." && bytes != b".." {
                    names.push(OsString::from_vec(bytes.to_vec()));
                }
            }
            Ok(names)
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use super::*;
    use std::ffi::OsString;
    pub(super) struct Handle(PathBuf);
    impl Handle {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            Ok(Self(path.to_owned()))
        }
        fn path(&self, rel: &Path) -> io::Result<PathBuf> {
            let mut path = self.0.clone();
            for component in rel.components() {
                let std::path::Component::Normal(component) = component else {
                    return Err(io::Error::other("path escapes from parent"));
                };
                if path != self.0 && std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                    return Err(io::Error::other("path escapes from parent"));
                }
                path.push(component);
            }
            Ok(path)
        }
        pub(super) fn metadata(&self, rel: &Path) -> io::Result<Info> {
            let path = self.path(rel)?;
            #[cfg(windows)]
            let file = {
                use std::os::windows::fs::OpenOptionsExt;
                std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(0x02000000 | 0x00200000)
                    .open(path)?
            };
            #[cfg(not(windows))]
            let file = File::open(path)?;
            Info::from_file(&file)
        }
        pub(super) fn kind(&self, rel: &Path) -> io::Result<u32> {
            let m = self.metadata(rel)?;
            Ok(if m.file_type().is_symlink() {
                0xa000
            } else if m.is_dir() {
                0x4000
            } else {
                0x8000
            })
        }
        pub(super) fn read(&self, rel: &Path, _expected: &Info) -> io::Result<Vec<u8>> {
            std::fs::read(self.path(rel)?)
        }
        pub(super) fn mkdir(&self, rel: &Path, _mode: u32) -> io::Result<()> {
            std::fs::create_dir(self.path(rel)?)
        }
        pub(super) fn remove(&self, rel: &Path, dir: bool) -> io::Result<()> {
            if dir {
                std::fs::remove_dir(self.path(rel)?)
            } else {
                std::fs::remove_file(self.path(rel)?)
            }
        }
        pub(super) fn create(&self, rel: &Path) -> io::Result<File> {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.path(rel)?)
        }
        pub(super) fn install(
            &self,
            source: &Path,
            destination: &Path,
            replacing: bool,
        ) -> io::Result<()> {
            if replacing {
                std::fs::rename(self.path(source)?, self.path(destination)?)
            } else {
                std::fs::hard_link(self.path(source)?, self.path(destination)?)
            }
        }
        pub(super) fn entries(&self, rel: &Path) -> io::Result<Vec<OsString>> {
            std::fs::read_dir(self.path(rel)?)?
                .map(|e| e.map(|e| e.file_name()))
                .collect()
        }
    }
}

pub(super) struct Root {
    pub(super) path: PathBuf,
    handle: platform::Handle,
}
impl Root {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        Ok(Self {
            path: path.to_owned(),
            handle: platform::Handle::open(path)
                .map_err(|e| format!("open board output root {}: {e}", super::quote(path)))?,
        })
    }
    pub(super) fn display(&self, rel: &Path) -> String {
        super::quote(&self.path.join(rel))
    }
    pub(super) fn kind(&self, rel: &Path) -> io::Result<u32> {
        self.handle.kind(rel)
    }
    pub(super) fn metadata(&self, rel: &Path) -> io::Result<Info> {
        let kind = self.handle.kind(rel)?;
        if kind == 0xa000 {
            return Err(io::Error::other("not a regular file or directory"));
        }
        self.handle.metadata(rel)
    }
    pub(super) fn read(&self, rel: &Path, expected: &Info) -> io::Result<Vec<u8>> {
        self.handle.read(rel, expected)
    }
    pub(super) fn mkdir(&self, rel: &Path, mode: u32) -> io::Result<()> {
        self.handle.mkdir(rel, mode)
    }
    pub(super) fn remove(&self, rel: &Path, dir: bool) -> io::Result<()> {
        self.handle.remove(rel, dir)
    }
    pub(super) fn entries(&self, rel: &Path) -> io::Result<Vec<std::ffi::OsString>> {
        self.handle.entries(rel)
    }
    /// Keep applied state even if a later stat, copy, or temporary cleanup
    /// fails, so the caller can journal and undo every visible mutation.
    pub(super) fn install(
        &self,
        source: &Path,
        rel: &Path,
        replacing: bool,
    ) -> (bool, Option<Info>, Result<(), String>) {
        use std::io::Write;
        let mut applied = false;
        let mut installed = None;
        let result = (|| {
            let mut input = File::open(source).map_err(|e| e.to_string())?;
            let info = input.metadata().map_err(|e| e.to_string())?;
            if !info.is_file() {
                return Err("source is not a regular file".to_owned());
            }
            let mode = super::mode(&info);
            let parent = rel.parent().unwrap_or(Path::new(""));
            let (temporary, mut file) = (0..100)
                .find_map(|_| {
                    let mut random = [0u8; 16];
                    if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random)
                        .is_err()
                    {
                        return Some(Err("generate temporary board output name".to_owned()));
                    }
                    let temporary = parent.join(format!(".d2-board-file-{}", super::hex(&random)));
                    match self.handle.create(&temporary) {
                        Ok(file) => Some(Ok((temporary, file))),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                        Err(error) => Some(Err(format!(
                            "prepare board output {}: {error}",
                            self.display(rel)
                        ))),
                    }
                })
                .unwrap_or_else(|| Err("create unique temporary board output file".to_owned()))?;
            let temporary_info = Info::from_file(&file).map_err(|e| e.to_string())?;
            let publication = (|| -> io::Result<()> {
                std::io::copy(&mut input, &mut file)?;
                super::set_file_mode(&file, mode)?;
                // Retain the installed identity through the same open file;
                // later path lookups are checked separately during rollback.
                let prepared_info = Info::from_file(&file)?;
                match self.handle.install(&temporary, rel, replacing) {
                    Ok(()) => {
                        applied = true;
                        installed = Some(prepared_info);
                    }
                    Err(first_error) if replacing => {
                        // Go's Windows replacement fallback removes the old
                        // destination, then retries rename under the journal.
                        #[cfg(windows)]
                        {
                            self.handle.remove(rel, false)?;
                            applied = true;
                            self.handle.install(&temporary, rel, true)?;
                            installed = Some(prepared_info);
                        }
                        #[cfg(not(windows))]
                        {
                            return Err(first_error);
                        }
                    }
                    Err(link_error) => {
                        match self.handle.kind(rel) {
                            Ok(_) => {
                                return Err(io::Error::new(
                                    io::ErrorKind::AlreadyExists,
                                    "destination appeared during publication",
                                ));
                            }
                            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error),
                        }
                        // Some filesystems do not support hard links. An
                        // exclusive copy preserves no-clobber publication.
                        let mut output = self.handle.create(rel).map_err(|error| {
                            if error.kind() == io::ErrorKind::AlreadyExists {
                                error
                            } else {
                                link_error
                            }
                        })?;
                        applied = true;
                        installed = Some(Info::from_file(&output)?);
                        let mut bytes = &self.handle.read(&temporary, &temporary_info)?[..];
                        std::io::copy(&mut bytes, &mut output)?;
                        super::set_file_mode(&output, mode)?;
                        output.flush()?;
                    }
                }
                Ok(())
            })()
            .map_err(|error| format!("publish board output {}: {error}", self.display(rel)));
            let cleanup = match self.handle.metadata(&temporary) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!(
                    "inspect temporary board output {}: {error}",
                    self.display(&temporary)
                )),
                Ok(current) if !super::transaction::same_file(&temporary_info, &current) => {
                    Err(format!(
                        "refusing to remove changed temporary board output {}",
                        self.display(&temporary)
                    ))
                }
                Ok(_) => self.handle.remove(&temporary, false).map_err(|error| {
                    format!(
                        "remove temporary board output {}: {error}",
                        self.display(&temporary)
                    )
                }),
            };
            match (publication, cleanup) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                (Err(first), Err(second)) => Err(format!("{first}\n{second}")),
            }
        })();
        (applied, installed, result)
    }
}
