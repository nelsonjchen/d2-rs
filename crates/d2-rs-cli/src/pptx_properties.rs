// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::Path;

// d2cli/main.go:630–644: stdout uses the input basename; a file output uses
// its own basename. This is presentation metadata, independent of board labels.
pub(super) fn pptx_document_properties(
    input: &str,
    output: Option<&str>,
) -> d2_rs_export::PptxDocumentProperties {
    let default_output = super::default_output_path(input, d2_rs_export::ExportFormat::Pptx);
    let output = output
        .map(Path::new)
        .or(default_output.as_deref())
        .unwrap_or_else(|| Path::new("-"));
    let source = if output == Path::new("-") {
        Path::new(input)
    } else {
        output
    };
    let title = if source == Path::new("-") {
        "stdin".to_owned()
    } else {
        // filepath.Ext treats a leading dot as an extension too.
        let name = source.file_name().unwrap_or_default().to_string_lossy();
        name.rsplit_once('.')
            .map_or(name.as_ref(), |(stem, _)| stem)
            .to_owned()
    };
    d2_rs_export::PptxDocumentProperties {
        subject: title.clone(),
        title,
        // Test hook for complete-package contracts captured under a fixed
        // account. Ordinary CLI metadata still follows Go's user.Current.
        creator: std::env::var("D2RS_TEST_PPTX_CREATOR").unwrap_or_else(|_| current_username()),
        ..Default::default()
    }
}

#[cfg(unix)]
fn current_username() -> String {
    // os/user.Current uses the current UID's account entry, not the spoofable
    // USER/LOGNAME environment. getpwuid_r keeps storage local to this call.
    let mut buffer = vec![0_u8; 1024];
    loop {
        let mut account = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found = std::ptr::null_mut();
        // SAFETY: getpwuid_r writes only to the provided account and buffer.
        // The returned name remains valid until this call's buffer is dropped.
        let result = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                account.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if result == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if result != 0 || found.is_null() {
            return String::new();
        }
        // SAFETY: a successful lookup initialized account and its name.
        return unsafe { std::ffi::CStr::from_ptr((*found).pw_name) }
            .to_string_lossy()
            .into_owned();
    }
}

#[cfg(windows)]
fn current_username() -> String {
    // os/user.Current requests the domain-qualified SAM account name.
    #[link(name = "secur32")]
    unsafe extern "system" {
        fn GetUserNameExW(format: i32, buffer: *mut u16, size: *mut u32) -> u8;
    }
    let mut size = 0_u32;
    // SAFETY: the first call queries the required buffer length.
    unsafe { GetUserNameExW(2, std::ptr::null_mut(), &mut size) };
    if size == 0 {
        return String::new();
    }
    let mut buffer = vec![0_u16; size as usize];
    // SAFETY: the allocated buffer holds the requested number of UTF-16 units.
    if unsafe { GetUserNameExW(2, buffer.as_mut_ptr(), &mut size) } == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..size as usize])
}

#[cfg(not(any(unix, windows)))]
fn current_username() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_subject_follow_output_and_stdout_paths() {
        for (input, output, expected) in [
            ("a/source.d2", Some("b/output.deck.pptx"), "output.deck"),
            ("a/source.d2", Some("-"), "source"),
            ("a/source.d2", None, "source"),
            ("-", Some("-"), "stdin"),
            ("-", None, "stdin"),
            ("-", Some("named.pptx"), "named"),
            (".hidden", Some("-"), ""),
        ] {
            let properties = pptx_document_properties(input, output);
            assert_eq!(properties.title, expected);
            assert_eq!(properties.subject, expected);
        }
    }

    #[test]
    fn file_and_multiboard_stdout_packages_keep_document_properties() {
        use std::io::Read;
        let directory =
            std::env::temp_dir().join(format!("d2-rs-pptx-properties-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("source.d2");
        std::fs::write(&input, "x -> y\nlayers: {child: {a -> b}}\n").unwrap();
        let output = directory.join("chosen.pptx");
        let input_arg = input.to_string_lossy().into_owned();
        for destination in [output.to_string_lossy().into_owned(), "-".into()] {
            let result = super::super::run_document(&[
                "--stdout-format=pptx".into(),
                input_arg.clone(),
                destination.clone(),
            ])
            .unwrap();
            let bytes = if destination == "-" {
                result
            } else {
                std::fs::read(&output).unwrap()
            };
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut core = String::new();
            archive
                .by_name("docProps/core.xml")
                .unwrap()
                .read_to_string(&mut core)
                .unwrap();
            let title = if destination == "-" {
                "source"
            } else {
                "chosen"
            };
            assert!(core.contains(&format!("<dc:title>{title}</dc:title>")));
            assert!(core.contains(&format!("<dc:subject>{title}</dc:subject>")));
            assert!(archive.by_name("ppt/slides/slide2.xml").is_ok());
        }
        assert!(!directory.join("-").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
