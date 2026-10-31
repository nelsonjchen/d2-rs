# Third-party notices

d2-rs's first-party code is licensed under MPL-2.0. Third-party portions
retain the terms below; dependency notices are in DEPENDENCY_NOTICES.md.

## D2 and TALA

Copyright 2022 Terrastruct Inc.

TALA's runtime source was published under D2's MPL-2.0 license in
https://github.com/d2lang/d2/pull/2882 (merge commit 99b0791).
The upstream TALA authors list recognizes Alexander Wang, Gavin Nishizawa,
and Júlio César Batista. See docs/provenance.md for Weftan's recovery-based
translation history and the limits of the upstream comparison.

The layout pipeline and D2-derived geometry are distributed under MPL-2.0;
the complete license is in LICENSE. Rust translation and subsequent
modifications: Copyright 2026 Nelson Chen.

The OOXML base package in `crates/d2-rs-export/assets/d2-template.pptx` is the
v0.9.0 D2 PowerPoint template from `lib/pptx/template.pptx`. It is retained
under D2's MPL-2.0 terms; the pinned D2 reference checkout is the corresponding
source.

## Dagro, Dagre, and Graphlib

`crates/d2-rs-dagre` translates the ranking, compound ordering, and
Brandes–Köpf placement algorithms from `github.com/d2lang/dagro v0.2.1`,
the dependency pinned by D2 v0.9.0. Dagro is derived from Dagre 3.1.1 and
Graphlib. These portions retain the following MIT notice; Rust additions
and modifications are distributed under MPL-2.0.

