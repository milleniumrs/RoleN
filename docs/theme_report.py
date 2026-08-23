#!/usr/bin/env python3
"""Report which background colours each RoleN theme actually paints.

Runs the offscreen renderer (examples/theme_dump.rs) for every theme and
counts the ANSI background codes of the rendered cells, so "is this theme
light or dark?" is answered with data instead of a guess.

Usage: python docs/theme_report.py [theme ...]
"""
import re
import subprocess
import sys
from collections import Counter

BG = {
    40: ("black", "dark"), 41: ("dark red", "dark"), 42: ("dark green", "dark"),
    43: ("olive", "dark"), 44: ("dark blue", "dark"), 45: ("magenta", "dark"),
    46: ("teal", "dark"), 47: ("silver", "light"), 49: ("default", "dark"),
    100: ("gray", "dark"), 101: ("red", "light"), 102: ("green", "light"),
    103: ("yellow", "light"), 104: ("blue", "light"), 105: ("pink", "light"),
    106: ("aqua", "light"), 107: ("white", "light"),
}
THEME_SRC = "rolen-tui/src/theme.rs"
CELL = re.compile(r"\x1b\[(\d+(?:;\d+)*)m")


def known_themes():
    """Read the theme list from AVAILABLE so this can never drift."""
    src = open(THEME_SRC, encoding="utf-8").read()
    block = src.split("pub const AVAILABLE", 1)[1].split("];", 1)[0]
    return re.findall(r'\("([\w-]+)",', block)


def render(theme: str) -> str:
    out = subprocess.run(
        ["cargo", "run", "-q", "-p", "rolen-tui", "--example", "theme_dump", "--", theme, "100", "26"],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    return out.stdout


def analyse(dump: str):
    counts = Counter()
    current = None
    for code in CELL.findall(dump):
        for part in code.split(";"):
            n = int(part)
            if n in BG:
                current = n
            elif n == 0:
                current = None
        if current is not None:
            counts[current] += 1
    return counts


def main():
    themes = sys.argv[1:] or known_themes()
    print(f"{'theme':<12} {'dominant background':<26} {'share':>6}  {'kind':<6} runners-up")
    print("-" * 88)
    for theme in themes:
        counts = analyse(render(theme))
        if not counts:
            print(f"{theme:<12} (no output)")
            continue
        total = sum(counts.values())
        ranked = counts.most_common(4)
        top, n = ranked[0]
        name, kind = BG[top]
        rest = ", ".join(f"{BG[c][0]} {v * 100 // total}%" for c, v in ranked[1:])
        print(f"{theme:<12} {name:<26} {n * 100 // total:>5}%  {kind:<6} {rest}")


if __name__ == "__main__":
    main()
