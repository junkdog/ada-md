"""Cells that always hit their cap, skipped while the tool is still the version that hit it: rerunning them costs
minutes and records only "> cap". A new version runs them again (with a note on stderr), so a fix shows up by
itself; drop or update its entry once it finishes in time. B2_TIMEOUTS=off runs everything.

    python3 -I bench/b2/timeouts.py b2p   # "input mode tool" per skipped cell, for b2p.zsh

mkspec.py leaves skipped interactive jobs out (cells "viewer-doc"); doctables.py shows a skipped b2p cell as
killed, "> cap", when no directory in B2P_DIRS has a record of it."""
import os, subprocess, sys

B = os.environ["B2_ROOT"] + "/tools/bin"
KNOWN = [
    # 284 s and 283 s of CPU to paint the 2 MB document, against the 90 s cap (feature-matrix-comparison)
    dict(bench="b2", tool="mdtui", bin=B + "/mdt", version="0.11.0", cells=["mdtui-xl"]),
    # bat takes minutes to print 80 MB; mcat never printed either huge file in 60 s
    dict(bench="b2p", tool="bat", bin="/usr/bin/bat", version="0.26.1", cells=["huge.log whole bat"]),
    dict(bench="b2p", tool="mcat", bin=B + "/mcat", version="0.6.5",
         cells=["huge.json whole mcat", "huge.log whole mcat"]),
]


def installed(path):
    """The version `path --version` reports ("bat 0.26.1 (v0.25.0-402-g979ba226)" gives 0.26.1), or None."""
    try:
        out = subprocess.run([path, "--version"], capture_output=True, text=True, timeout=10).stdout.split()
    except (OSError, subprocess.TimeoutExpired):
        return None
    return out[1] if len(out) > 1 else None


def listed(bench):
    """Every cell listed for bench, whatever the installed version: what doctables.py marks when it's missing."""
    return {c for k in KNOWN if k["bench"] == bench for c in k["cells"]}


def skipped(bench):
    """The cells to skip now: listed for bench, with the tool still at the listed version."""
    if os.environ.get("B2_TIMEOUTS") == "off":
        return set()
    out = set()
    for k in (k for k in KNOWN if k["bench"] == bench):
        if (v := installed(k["bin"])) == k["version"]:
            out.update(k["cells"])
        else:
            print(f"timeouts: {k['tool']} is {v} now, not {k['version']}: running {', '.join(k['cells'])} again; "
                  "update bench/b2/timeouts.py once they finish in time", file=sys.stderr)
    return out


if __name__ == "__main__":
    print("\n".join(sorted(skipped(sys.argv[1]))))