```text
MIT License

Copyright (c) 2012-2014 Chris Pettitt
Copyright (c) 2026 Dagro contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## MathJax TeX font data

`crates/d2-rs-math/src/glyphs.rs` contains a source-derived subset of
`github.com/d2lang/mathjax-go v0.1.0/internal/font/data_normal_gen.go`, itself
generated from MathJax 3.2.2 TeX font data. The upstream generated file and
MathJax sources are Copyright 2017–2022 The MathJax Consortium and are licensed
under the Apache License, Version 2.0. Translated parser/layout portions and
the generated glyph metrics and outlines retain Apache-2.0 terms; first-party
Rust modifications retain MPL-2.0. The pinned module's `LICENSE`, `NOTICE`, and
`PROVENANCE.md` are the corresponding source and attribution records.

Apache License 2.0:
https://www.apache.org/licenses/LICENSE-2.0

The exact MathJax SVG parser/layout and generated table translations in
`crates/d2-rs-math` also derive from pinned `mathjax-go v0.1.0`, a translation
and modification of MathJax 3.2.2. Copyright 2026 D2 contributors;
Copyright (c) 2009–2022 The MathJax Consortium; color support additionally
Copyright (c) 2018–2022 Omar Al-Ithawi and The MathJax Consortium.
The mhchem portions retain Copyright (c) 2015–2021 Martin Hensel and the
initial go-tex translation's Copyright (c) 2026 luo-studio and
Copyright (c) erweixin. The complete Apache-2.0 license, upstream NOTICE,
and additional MIT license are retained without changes in
`crates/d2-rs-math/licenses/mathjax-go-{LICENSE,NOTICE,MIT-Luo-Studio}.txt`.
`crates/d2-rs-math/SOURCE.md` maps the translated source files and generators.

## Source Sans Pro and Source Code Pro

The native text measurer and raster exporter embed the Source Sans Pro and
Source Code Pro faces from D2 v0.9.0's `d2renderers/d2fonts/ttf` directory,
including regular, semibold, bold, and italic variants where available. The
fonts are Copyright 2010–2018 Adobe Systems Incorporated and are licensed
under the SIL Open Font License, Version 1.1. The complete font license is included at
`crates/d2-rs-text/assets/OFL-1.1.txt` and remains applicable to the font
bytes.

## Fibonacci heap

The recovered heap translated in crates/weftan/src/engine/routing/fibheap.rs
closely corresponds to Workiva/go-datastructures' fibheap/fibheap.go,
including circular-list merging, consolidation, cascading cuts, and
less-than-or-equal decrease-key ties. We preserve its Apache-2.0 notice.
The Rust version replaces pointers with arena indices, adds generic payloads,
and reuses scratch allocations.

Reference reviewed: https://github.com/Workiva/go-datastructures/blob/89d15facb2e3d7ae0c819fa43b66a2153e77196f/fibheap/fibheap.go

Special thanks to Keith Schwarz, whose code and documentation were used as
a reference by that implementation:
http://www.keithschwarz.com/interesting/code/?dir=fibonacci-heap

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at
https://www.apache.org/licenses/LICENSE-2.0
Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.


## Go standard library

`crates/d2-rs-export/src/go_png.rs` and `go_zip.rs` translate Go 1.27.0's
RGB/NRGBA8 PNG writer, adaptive filters, buffered IDAT emission and the
reachable ZIP Create/Copy headers, descriptors and directory records from
`image/png/writer.go`, `bufio/bufio.go` and
`archive/zip/{writer,struct,register}.go`. Copyright 2009, 2010 The Go Authors.
All rights reserved. The complete Go BSD-3-Clause terms below apply.

The URL decoder in `crates/d2-rs-wasm/src/url_codec.rs` translates Go1.27.0
`encoding/base64` decodeQuantum and `compress/flate` Huffman/bit-reader rules.
Copyright 2009 The Go Authors. The Go BSD terms below apply.

The WASM host-value adapter in `crates/d2-rs-wasm/src/go-js-string-args.cjs`
follows Go 1.27.0 `syscall/js.Value.String` and `wasm_exec.js` value
marshaling. Copyright 2018 The Go Authors. The Go BSD terms below apply.

`crates/d2-rs-goflate/src/` (`deflate.rs`, `deflatefast.rs`, `level1.rs`
through `level6.rs`, `huffman_bit_writer.rs`, `huffman_code.rs`, `token.rs`,
`zlib.rs`, `adler32.rs`) translates the Go 1.27.0 `compress/flate` encoder,
`compress/zlib` writer and `hash/adler32`. Copyright 2009-2026 The Go Authors.
The Go BSD terms below apply.

Portions of `crates/d2-tala-rs/src/internal/runtime/go_rng.rs` and
`crates/d2-tala-rs/src/internal/runtime/go_sort.rs` are translated from the Go 1.27.0
standard library's `math/rand` and `sort` packages.

`crates/d2-tala-rs/src/internal/placement/owner_median.rs` also follows
Go 1.27.0 `src/math/dim.go`, `dim_arm64.s` and `dim_amd64.s` for
`math.Max`, including architecture-specific NaN and signed-zero behavior.

`crates/d2-tala-rs/src/internal/routing/channel_points.rs` also translates
Go 1.27.0 `src/math/hypot.go` (the generic implementation used by the pinned
ARM64 oracle). The inherited RNG and sorting files now live under
`crates/d2-tala-rs/src/internal/runtime/`.

The graphjson decoder in `crates/d2-tala-rs/src/internal/graphjson/decode.rs`
uses Go 1.27.0 JSON field-update semantics and translates error formatting from
`src/encoding/json/v2_inject.go` and `v2_decode.go`.

The numeric parser in `crates/d2-rs-language/src/parser/numeric.rs` translates
Go 1.27.0 `src/math/big/ratconv.go` and `natconv.go` string-acceptance and
exponent rules.

The public parser in `crates/d2-rs-language/src/parse.rs` translates the
UTF-16LE decoder from `golang.org/x/text` v0.41.0
`encoding/unicode/unicode.go`. Diagnostic input quoting follows Go 1.27.0
`src/strconv/quote.go`.

D2 scalar floating-point parsing in `crates/d2-rs-core/src/go_float.rs`
translates Go 1.27.0 `src/internal/strconv/{atof,atoi}.go` lexical scanning,
underscore validation and hexadecimal sticky-bit rounding. Decimal conversion
uses Rust's correctly rounded parser after Go syntax validation. These
Go-derived portions retain the Go BSD terms below (Copyright 2009 The Go Authors).

The IR edge matching table in `crates/d2-rs-core/src/ir/fold.rs` is generated
from Go 1.27.0 `unicode.SimpleFold` and its Unicode 17.0.0 tables.

The substitution array backing model in
`crates/d2-rs-language/src/compiler/arrays.rs` translates Go 1.27.0
`src/runtime/slice.go`, `src/runtime/msize.go`, and the allocation constants in
`src/internal/runtime/gc/sizeclasses.go` and `malloc.go`. It preserves the
observable nested-append and range aliasing of D2's `[]Value` slices.

The native TALA container slice storage in
`crates/d2-tala-rs/src/internal/layoutgraph/container_slices.rs` and the typed
raw incident slice storage in
`crates/d2-tala-rs/src/internal/layoutgraph/owner_pointer_slices.rs` translate
Go 1.27.0 scanned-pointer growth and allocation size classes from
`src/runtime/slice.go`, `src/runtime/msize.go`, and
`src/internal/runtime/gc/{malloc,sizeclasses}.go`. Shared map/header storage
and exact capacity-window rollback follow the pinned D2 TALA source.

Native file handling in `crates/d2-rs-language/src/stream/native.rs` follows
Go 1.27.0 `src/os/file.go` and `src/internal/poll/fd_unix.go`. Generated
`stream/os_errors.rs` preserves the architecture-specific Go syscall errno
strings for Darwin/Linux ARM64/x86-64, under the Go BSD terms below.

HTML/URL classification in `crates/d2-rs-language/src/compiler/import_urls.rs`
and its generated entity table follow Go 1.27.0 `src/html/{escape,entity}.go`
and the reachable parsing/validation rules of `src/net/url/url.go`,
`gen_encoding_table.go`, and `src/net/netip/netip.go`. The byte path join in
`compiler/import_links.rs` follows `src/path/path.go`. These Go-derived
portions retain the Go BSD terms below.

The streaming parser in `crates/d2-rs-language/src/stream.rs` follows Go 1.27.0
`src/bufio/bufio.go` and `src/unicode/utf8/utf8.go`, plus `golang.org/x/text`
v0.41.0 `encoding/unicode/unicode.go` and `transform/transform.go`, for
buffering, BOM transitions, UTF-16 decoding and read-error behavior.

Public request/font admission in
`crates/d2-rs-wasm/src/public_compile_request.rs` follows D2's MPL-2.0
`d2js/d2wasm`, `d2renderers/d2fonts`, `lib/font` and `lib/textmeasure`
contracts. The font-collection header/error checks follow
`golang.org/x/image v0.45.0/font/sfnt/sfnt.go` (Copyright 2016 The Go
Authors); those Go-derived portions retain the Go BSD terms below.
The Rust validator does not embed a Go runtime or translate the WOFF encoder.

Native underline metrics in `crates/d2-rs-export/src/scanline.rs` follow
`golang.org/x/image v0.45.0/font/sfnt/sfnt.go` hhea metrics and signed Int26_6
scaling. These portions retain the Go BSD terms below.

Copyright 2013 The Go Authors. All rights reserved.
Copyright 2015 The Go Authors. All rights reserved.
Copyright 2016 The Go Authors. All rights reserved.
Copyright 2024 The Go Authors. All rights reserved.
Copyright 2025 The Go Authors. All rights reserved.
Copyright 2009 The Go Authors.
Copyright 2010 The Go Authors. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

- Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.
- Redistributions in binary form must reproduce the above copyright notice,
  this list of conditions and the following disclaimer in the documentation
  and/or other materials provided with the distribution.
- Neither the name of Google LLC nor the names of its contributors may be used
  to endorse or promote products derived from this software without specific
  prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS “AS IS”
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.

## Apache License 2.0 (Fibonacci heap)


                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
## Source Sans Pro embedded in SVG documentation

Copyright 2010 - 2018 Adobe Systems Incorporated (http://www.adobe.com/),
with Reserved Font Name 'Source'.

The checked-in D2 SVG diagrams embed subsets of Source Sans Pro. The font
metadata identifies SIL Open Font License 1.1; this applies to the embedded
font software, not to the diagrams as a whole.

SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font creation
efforts of academic and linguistic communities, and to provide a free and
open framework in which fonts may be shared and improved in partnership
with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply
to any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to a
new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed, modify,
redistribute, and sell modified and unmodified copies of the Font
Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components,
in Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the corresponding
Copyright Holder. This restriction only applies to the primary font name as
presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created
using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.

## Goldmark

Native Markdown text primitives derive from github.com/yuin/goldmark v1.8.5,
pinned by D2 v0.9.0. Rust translations retain the following MIT terms.
Translated source files are marked `MPL-2.0 AND MIT`: Nelson Chen's Rust
translation and modifications are MPL-2.0, while the corresponding Goldmark
material remains available under its original MIT license.

MIT License

Copyright (c) 2019 Yusuke Inuzuka

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## D2 command logging

The native CLI stderr line decoration follows `cmdlog.prefixWriter` in
`github.com/d2lang/util-go v0.2.0`, the dependency pinned by D2 v0.9.0.
This source reference retains its ISC license:

```text
Copyright (c) 2022 Terrastruct Inc. info@terrastruct.com

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

