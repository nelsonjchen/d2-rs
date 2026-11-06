// SPDX-License-Identifier: MPL-2.0
//! Windows diagnostics matching Go 1.27 syscall.Errno.Error.

pub fn error_message(error: &std::io::Error) -> String {
    let Some(code) = error.raw_os_error() else {
        return error.to_string();
    };
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FormatMessageW(
            flags: u32,
            source: *const std::ffi::c_void,
            message: u32,
            language: u32,
            buffer: *mut u16,
            size: u32,
            arguments: *const std::ffi::c_void,
        ) -> u32;
    }
    let mut buffer = [0_u16; 300];
    // syscall_windows.go asks for US English first, then the system language.
    for language in [0x0409, 0] {
        // SAFETY: FormatMessageW writes at most buffer.len() UTF-16 units;
        // FROM_SYSTEM and IGNORE_INSERTS need no source or argument pointers.
        let length = unsafe {
            FormatMessageW(
                0x1000 | 0x2000 | 0x0200,
                std::ptr::null(),
                code as u32,
                language,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                std::ptr::null(),
            )
        } as usize;
        if length != 0 {
            return String::from_utf16_lossy(&buffer[..length])
                .trim_end_matches(['\r', '\n'])
                .to_owned();
        }
    }
    format!("winapi error #{code}")
}
