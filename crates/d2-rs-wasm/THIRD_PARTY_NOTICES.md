# Third-party notices

d2-rs's first-party code is licensed under MPL-2.0. Third-party portions
retain the terms below; dependency notices are in the repository root's DEPENDENCY_NOTICES.md.

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
under the Apache License, Version 2.0. The Rust layout and parser around those
tables are first-party MPL-2.0 code; the generated glyph metrics and outlines
retain Apache-2.0 terms. The pinned module's `LICENSE`, `NOTICE`, and
`PROVENANCE.md` are the corresponding source and attribution records.

Apache License 2.0:
https://www.apache.org/licenses/LICENSE-2.0

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

The URL decoder in `crates/d2-rs-wasm/src/url_codec.rs` translates Go1.27.0
`encoding/base64` decodeQuantum and `compress/flate` Huffman/bit-reader rules.
Copyright 2009 The Go Authors. The Go BSD terms below apply.

The WASM host-value adapter in `crates/d2-rs-wasm/src/go-js-string-args.cjs`
follows Go 1.27.0 `syscall/js.Value.String` and `wasm_exec.js` value
marshaling. Copyright 2018 The Go Authors. The Go BSD terms below apply.

Portions of `crates/d2-tala-rs/src/internal/runtime/go_rng.rs` and
`crates/d2-tala-rs/src/internal/runtime/go_sort.rs` are translated from the Go 1.27.0
standard library's `math/rand` and `sort` packages.

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

The IR edge matching table in `crates/d2-rs-core/src/ir/fold.rs` is generated
from Go 1.27.0 `unicode.SimpleFold` and its Unicode 17.0.0 tables.

The substitution array backing model in
`crates/d2-rs-language/src/compiler/arrays.rs` translates Go 1.27.0
`src/runtime/slice.go`, `src/runtime/msize.go`, and the allocation constants in
`src/internal/runtime/gc/sizeclasses.go` and `malloc.go`. It preserves the
observable nested-append and range aliasing of D2's `[]Value` slices.

The native TALA container slice storage in
`crates/d2-tala-rs/src/internal/layoutgraph/container_slices.rs` translates
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

Copyright 2013 The Go Authors. All rights reserved.
Copyright 2015 The Go Authors. All rights reserved.
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
