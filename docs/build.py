#!/usr/bin/env python3
"""Build the Maestro documentation artefacts.

  docs/TUI-DESIGN.md   --md2latex-->  TUI-DESIGN.tex  --xelatex-->  TUI-DESIGN.pdf

docs/architecture.png is NOT built here: it is exported by hand from
docs/architecture.drawio (draw.io > File > Export as > PNG). This script used
to regenerate it from a TikZ source and would silently overwrite that export.

Requires MiKTeX (xelatex) on PATH. Run from anywhere:
    python docs/build.py
"""
import os
import shutil
import subprocess
import sys

DOCS = os.path.dirname(os.path.abspath(__file__))
AUX = (".aux", ".log", ".out", ".toc", ".synctex.gz")


def run(cmd, **kw):
    print("  $", " ".join(cmd))
    result = subprocess.run(cmd, cwd=DOCS, capture_output=True, text=True, encoding="utf-8", errors="replace", **kw)
    if result.returncode != 0:
        tail = "\n".join((result.stdout or "").splitlines()[-15:])
        print(tail, file=sys.stderr)
        raise SystemExit(f"failed: {' '.join(cmd)}")
    return result


def require(tool):
    if shutil.which(tool) is None:
        raise SystemExit(f"{tool} not found on PATH (install MiKTeX)")


def clean_aux():
    for name in os.listdir(DOCS):
        if name.endswith(AUX):
            os.remove(os.path.join(DOCS, name))


def main():
    require("xelatex")

    print("design document:")
    run([sys.executable, "md2latex.py", "TUI-DESIGN.md", "TUI-DESIGN.tex"])
    for _ in range(2):  # second pass resolves the table of contents
        run(["xelatex", "-interaction=nonstopmode", "TUI-DESIGN.tex"])

    clean_aux()
    print("\nartefacts:")
    for name in ("TUI-DESIGN.pdf",):
        path = os.path.join(DOCS, name)
        print(f"  {name:<20} {os.path.getsize(path) // 1024} KB")
    print("  architecture.png     exported by hand from architecture.drawio")


if __name__ == "__main__":
    main()
