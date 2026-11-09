// SPDX-License-Identifier: MPL-2.0
// Template from pinned d2cli/help.go.
r###"{program} {version}
Usage:
  {program} [--watch=false] [--theme=0] file.d2 [file.svg | file.png | file.pdf | file.pptx | file.gif | file.txt]
  {program} layout [name]
  {program} fmt file.d2 ...
  {program} play [--theme=0] [--sketch] file.d2
  {program} validate file.d2

{program} compiles and renders file.d2 to file.svg | file.png | file.pdf | file.pptx | file.gif | file.txt
It defaults to file.svg if an output path is not provided.
Use --isometric for SVG, PNG, GIF, PDF or PPTX output with the native isometric renderer:
  {program} --isometric file.d2 file.svg
  {program} --isometric file.d2 file.png
  {program} --isometric file.d2 file.gif
  {program} --isometric file.d2 file.pdf
  {program} --isometric file.d2 file.pptx
You can also set D2_ISOMETRIC=true or vars: {d2-config: {isometric: true}} in D2 source.
Flags override environment variables, which override source configuration; --isometric=false disables it.
Isometric images preserve the compiled layout and render entirely in Go; no browser or GPU is required.
Images fit the diagram within 1600x1000 pixels (1000x625 for GIF); --scale changes those maximum dimensions.
Multiple boards export to SVG or PNG directories, PDF pages, PPTX slides or one GIF slideshow.
PDF and PPTX retain board navigation and clickable links. --animate-interval sets the GIF slide delay.
Use --target to select a board, or --target='' for the root without its subboards.

Use - to have d2 read from stdin or write to stdout.

PNG exports support up to 32768 pixels per dimension, subject to rendering resource limits.

See man d2 for more detailed docs.

Flags:
{flags}

Subcommands:
  {program} layout - Lists available layout engine options with short help
  {program} layout [name] - Display long help for a particular layout engine, including its configuration options
  {program} themes - Lists available themes
  {program} fmt file.d2 ... - Format passed files
	{program} play file.d2 - Opens the file in playground, an online web viewer (https://play.d2lang.com)
  {program} validate file.d2  - Validates file.d2

See more docs and the source code at https://github.com/d2lang/d2.
Hosted icons at https://icons.d2lang.com.
Playground runner at https://play.d2lang.com.
"###
