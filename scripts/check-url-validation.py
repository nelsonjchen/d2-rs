#!/usr/bin/env python3
"""Go net/url Parse/ParseRequestURI diagnostics and D2 URL predicates.

Fixture generator for recipe `lang-public-url-validation`
(tools/fixtures/langlibelk-language-url.toml): builds the stdlib-only
tools/url-validation-oracle (GODEBUG default=go1.27, as the pinned D2 module)
with the manifest Go toolchain, no D2 checkout involved, and writes
public-url-validation-v090.json (historical logical path) to the staged output.
`cargo test -p d2-rs-language --features oracle-fixtures` does the native
comparison (docs/FIXTURES.md).
"""
import subprocess

from fixture_recipe import go_env, workdir
from language_oracle import call, fixture, write
from references import ROOT

def url_requests() -> list[dict]:
    """Go net/url Parse/ParseRequestURI inputs (all bytes in URL components, escapes, IPs, authorities)."""
    urls = [b'', b'*', b'::????:::%%orange', b'1:a', b'a_b:x', b'a:%ZZ', b'a:/%ZZ', b'/x?%ZZ', b'/x#%ZZ', b'/x#\x00', b'//x', b'///x', b'http://host/x#%ZZ']
    for c in range(256):
        for template in [b'http://h/%s', b'http://%s/x', b'http://u%s:p@h/x', b'/x#%s']:
            urls.append(template.replace(b'%s', bytes([c])))
    for escape in [b'%', b'%0', b'%ZZ', b'%0z', b'%00', b'%20', b'%25', b'%2f', b'%7f', b'%80', b'%ff', b'%E2%80%94']:
        for template in [b'%s', b'http://h/%s', b'http://%s/x', b'http://u%s:p@h/x', b'http://h/x?%s', b'http://h/x#%s', b'http://[::1%25%s]/x']:
            urls.append(template.replace(b'%s', escape))
    ips = [b'', b'bad', b'1.2.3.4', b'1.2.3', b'01.2.3.4', b'256.2.3.4', b'.1.2.3', b'1..2.3', b'1.2.3.', b'1.2.3.4.5', b'1.2.x.4', b'::', b'::1',
           b'::1%25en0', b'::1%25', b'%25en0', b'1:2', b':1', b'1:', b'1:2:3:4:5:6:7:8', b'1:2:3:4:5:6:7:8:9', b'1::2::3', b'12345::',
           b'::ffff:192.0.2.1', b'1:192.0.2.1', b'1:2:3:4:5:6:192.0.2.1', b'1:2:3:4:5:6:7:192.0.2.1', b'1:2:3:4:5:6:7::8', b'::1x', b'::1[',
           b'::1]', b'::1%25a%20b']
    for ip in ips:
        for suffix in [b'', b':', b':80', b':x', b':1,b:2']:
            urls.append(b'http://[' + ip + b']' + suffix + b'/x')
    for scheme in [b'http', b'HTTPS', b'postgres', b'']:
        for host in [b'', b'h', b'h:80', b'a:1,b:2', b'u@h', b'u:p@h', b'u@@h', b'u%ZZ@h', b'u%ZZ@h:x', b'h[x]', b'h]']:
            urls.append(scheme + b'://' + host + b'/x')
    urls = list(dict.fromkeys(urls))
    return [dict(hex=u.hex(), request=mode) for u in urls for mode in [False, True]]


go = workdir() / "bin/url-validation-oracle"
subprocess.run(["go", "build", "-o", str(go), str(ROOT / "tools/url-validation-oracle/main.go")], cwd=workdir(), check=True, env=go_env())
requests = url_requests()
write(fixture("public-url-validation-v090.json"), dict(
    scope="Go 1.27 default=go1.27 (matching pinned D2 caller module) net/url Parse and ParseRequestURI diagnostics and D2 predicates; no URL serialization claim",
    requests=requests, source=call(go, requests)))
print(f"{len(requests)} URL observations captured")