Noto Color Emoji COLRv1 2.051: pinned unmodified compressed asset from original Go D2 v0.9.0; SIL Open Font License 1.1. See crates/d2-rs-export/assets/NotoColorEmoji-OFL.txt and its README.md for provenance.

## Chroma code syntax and themes

The native Go and Go-template token rules, token splitting, and style inheritance
in `crates/d2-rs-text/src/code.rs` are translated from Chroma v2.27.0, the
dependency pinned by original Go D2 v0.9.0. Its `github.xml` and
`catppuccin-mocha.xml` code styles are preserved byte-for-byte in
`crates/d2-rs-text/assets/chroma/`. These portions retain Chroma's MIT terms:
Copyright (C) 2017 Alec Thomas. The complete original notice is preserved at
`crates/d2-rs-text/assets/chroma/COPYING`; its bundled font notice is retained
there as part of the original file, although no Chroma font bytes are imported.
Rust translation and modifications: Copyright 2026 Nelson Chen.

The pinned regexp2 v2.6.0 category behavior used by the translated rules is
identified by source comparison; its original MIT notice is preserved at
`crates/d2-rs-text/assets/chroma/COPYING-regexp2`. Native matching executes
through the Rust regex crate. Native Markdown/YAML and JavaScript/JSON use pinned
original `lexers/markdown.go`, `embedded/{yaml,javascript,json}.xml` assets and
source RegexLexer/Include/Push/Pop/UsingSelf/UsingByGroup execution. Native registry configuration and lookup follow original Chroma `registry.go`
and `lexer.go`; the pinned definition inventory is retained in
`assets/chroma/registry-config.json`. Regex features
use the pure Rust fancy-regex crate. Wider registry coverage remains open.

## csscolorparser v0.1.8 acceptance rules

Gradient-stop syntax validation in `crates/d2-rs-language/src/compiler/style_colors.rs`
translates reachable acceptance rules from the pinned original Go dependency
`github.com/mazznoer/csscolorparser v0.1.8/colorparser.go`. Color conversion is
not translated or claimed by this validator.

MIT License

Copyright (c) 2020 Nor Khasyatillah

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Go encoding/xml name ranges

The XML-name ranges in crates/d2-rs-export/src/go_xml_names.rs are translated
from the Go 1.27.0 toolchain used by the pinned D2 source. They retain the
Go Authors BSD license:

Copyright 2009 The Go Authors.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
   * Neither the name of Google LLC nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.


The BSD-3-Clause Go encoding/xml notice above also covers the translated
procInst lookup in crates/d2-rs-export/src/svg_import_envelope.rs:decoder_field
(Go 1.27.0). The other envelope admission rules derive from D2 v0.9.0 MPL-2.0
import.go and lib/svg/doctype.go.

The Go encoding/xml BSD-3-Clause notice also covers reachable lexical, entity,
character-range and element-stack translation in
crates/d2-rs-export/src/svg_asset_xml.rs (Go 1.27.0). D2 asset root/admission
rules retain MPL-2.0 mapping to lib/imageasset/probe.go.

## Brotli reader error compatibility

Native SVG codec reader boundary/error compatibility maps to github.com/andybalholm/brotli v1.2.2 reader.go and decode.go, used by pinned original D2 v0.9.0. The decoder itself executes the native Rust brotli-decompressor crate.

Copyright (c) 2009, 2010, 2013-2016 by the Brotli Authors.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.


