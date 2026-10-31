#!/usr/bin/env python3
"""Translate pinned Go errno strings; no host libc/localization dependence."""
from pathlib import Path
import hashlib, json, re
root = Path(__file__).resolve().parents[1]
go = root / '.reference/go-mod/golang.org/toolchain@v0.0.1-go1.27.0.darwin-arm64/src/syscall'
lines = ['// SPDX-License-Identifier: BSD-3-Clause', '// Generated from Go 1.27.0 syscall tables; see THIRD_PARTY_NOTICES.md.']
for os_name, arch, source in [('macos', 'aarch64', 'zerrors_darwin_arm64.go'), ('macos', 'x86_64', 'zerrors_darwin_amd64.go'), ('linux', 'aarch64', 'zerrors_linux_arm64.go'), ('linux', 'x86_64', 'zerrors_linux_amd64.go')]:
    data = (go / source).read_bytes()
    table = data.decode().split('var errors = [...]string{', 1)[1].split('\n}',1)[0]
    lines += [f'// {source} sha256: {hashlib.sha256(data).hexdigest()}', f'#[cfg(all(target_os = "{os_name}", target_arch = "{arch}"))]' , 'pub(super) fn errno_message(code: i32) -> String {', '    match code {']
    for number, value in re.findall(r'^\s*(\d+):\s*(".*"),$', table, re.M):
        lines.append(f'        {number} => {value}.into(),')
    lines += ['        _ => format!("errno {code}"),', '    }', '}']
(root / 'crates/d2-rs-language/src/stream/os_errors.rs').write_text('\n'.join(lines)+'\n')
