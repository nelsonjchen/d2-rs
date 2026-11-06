// SPDX-License-Identifier: MPL-2.0
//! Native import files with Go os.PathError formatting.
use super::{ByteReader, ReadError, os_errors::errno_message};
use std::{fs::File, io::Read};

struct NativeFile {
    file: File,
    path: String,
}
fn path_error(operation: &str, path: &str, error: std::io::Error) -> String {
    let message = error
        .raw_os_error()
        .map_or_else(|| error.to_string(), errno_message);
    format!("{operation} {path}: {message}")
}
/// Open a native file for the import cache's `load_stream` provider.
/// Virtual filesystem callers should supply their own `ByteReader` instead.
pub fn open_native(path: &str) -> Result<Box<dyn ByteReader>, String> {
    // Go syscall.BytePtrFromString rejects NUL with EINVAL.
    if path.as_bytes().contains(&0) {
        return Err(format!("open {path}: {}", errno_message(22)));
    }
    loop {
        match File::open(path) {
            Ok(file) => {
                return Ok(Box::new(NativeFile {
                    file,
                    path: path.into(),
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(path_error("open", path, error)),
        }
    }
}
impl ByteReader for NativeFile {
    fn read_bytes(&mut self, destination: &mut [u8]) -> (usize, Option<ReadError>) {
        if destination.is_empty() {
            return (0, None);
        }
        let limit = destination.len().min(1 << 30);
        let destination = &mut destination[..limit];
        loop {
            match self.file.read(destination) {
                Ok(0) => return (0, Some(ReadError::Eof)),
                Ok(n) => return (n, None),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return (
                        0,
                        Some(ReadError::Other(path_error("read", &self.path, error))),
                    );
                }
            }
        }
    }
}