Native host-font cmap admission in `crates/d2-rs-export/src/font_cmap.rs`
translates `golang.org/x/image v0.45.0/font/sfnt/cmap.go` and the cmap selection
in `sfnt.go`, plus `golang.org/x/text` Macintosh encoding data. These portions
retain the Go BSD-3-Clause terms reproduced above.
Copyright 2013 The Go Authors. All rights reserved.
Copyright 2017 The Go Authors. All rights reserved.

## go-text shaping source portions

Native `font_shaping.rs` follows pinned D2 fontface/shaping.go and go-text
/typesetting v0.3.4 script subdivision, paired delimiter data, zero-mark
advance and GPOS mark-anchor boundaries. The go-text-derived portions retain
the following BSD terms (the upstream also offers Unlicense):

The BSD 3 Clause License

Copyright 2021 The go-text authors

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

Native `colr_plan.rs` table layouts, first-match glyph lookup and Float1616
decoding follow go-text/typesetting v0.3.4 `font/opentype/tables` under the
BSD-3-Clause terms reproduced above. Copyright 2021 The go-text authors.

## golang.org/x/net HTML name folding

The native Markdown HTML name adapter in `markdown/html_names.rs` follows
`golang.org/x/net v0.57.0/html/token.go:Tokenizer.TagName/TagAttr`.
Copyright 2010 The Go Authors. All rights reserved.

Copyright 2009 The Go Authors.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
   * Neither the name of Google LLC nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.


Checkpoint494: the Go standard-library BSD3 notice above also covers native text/xml.rs lexical, stack and error-message validation and text/go_xml_names.rs XML-name ranges from pinned Go1.27 encoding/xml/xml.go. text/go_quote.rs reuses the repository Source strconv.Quote translation and Unicode17 printability generated from the same Go toolchain. Original Go Authors2009 copyright and source notices remain in these files. The raw compiler HTML projection corrections follow pinned Goldmark v1.8.5 renderer/html/html.go, util/util.go and extension/table.go under its retained MIT notice.

Local shaping dependency: vendor/rustybuzz-go-v090 contains Rustybuzz 0.20.1 under its retained MIT LICENSE. D2-specific lookup/digest changes follow pinned go-text/typesetting v0.3.4 HarfBuzz-derived code (Unlicense OR BSD-3-Clause; GO_TEXT_LICENSE retained); source mapping and modifications are recorded in vendor/rustybuzz-go-v090/SOURCE.md. Corresponding source is included in this repository.

Checkpoint498 Unicode lookup tables and source control data follow pinned go-text/typesetting v0.3.4 internal/unicodedata (Unlicense OR BSD-3-Clause), with its GO_TEXT_LICENSE retained in vendor/rustybuzz-go-v090. Modified HarfBuzz portions additionally retain the pinned Go harfbuzz MIT copyright/license in GO_HARFBUZZ_LICENSE. Local package metadata describes both source notices. Native Source table generation is scripts/generate-source-unicode.py; raw complete lookup evidence is tests/fixtures/go-unicode-v090.json in that local package.

Checkpoint524 additionally translates CD parsing/layout/delimiter assembly,
dimension conversion, font scope/identifier handling and recoverable error
metrics from pinned mathjax-go v0.1.0 (Copyright 2017–2022 The MathJax
Consortium, Apache-2.0). New calligraphic/italic generated tables and original
U+23D0/U+2016 glyph paths retain the existing MathJax font-data license.
The generated-error font-family style diagnostic follows original D2
v0.9.0 d2svgimport/style.go (MPL-2.0).

Checkpoint525 nested viewport/aspect/clip transport and grouped clip painting
translate original D2 v0.9.0 d2svgimport, d2scene/aspect.go and d2raster
source under MPL-2.0. Existing MathJax-generated glyph licenses and source
notices remain unchanged.

Checkpoint526 deferred delimiter paint, retained script/local label frames,
style and negative-depth changes translate pinned mathjax-go v0.1.0
internal/svg/{mo,mtable,msubsup,mpadded}.go and tex/environments.go.
Copyright 2017–2022 The MathJax Consortium; Apache-2.0. Existing original
D2 MPL-2.0 boundary/tests and font notices remain intact.

Checkpoint527 translates original mathjax-go v0.1.0 internal/tex/
environments.go splitTable/captureEnvironment/parseCDRow and parser.go
readControlSequence/readArgument: immediate optional row annotations,
matching nested environment capture and earliest recoverable errors.
Original MathJax Consortium Apache-2.0 attribution is retained.
Eleven source root/measurement/error contracts are permanent; complete
render evidence is separate in .reference/cd-capture-20261007/.
Full font/extension/error and broader D2 compatibility remain open.

Checkpoint528 translates original mathjax-go v0.1.0 tex/parser.go numeric
mn runs, svg/wrapper.go token IC/unknown-font/isCJK metrics and
svg/msubsup.go numeric script core semantics. MathJax Consortium Apache-2.0
attribution is retained. number_unicode.rs derives Go1.27.0 Unicode17
IsDigit ranges; Go Authors BSD-3-Clause notices above apply. The complete
72-range table is verified by scripts/check-math-number-unicode.py against
the pinned original parser oracle. Thirty-two measured/root contracts are
permanent. Evidence: .reference/numeric-tex-20261007/. This does not qualify
full fonts, extensions, errors, API or D2 parity; wider gates remain open.

Checkpoint529 translates original mathjax-go v0.1.0 layout/lengths.go
relative/absolute/size-dependent dimensions, tex/commands.go horizontal
spacing, tex/environments.go raw CD minimum-label kern, svg/wrapper.go
mspace/signed bbox Append and svg/typesetter.go final viewport clamps.
MathJax Consortium Apache-2.0 attribution is retained. Native Unicode
classification adds Go1.27.0 digit/letter-union ranges; existing Go Authors
BSD-3-Clause notices apply. The72/736 ranges are checked against the pinned
original parser oracle. Thirty-six root/measurement/error contracts are
permanent; strict image qualification retains two single-pixel failures.
Evidence: .reference/length-tex-20261007/. Wider compatibility remains open.

