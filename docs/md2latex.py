#!/usr/bin/env python3
"""md -> LaTeX converter for Maestro docs (MiKTeX/xelatex build).

Handles the Markdown subset used by Maestro's docs: headings, fenced code
blocks (ASCII wireframes), tables, nested lists, bold/italic, inline code.
Sanitizes Unicode glyphs that standard Windows fonts lack.
"""
import re
import sys

# Glyphs missing from Georgia/Consolas -> ASCII stand-ins
REPL = {
    "\u2753": "[?]", "\u26a0": "[!]", "\u25cf": "(*)", "\U0001f916": "[AI]",
    "\U0001f527": "[tool]", "\U0001f4ac": "[say]", "\U0001f4ce": "[clip]",
    "\u23f3": "[~]", "\u2705": "[ok]", "\u274c": "[x]", "\U0001f4a1": "[i]",
    "\u25b6": ">", "\u23f8": "||", "\u25c0": "<", "\u2593": "#", "\u2588": "#",
    "\u2587": "#", "\u2586": "=", "\u2585": "=", "\u2583": "_", "\u2501": "-",
    "\u2503": "|", "\u250f": "+", "\u2513": "+", "\u2517": "+", "\u251b": "+",
    "\u2533": "+", "\u252b": "+", "\u2523": "+", "\u253b": "+", "\u25f7": "o",
    "\u25cc": "o", "\u23ef": "[>]", "\u2192": "->", "\u2194": "<->",
    "\u21c4": "=>", "\u23f1": "[time]", "\u2601": "cloud", "\u2699": "[set]",
    "\u26c1": "local", "\U0001f4dc": "[log]",
}

ESCAPES = [("\\", r"\textbackslash{}"), ("&", r"\&"), ("%", r"\%"), ("$", r"\$"),
           ("#", r"\#"), ("_", r"\_"), ("{", r"\{"), ("}", r"\}"),
           ("~", r"\textasciitilde{}"), ("^", r"\textasciicircum{}")]


def sanitize(text: str) -> str:
    for k, v in REPL.items():
        text = text.replace(k, v)
    return text


def esc(text: str) -> str:
    for k, v in ESCAPES:
        text = text.replace(k, v)
    return text


def inline(text: str) -> str:
    """Inline markdown -> LaTeX, protecting code spans from escaping rules."""
    out = []
    for part in re.split(r"(`[^`]*`)", text):
        if len(part) >= 2 and part.startswith("`") and part.endswith("`"):
            out.append(r"\texttt{" + esc(sanitize(part[1:-1])) + "}")
            continue
        s = esc(sanitize(part))
        s = re.sub(r"\*\*(.+?)\*\*", r"\\textbf{\1}", s)
        s = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"\\emph{\1}", s)
        out.append(s)
    return "".join(out)


