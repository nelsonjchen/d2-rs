// Copyright (c) 2009-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned mathjax-go 94d80186bfd7 internal/tex/symbols.go simpleMacros.
pub(super) fn body(name: &str) -> Option<&'static str> {
    Some(match name {
        " " => "\\text{ }",
        "qc" => "\\qqtext*{,}",
        "qcomma" => "\\qqtext*{,}",
        "pmqty" => "\\mqty(#1)",
        "Pmqty" => "\\mqty*(#1)",
        "bmqty" => "\\mqty[#1]",
        "vmqty" => "\\mqty|#1|",
        "spmqty" => "\\smqty(#1)",
        "sPmqty" => "\\smqty*(#1)",
        "sbmqty" => "\\smqty[#1]",
        "svmqty" => "\\smqty|#1|",
        "varliminf" => "\\mathop{\\underline{\\mmlToken{mi}{lim}}}",
        "varlimsup" => "\\mathop{\\overline{\\mmlToken{mi}{lim}}}",
        "varinjlim" => "\\mathop{\\underrightarrow{\\mmlToken{mi}{lim}}}",
        "varprojlim" => "\\mathop{\\underleftarrow{\\mmlToken{mi}{lim}}}",
        "boxed" => "\\fbox{$\\displaystyle{#1}$}",
        "Residue" => "\\mathrm{Res}",
        "mathstrut" => "\\vphantom{(}",
        "pmb" => "\\rlap{#1}\\kern1px{#1}",
        "TeX" => "T\\kern-.14em\\lower.5ex{E}\\kern-.115em X",
        "LaTeX" => "L\\kern-.325em\\raise.21em{\\scriptstyle{A}}\\kern-.17em\\TeX",
        "skew" => "{{#2{#3\\mkern#1mu}\\mkern-#1mu}{}}",
        "flatfrac" => "\\left.#1\\middle/#2\\right.",
        "stackrel" => "\\mathrel{\\mathop{#2}\\limits^{#1}}",
        "stackbin" => "\\mathbin{\\mathop{#2}\\limits^{#1}}",
        "pmod" => "\\pod{\\mmlToken{mi}{mod}\\kern 6mu #1}",
        "pod" => "\\mathchoice{\\kern18mu}{\\kern8mu}{\\kern8mu}{\\kern8mu}(#1)",
        "mod" => {
            "\\mathchoice{\\kern18mu}{\\kern12mu}{\\kern12mu}{\\kern12mu}\\mmlToken{mi}{mod}\\,\\,#1"
        }
        "bmod" => "\\mmlToken{mo}[lspace=\"thickmathspace\" rspace=\"thickmathspace\"]{mod}",
        "iff" => "\\;\\Longleftrightarrow\\;",
        "implies" => "\\;\\Longrightarrow\\;",
        "impliedby" => "\\;\\Longleftarrow\\;",
        "substack" => "\\begin{subarray}{c}#1\\end{subarray}",
        "Bra" => "{\\left\\langle {#1} \\right\\vert}",
        "Ket" => "{\\left\\vert {#1} \\right\\rangle}",
        "Ketbra" => "{\\left\\vert {#1} \\right\\rangle\\left\\langle {#2} \\right\\vert}",
        "crampedsubstack" => "\\begin{crampedsubarray}{c}#1\\end{crampedsubarray}",
        "bigtimes" => "\\mathop{\\Large\\kern-.1em\\boldsymbol{\\times}\\kern-.1em}",
        "matrixdeterminant" => "\\vmqty{#1}",
        "mdet" => "\\vmqty{#1}",
        "smdet" => "\\svmqty{#1}",
        "longrightleftharpoons" => {
            "\\stackrel{\\textstyle{-}\\!\\!{\\rightharpoonup}}{\\smash{{\\leftharpoondown}\\!\\!{-}}}"
        }
        "longRightleftharpoons" => {
            "\\stackrel{\\textstyle{-}\\!\\!{\\rightharpoonup}}{\\smash{\\leftharpoondown}}"
        }
        "longLeftrightharpoons" => {
            "\\stackrel{\\textstyle\\vphantom{{-}}{\\rightharpoonup}}{\\smash{{\\leftharpoondown}\\!\\!{-}}}"
        }
        "longleftrightarrows" => {
            "\\stackrel{\\longrightarrow}{\\smash{\\longleftarrow}\\Rule{0px}{.25em}{0px}}"
        }
        "tripledash" => {
            "\\vphantom{-}\\raise2mu{\\kern2mu\\tiny\\text{-}\\kern1mu\\text{-}\\kern1mu\\text{-}\\kern2mu}"
        }
        // parser.go newParseState installs Mathtools' optional Macro too.
        "MoveEqLeft" => "\\hspace{#1em}&\\hspace{-#1em}",
        _ => return None,
    })
}
pub(super) fn optional_default(name: &str) -> Option<&'static str> {
    (name == "MoveEqLeft").then_some("2")
}