Checkpoint530 stages a native translation of pinned mathjax-go v0.1.0
internal/mhchem/texify.go (mhchemparser4.1.1). Copyright2015-2021 Martin
Hensel,2026 luo-studio and2026 erweixin; Apache-2.0 AND MIT. The MIT notice
is retained in crates/d2-rs-math/licenses/mhchem-go-tex-MIT.txt. Original
PROVENANCE.md maps the pinned parser and initial Go translation. All718
node/error texifier contracts are source-qualified and permanent. The
state machine and host-TeX integration remain pending; the staged component
is not yet used by ce/pu. Evidence: .reference/mhchem-texifier-20261007/.

Checkpoint531 translates pinned mathjax-go v0.1.0 internal/mhchem/jsregexp.go
and patterns.go into native Rust. Its exact data/patterns.json is retained;
eleven inline patterns are extracted unchanged from original patterns.go.
Copyright2015-2021 Martin Hensel,2026 luo-studio and2026 erweixin;
Apache-2.0 AND MIT; the existing mhchem-go-tex-MIT.txt notice applies.
15984 regex and25688 token/remainder/error source contracts are permanent.
Only the matching/output stages are translated; state-machine/actions and
host integration remain pending. Evidence: .reference/mhchem-matcher-20261007/.

Checkpoint532 translates complete pinned mathjax-go v0.1.0 internal/mhchem
state-machine driver,nullable buffers and all action handlers into Rust.
data/machines.json is copied exactly (14 machines,51 states,1277 transitions).
Copyright2015-2021 Martin Hensel,2026 luo-studio and2026 erweixin;
Apache-2.0 AND MIT; existing mhchem-go-tex-MIT.txt is retained.
All117 original mhchemparser4.1.1 assertions plus retained/exploratory/errors
qualify973 exact expansion/error contracts. Host-TeX integration remains
pending. Evidence: .reference/mhchem-engine-20261007/.

Checkpoint533 translates pinned mathjax-go v0.1.0 tex/commands.go phantom,
smash and lap commands,parser.go argument/bracket reading, svg/mpadded.go,
mphantom.go and transparent script-core H/D from msubsup.go. Root bounds
retain msqrt.go em arithmetic before scaling when smash exposes isolated D.
Copyright2017-2022 The MathJax Consortium; Apache-2.0 notices apply.
All304 fresh root/measure contracts are permanent; image qualification is
recorded separately. Chemistry host integration and wider parity remain open.
Evidence: .reference/padded-tex-20261007/.

Checkpoint533 additionally translates original default mtext glyph advances and
local frames from mathjax-go svg/wrapper.go: no mi italic correction is applied.
The24 added literal-text controls complete304 permanent root/measure contracts.
Final native image evidence and explicit failures are recorded in
.reference/padded-tex-20261007/qualification.json; wider compatibility stays open.

Checkpoint534 translates mathjax-go v0.1.0 tex/commands.go mathClass,
parser.go groups/arguments/scripts, tex/inheritance.go and
svg/texclass.go,texatom.go,msubsup.go,munderover.go,mo.go,wrapper.go into
native classes.rs/lib.rs. Source normal-font long arrows and the reachable
SmallOp/-tex-variant font subsets are generated from data_normal_gen.go,
data_smallop_gen.go,data_tex_variant_gen.go. Copyright2017-2022/2018-2022
The MathJax Consortium; Apache-2.0 notices and original font attribution apply.
All275 permanent original root/measure/glyph/error contracts and actual image
qualification are recorded in .reference/class-tex-20261007/.
Chemistry injection and broader compatibility remain pending.

Checkpoint548: native text/uniseg.rs and generated uniseg_data.rs translate pinned github.com/rivo/uniseg v0.4.7 line/grapheme/width rules (MIT, Copyright (c) 2019 Oliver Kuederle) and Unicode15.0.0 property payloads. Complete MIT and Unicode copyright/permission texts are retained in crates/d2-rs-text/licenses/UNISEG-LICENSE.txt and UNICODE-LICENSE.txt. Go1.27 Unicode17 symbol/mark categories retain the Go Authors2009 BSD3 notice and complete GO-LICENSE.txt in the same directory. Native text/measure.rs CSS fallback and render/markdown_wrap.rs follow original D2 v0.9.0 textmeasure algorithms under MPL-2.0. Reproduce ordered tables with scripts/generate-uniseg-boundaries.py and the pinned Go unicode/tables.go; no runtime oracle or generated fixture output is used.

Checkpoint675: `crates/d2-rs-render/src/image_mime.rs` and
`image_mime_data.rs` translate the ordered content-type sniffer and built-in
MIME extension table from pinned Go1.27.0 `net/http/internal/sniff.go` and
`mime/type.go`. Copyright 2010, 2011 The Go Authors. All rights reserved.
These portions retain the Go BSD-3-Clause license reproduced above. Regenerate
the tables with `scripts/generate-http-mime-tables.py`. System MIME database
registrations are not included in this bounded built-in translation.

Checkpoint676: `crates/d2-rs-render/src/image_mime_registry.rs` extends the
native MIME fallback with Go1.27.0 `mime/type.go`, `type_unix.go`, `grammar.go`
and reachable `mediatype.go` registration and parameter serialization behavior.
Copyright 2010 The Go Authors. All rights reserved. Go BSD-3-Clause terms above
apply. Unix host databases are read in the original source order; the native
algorithms do not use a Go process. Windows registry lookup and invalid UTF-8
host database contents remain unqualified. Source-derived registration controls
are stored in tests/fixtures/mime-registration-source-go127.json.