def convert(md: str) -> str:
    lines = md.split("\n")
    out, para = [], []
    list_depth = 0          # 0 = not in a list
    in_code = False
    pending_fig = None      # figure name from a <!-- tikz: name --> marker
    i = 0

    def flush_para():
        nonlocal para
        if para:
            out.append(inline(" ".join(para)) + "\n\n")
            para = []

    def close_lists(target=0):
        nonlocal list_depth
        while list_depth > target:
            out.append("  " * (list_depth - 1) + "\\end{itemize}\n")
            list_depth -= 1

    while i < len(lines):
        raw = lines[i]
        stripped = raw.strip()

        # ---- figure marker: <!-- tikz: name --> replaces the next code fence ----
        m = re.match(r"^<!--\s*tikz:\s*([\w.-]+)\s*-->$", stripped)
        if m:
            pending_fig = m.group(1)
            i += 1
            continue

        # ---- fenced code: either a TikZ figure or a literal block ----
        if stripped.startswith("```"):
            flush_para(); close_lists()
            if not in_code and pending_fig:
                # swallow the ASCII mock-up, emit the vector figure instead
                i += 1
                while i < len(lines) and not lines[i].strip().startswith("```"):
                    i += 1
                out.append("\\begin{center}\n\\input{tikz/" + pending_fig + ".tex}\n\\end{center}\n\n")
                pending_fig = None
                i += 1
                continue
            out.append("\\end{lstlisting}\n" if in_code else "\\begin{lstlisting}\n")
            in_code = not in_code
            i += 1
            continue
        if in_code:
            out.append(sanitize(raw) + "\n")
            i += 1
            continue

        # ---- headings (## -> section, since the H1 title is the doc title) ----
        m = re.match(r"^(#{2,4})\s+(.*)$", stripped)
        if m:
            flush_para(); close_lists()
            cmd = {2: "\\section", 3: "\\subsection", 4: "\\subsubsection"}[len(m.group(1))]
            out.append(f"{cmd}{{{inline(m.group(2))}}}\n")
            i += 1
            continue

        # ---- tables -> tabularx (wrapping columns) ----
        if stripped.startswith("|") and stripped.endswith("|"):
            flush_para(); close_lists()
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append(lines[i].strip())
                i += 1
            rows = [r for r in rows if not re.match(r"^\|[\s\-:|]+\|$", r)]
            if rows:
                ncols = max(len(r.strip("|").split("|")) for r in rows)
                spec = "l" + "X" * (ncols - 1) if ncols > 1 else "X"
                out.append("\\begin{small}\n\\begin{tabularx}{\\linewidth}{" + spec + "}\n\\toprule\n")
                for idx, r in enumerate(rows):
                    cells = [inline(c.strip()) for c in r.strip("|").split("|")]
                    cells += [""] * (ncols - len(cells))
                    out.append(" & ".join(cells) + " \\\\\n")
                    if idx == 0:
                        out.append("\\midrule\n")
                out.append("\\bottomrule\n\\end{tabularx}\n\\end{small}\n\n")
            continue

        # ---- horizontal rule ----
        if re.match(r"^-{3,}$", stripped):
            flush_para(); close_lists()
            i += 1
            continue

        # ---- lists (nesting by leading whitespace) ----
        m = re.match(r"^(\s*)(?:[-*]|\d+\.)\s+(.*)$", raw)
        if m:
            flush_para()
            depth = len(m.group(1)) // 2 + 1
            while list_depth < depth:
                out.append("  " * list_depth + "\\begin{itemize}\n")
                list_depth += 1
            close_lists(depth)
            out.append("  " * list_depth + "\\item " + inline(m.group(2)) + "\n")
            i += 1
            continue

        if not stripped:
            flush_para(); close_lists()
            i += 1
            continue

        para.append(stripped)
        i += 1

    flush_para(); close_lists()
    return "".join(out)


PREAMBLE = r"""\documentclass[10pt,a4paper]{article}
\usepackage[a4paper,margin=1.7cm]{geometry}
\usepackage{fontspec}
\usepackage{booktabs}
\usepackage{tabularx}
\usepackage{xcolor}
\usepackage{fancyhdr}
\usepackage{listings}
\usepackage{titlesec}
\usepackage{tikz}
\usepackage{hyperref}
\input{tikz/style.tex}
\hypersetup{colorlinks=true, linkcolor=black, urlcolor=blue!60!black}
\setmainfont{Georgia}
\setmonofont{Consolas}
% the document carries its own section numbers (1., 3.1, ...)
\setcounter{secnumdepth}{0}
\setcounter{tocdepth}{2}
% wireframes: never wrap, fixed columns, small enough to fit 100 cols
\lstset{
  basicstyle=\scriptsize\ttfamily,
  breaklines=false,
  columns=fixed,
  keepspaces=true,
  frame=single,
  framesep=3pt,
  xleftmargin=2pt,
  backgroundcolor=\color{gray!7},
  aboveskip=8pt, belowskip=8pt,
}
\titleformat{\section}{\large\bfseries}{}{0pt}{}
\titleformat{\subsection}{\normalsize\bfseries}{}{0pt}{}
\titleformat{\subsubsection}{\small\bfseries}{}{0pt}{}
\setlength{\parskip}{4pt}
\setlength{\parindent}{0pt}
\pagestyle{fancy}
\fancyhf{}
\fancyhead[L]{\small Maestro --- TUI Design}
\fancyhead[R]{\small\thepage}
\renewcommand{\headrulewidth}{0.4pt}
\title{\textbf{Maestro --- TUI Design}\\[2pt]
\large Wireframes, interaction flows and settings inventory}
\author{}
\date{}
\begin{document}
\maketitle
\thispagestyle{empty}
{\small\tableofcontents}
\clearpage
"""


def main():
    src, dst = sys.argv[1], sys.argv[2]
    with open(src, encoding="utf-8") as f:
        md = f.read()
    md = re.sub(r"^# .*?\n", "", md, count=1)          # H1 becomes the title page
    md = re.sub(r"^> .*?\n", "", md, count=1, flags=re.M)  # drop the PRD companion note
    with open(dst, "w", encoding="utf-8", newline="\n") as f:
        f.write(PREAMBLE + convert(md) + "\\end{document}\n")
    print(f"wrote {dst}")


if __name__ == "__main__":
    main()