## Go reduced SVG transform trigonometry

`crates/d2-rs-math/src/svg_trig.rs` derives reduced-input Sincos/Tan from
Go 1.27.0 math/sincos.go, sin.go and tan.go. Copyright 2010 The Go Authors.
The Go BSD license reproduced above applies.

Cephes Math Library Release 2.8: June, 2000
Copyright 1984, 1987, 1989, 1992, 2000 by Stephen L. Moshier

The readme file at http://netlib.sandia.gov/cephes/ says:
Some software in this archive may be from the book _Methods and
Programs for Mathematical Functions_ (Prentice-Hall or Simon & Schuster
International, 1989) or from the Cephes Mathematical Library, a
commercial product. In either event, it is copyrighted by the author.
What you see here may be used freely but it comes with no support or
guarantee.

The two known misprints in the book are repaired here in the
source listings for the gamma function and the incomplete beta
integral.

Stephen L. Moshier
moshier@na-net.ornl.gov

## Native pinned CSS color parser

`crates/d2-rs-math/src/svg_color.rs` and `css_named_colors.json` derive from
csscolorparser v0.1.8, Copyright (c) 2020 Nor Khasyatillah.

```text
MIT License

Copyright (c) 2020 Nor Khasyatillah

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Native WebP configuration admission in `crates/d2-rs-math/src/svg_image_config.rs`
follows `golang.org/x/image` v0.45.0 `webp/decode.go`, `vp8/decode.go` and
`vp8l/decode.go` header admission. Copyright 2011 The Go Authors. All rights
reserved. The complete Go Authors BSD-3-Clause conditions and disclaimer above
apply to this translation.

Owned WebP YCbCr sixteen-bit color conversion in `crates/d2-rs-export/src/owned_png_scene.rs` follows pinned Go1.27 `image/color/ycbcr.go`. Copyright 2009 The Go Authors. All rights reserved. The Go BSD-3-Clause conditions and disclaimer reproduced above apply. VP8 decoding uses the unchanged image-webp dependency and its retained notices.

Native `crates/d2-rs-core/src/jpeg_idct.rs` follows Go1.27 `image/jpeg/dct.go`. Copyright 2025 The Go Authors. All rights reserved. The Go BSD-3-Clause conditions and disclaimer reproduced above apply.

Native `crates/d2-rs-core/src/jpeg_entropy.rs` follows Go1.27 `image/jpeg/huffman.go` and buffered entropy reading in `image/jpeg/reader.go`. Copyright 2009 The Go Authors. All rights reserved. The Go BSD-3-Clause conditions and disclaimer reproduced above apply.

Native `crates/d2-rs-core/src/jpeg_decode.rs` follows Go1.27 `image/jpeg/reader.go`, `scan.go`, and `image/color/ycbcr.go`, with the separately mapped inverse DCT and entropy reader. Copyright 2009, 2012, 2025 The Go Authors. All rights reserved. The Go BSD-3-Clause conditions and disclaimer reproduced above apply.

`crates/d2-rs-core/src/jpeg_first.rs` follows D2v0.9.0 rasterimage JPEG/contextReader behavior and the pinned Go1.27 bufio reader. Go bufio portions: Copyright 2009 The Go Authors. All rights reserved. The Go BSD-3-Clause conditions and disclaimer reproduced above apply. Functional JPEG input cancellation preserves the source reader result semantics.

## ELK SIMPLE Box packing

`crates/d2-rs-elk/src/box_layout.rs` translates SIMPLE sorting and row packing
from `github.com/d2lang/elk-go v0.2.0/internal/box/provider.go`, pinned by D2
v0.9.0. Copyright (c) 2009, 2020 Kiel University and others; Copyright (c)
2015, 2020 Kiel University and others. These portions retain EPL-2.0. The
complete license is reproduced in `crates/d2-rs-elk/ELK-EPL-2.0.txt`
(`ELK-EPL-2.0.txt` within the packaged ELK crate). Rust translation and
modifications: Copyright 2026 Nelson Chen. Source and reproduction bindings
are recorded in `.reference/elk-box-20261019/source-overlay.json`.

## ELK Fixed and utility-provider scope preparation

`crates/d2-rs-elk/src/fixed_layout.rs` and `utility.rs` translate reachable
parts of pinned `elk-go v0.2.0/internal/fixed/provider.go`,
`internal/common/elkutil/elkutil.go`, `internal/core/engine.go` and
`internal/geometry/geometry.go`. Copyright (c) 2010, 2019 and 2015, 2020
Kiel University and others (Fixed); Copyright (c) 2008, 2020 Kiel University
and others (ElkUtil); Copyright (c) 2008, 2018 Kiel University and others,
2018, 2020 TypeFox GmbH and others, and 2026 The elk-go Authors (engine);
Copyright (c) 2010, 2015; 2010, 2017; 2014, 2017; 2016 Kiel University and
others, and 2026 The elk-go Authors (geometry). These portions retain
EPL-2.0; complete terms are in `crates/d2-rs-elk/ELK-EPL-2.0.txt`
(`ELK-EPL-2.0.txt` in the packaged crate). Rust translation/modifications:
Copyright 2026 Nelson Chen. Source bindings: `.reference/elk-fixed-20261019/`.

## ELK default Rectpacking chain

`crates/d2-rs-elk/src/rectpacking/` translates greedy width estimation and
the default single-pass row/block compactor from pinned `elk-go v0.2.0`
`internal/rectpacking/provider.go` and `second_iteration.go`. Copyright (c)
2018, 2020 Kiel University and others; Copyright (c) 2020 Kiel University
and others; Copyright (c) 2026 The elk-go Authors. These portions retain
EPL-2.0; complete terms are in `crates/d2-rs-elk/ELK-EPL-2.0.txt`
(`ELK-EPL-2.0.txt` in the packaged crate). Rust translation/modifications:
Copyright 2026 Nelson Chen. Source bindings: `.reference/elk-rectpacking-20261019/`.

`rectpacking/micro.rs` also translates the pre/post-packing sizing and
label/port placement from pinned
`internal/common/nodespacing/node_label_and_size_calculator.go`: Copyright (c)
2014, 2015 Kiel University and others; Copyright (c) 2017, 2019 Kiel University
and others, under EPL-2.0. The complete terms and source bindings above apply.

## ELK Random provider

`crates/d2-rs-elk/src/random_layout.rs` translates pinned elk-go v0.2.0
`internal/random/provider.go`: Copyright (c) 2010, 2015 Kiel University and
others; Copyright (c) 2015, 2020 Kiel University and others, under EPL-2.0.
The shared Java RNG remains the source-derived implementation in `layered.rs`.
Complete terms are in `crates/d2-rs-elk/ELK-EPL-2.0.txt` (`ELK-EPL-2.0.txt`
in the packaged crate). Rust translation/modifications: Copyright 2026
Nelson Chen. Source and arithmetic evidence: `.reference/elk-random-20261019/`.

## ELK Rectpacking option processors and option registry

The Rectpacking translation additionally covers the option-driven provider
processors, repeated row/block compaction and whitespace expansion in pinned
elk-go v0.2.0 `internal/rectpacking/{provider,second_iteration,options}.go`.
`utility.rs` and `fixtures/original_option_ids.txt` derive suffix resolution
and registered identifiers from `internal/core/registry.go` and the built-in
registrations, including `internal/builtins/frozen_options.go`. Copyright (c)
2012, 2015; 2015, 2020; 2016, 2020 Kiel University and others, and 2026 The
elk-go Authors; EPL-2.0. Existing Rectpacking notices also apply. Complete
terms: `crates/d2-rs-elk/ELK-EPL-2.0.txt` (packaged as `ELK-EPL-2.0.txt`).
Rust modifications: Copyright 2026 Nelson Chen. Source/evidence bindings:
`.reference/elk-rectpacking-options-20261019/`.


## ELK Force and Stress providers

`crates/d2-rs-elk/src/force_layout/` translates elk-go v0.2.0
`internal/force/{graph,importer,components,provider,model,options}.go` and
`internal/stress/{provider,majorization,options}.go`: particle import, component
splitting/recombination, Fruchterman-Reingold and Eades force models, Java-shaped
priority-queue Dijkstra, and localized stress majorization. Copyright (c)
2008–2020 Kiel University and others; EPL-2.0. Native integration/tests:
Copyright 2026 Nelson Chen. Complete license: `crates/d2-rs-elk/ELK-EPL-2.0.txt`.
Source bindings and explicit unresolved timeout witnesses:
`.reference/elk-stress-followup-20261019/`.

`utility.rs` also follows elk-go `internal/core/engine.go` receiver-local
algorithm resolution. `layered.rs` follows the sole-component transfer branch
of `internal/layered/components/simple_row_graph_placer.go`; existing Layered
notices continue to apply.


The Force Eades logarithm translates Go1.27 `src/math/{log,frexp}.go`,
Copyright2009 The Go Authors, under BSD-3-Clause (`GO-MATH-LICENSE.txt` in
the ELK crate). Its constants/algorithm retain the notice: Copyright(C)1993
Sun Microsystems, Inc. All rights reserved. Developed at SunPro, a Sun
Microsystems, Inc. business. Permission to use, copy, modify, and distribute
this software is freely granted, provided that this notice is preserved.
Arm64 fused evaluation follows the pinned compiler's disassembly; scalar
qualification covers the current arm64 host, not every platform.


## ELK Radial provider and numeric routines

`crates/d2-rs-elk/src/radial_layout/` translates elk-go v0.2.0
`internal/radial/{provider,options}.go`, including original wedge allocation,
ordering, overlap removal, compactors, optimization, rotation, sizing and routes.
Copyright(c)2017 and2017–2020 Kiel University and others; EPL-2.0. Its node
margin calculation follows `internal/common/nodespacing/node_margin_calculator.go`,
Copyright(c)2010–2020 Kiel University and others. Complete ELK license:
`crates/d2-rs-elk/ELK-EPL-2.0.txt`.

`js_trig.rs` translates elk-go `internal/javacompat/jsmath.go`: fdlibm-derived
kernels, Copyright(C)1993 Sun Microsystems, Inc. All rights reserved. Developed
at SunSoft, a Sun Microsystems, Inc. business. Permission to use, copy, modify,
and distribute this software is freely granted, provided that this notice is
preserved. The original source was modified significantly by Google Inc.;
Copyright2016 the V8 project authors, Copyright(c)2026 The elk-go Authors.
V8 BSD-3-Clause license: `crates/d2-rs-elk/V8-BSD-3-Clause.txt`.

`go_trig.rs` and `go_math.rs` translate pinned Go1.27
`src/math/{sin,trig_reduce,atan,atan2,asin,hypot}.go`: Copyright2009,2010,2011,2018
The Go Authors; BSD-3-Clause (`crates/d2-rs-elk/GO-MATH-LICENSE.txt`). Cephes-derived
portions retain Stephen L.Moshier's1984,1987,1989,1992,2000 copyright and permission
notice in `crates/d2-rs-elk/GO-TRIG-NOTICE.txt`. Native integration and tests:
Copyright2026 Nelson Chen. Source mappings and qualification:
`.reference/elk-radial-20261019/`. Scalar bit qualification is for the arm64 host;
other architectures and WASM remain separately qualified work.

## ELK Mr. Tree provider

`crates/d2-rs-elk/src/mrtree/` translates pinned elk-go v0.2.0
`internal/mrtree/{model,options,provider}.go`: source component identity,
cycle-edge removal from placement incidence, dummy roots, fan/constraint ordering,
Walker placement, direction transforms, scan-line compaction, cycle/multilevel
routing, source packing and wire section transfer.

Copyright (c) 2013, 2015, 2017, 2020 Kiel University and others.
These portions retain EPL-2.0; see `EPL-2.0.txt`. Rust additions and modifications:
Copyright 2026 Nelson Chen, MPL-2.0. Original Go geometry/error witnesses,
source mapping and bounded qualification are recorded in
`.reference/elk-mrtree-20261019/`. Original source fatal stack-overflow cases
remain explicit unqualified failures; no complete raw-JSON/platform claim follows
from geometry comparisons.

## ELK SPoRE providers and Go runtime random primitives

`crates/d2-rs-elk/src/spore/` translates pinned elk-go v0.2.0
`internal/spore/` providers, importer, rectangle geometry, Delaunay triangulation,
ordered spanning trees, scanline overlap detection and tree growth/compaction.
Copyright (c) 2017 Kiel University and others; Copyright (c) 2026 The elk-go
Authors. These portions retain EPL-2.0. Source mapping, exact geometry/error
controls and explicitly nondeterministic cases: `.reference/elk-spore-20261019/`.

`spore/random.rs` translates Go1.27 internal/chacha8rand state/block functions
and math/rand's Int63/Float64 conversion. Copyright2023 The Go Authors.
BSD-3-Clause terms are in GO-MATH-LICENSE.txt; source attribution is in
GO-CHACHA8-NOTICE.txt. Native modifications: Copyright2026 Nelson Chen, MPL-2.0.
Seeded integer/float controls do not imply matching independent unseeded
processes or qualification of non-Unix entropy/WASM/platform behavior.

## rough-go v0.2.0 / Rough.js

Polygon path generation and zigzag scan-line filling are translated from the pinned D2 dependency
`github.com/d2lang/rough-go v0.2.0`, generator.go, renderer.go,
filler_zigzag.go, filler_scan_line_hachure.go and geometry.go.
`crates/d2-rs-sketch/src/generator.rs` translates the same module's core.go,
generator.go, renderer.go, path.go, number.go, math.go, geometry.go and
filler*.go (Rough.js 4.6.6, hachure-fill 0.5.2 and path-data-parser 0.1.0
derived), used by the d2sketch SVG port in
`crates/d2-rs-render/src/d2svg/sketch.rs`.

MIT License

Copyright (c) 2019 Preet Shihn
Copyright (c) 2020 Preet Shihn
Copyright (c) 2023 Preet Shihn
Copyright (c) 2026 rough-go contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## go-quantize median cut

`crates/d2-rs-export/src/gif_quantize.rs` translates palette bucket construction,
median partitioning and mode selection from `github.com/ericpauley/go-quantize`
revision `ae555eb2afa4` (D2 v0.9.0's pinned dependency). The D2 xgif glue retains
MPL-2.0 terms; these translated quantizer portions retain BSD-3-Clause:

```text
BSD 3-Clause License

Copyright (c) 2017, Eric Pauley
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this
  list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice,
  this list of conditions and the following disclaimer in the documentation
  and/or other materials provided with the distribution.

* Neither the name of the copyright holder nor the names of its
  contributors may be used to endorse or promote products derived from
  this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Go-PDF / FPDF document and UTF-8 font serialization

`crates/d2-rs-export/src/pdf_font.rs` and `pdf_document.rs` translate the UTF-8
font writer, CID width grouping, page streams, IDs and document/resource writer
from `codeberg.org/go-pdf/fpdf v0.12.0`, pinned by D2
v0.9.0. These translated portions retain the upstream MIT license:

```text
MIT License

Copyright (c) 2021 The Go-PDF authors
Copyright (c) 2020 David Barnes
Copyright (c) 2017 Kurt Jung and contributors acknowledged in the documentation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Run 53 shared SVG acquisition: the Go encoding/xml lexical/name/envelope and
Brotli reader compatibility translations formerly in d2-rs-export now live
in `crates/d2-rs-math/src/image_svg/`. Their original SPDX headers and full
BSD-3-Clause/MIT notices are preserved in d2-rs-math/THIRD_PARTY_NOTICES.md
and the exporter notice history. No third-party terms changed.

## Board output Unicode keys

`crates/d2-rs-cli/src/board_output/unicode_fold.rs` is generated from pinned
`golang.org/x/text` v0.41.0 `cases.Fold` and `unicode/norm` properties
(Unicode 17.0.0). Copyright 2009 The Go Authors. The Go BSD-3-Clause terms
above apply. Native NFC uses unicode-normalization 0.1.25; its complete
notices are in DEPENDENCY_NOTICES.md.

## Watch authorization parsing

`crates/d2-rs-cli/src/watch.rs` translates reachable cookie/query parsing and
redirect escaping from Go 1.27.0 `src/net/http/{cookie,server}.go` and
`src/net/url/{url,gen_encoding_table}.go`. Copyright 2009, 2025 The Go
Authors. All rights reserved. The complete Go BSD-3-Clause terms above apply.
