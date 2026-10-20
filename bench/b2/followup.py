"""B2 follow-ups. usage: python -I followup.py WHICH"""
import os, sys, json
sys.path.insert(0, os.path.dirname(__file__))
import b2

S = os.environ.get("B2_ROOT") or os.path.join(os.path.dirname(os.path.abspath(__file__)), "root")  # corpus/, tools/bin/, home/<viewer>/, b2/, b2a/
B = S + "/tools/bin"
env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
for k in ("TMUX", "KITTY_WINDOW_ID", "TERM_PROGRAM"):
    env.pop(k, None)
which = sys.argv[1]
cwd = f"{S}/jobs/followup-{which}"
os.makedirs(cwd, exist_ok=True)
for d in ("l", "xl"):
    open(f"{cwd}/{d}.md", "wb").write(open(f"{S}/corpus/{d}.md", "rb").read())
out = {"which": which}


def screen(s):
    s.live.advance_to(1e18)
    return "\n".join(l.rstrip() for l in s.live.text().split("\n") if l.strip())


if which == "mdtui-xl-paint":
    env["XDG_CONFIG_HOME"] = f"{S}/home/mdtui/config"
    s = b2.Session([B + "/mdt", "xl.md"], env, cwd)
    ok = s.wait_stable(1.0, 900, need="harvest")
    rp = b2.Replay(s.chunks)
    out["first_ms"] = rp.scan(s.t0, 1e18, "harvest")["shown"]
    out["ok"] = ok
    out["cpu_ms"] = s.cpu_ms()
    out["hwm_mib"] = s.hwm_kib() / 1024
    s.close("q")
elif which == "mdtui-l-resize":
    env["XDG_CONFIG_HOME"] = f"{S}/home/mdtui/config"
    s = b2.Session([B + "/mdt", "l.md"], env, cwd)
    s.wait_stable(0.6, 30, need="Successor")
    c0 = s.cpu_ms()
    t = s.resize(40, 90, "resize")
    n = len(s.chunks)
    while len(s.chunks) == n and b2.now() < t + 600:  # wait for any output at all
        s.pump(1.0)
    ok = s.wait_stable(1.0, 600)
    rp = b2.Replay(s.chunks)
    rp.scan(s.t0, t)
    rp.resize(40, 90)
    r = rp.scan(t, 1e18)
    out.update(ok=ok, repaint_first_ms=r["first"], repaint_done_ms=r["done"], cpu_ms=s.cpu_ms() - c0)
    out["screen"] = screen(s)[:600]
    s.close("q")
elif which == "leaf-xl-end":
    env["XDG_CONFIG_HOME"] = f"{S}/home/leaf/config"
    s = b2.Session([B + "/leaf", "--watch", "xl.md"], env, cwd)
    s.wait_stable(0.6, 30, need="harvest")
    s.send("G", "end")
    s.wait_stable(0.5, 10)
    out["screen"] = screen(s)
    s.close("q")
elif which == "mdfried-xl-search":
    env["XDG_CONFIG_HOME"] = f"{S}/home/mdfried/config"
    s = b2.Session([B + "/mdfried", "--watch", "xl.md"], env, cwd)
    s.wait_stable(0.6, 30, need="harvest")
    s.send("/", "open")
    s.wait_stable(0.2, 3)
    t = s.send("premature consolidation\r", "search")
    ok = s.wait_stable(0.5, 30, need="Abstraction level is correct")
    rp = b2.Replay(s.chunks)
    out["search_ms"] = rp.scan(t, 1e18, "Abstraction level is correct")["shown"]
    out["search_cpu_note"] = "see b2 runs for cpu"
    out["ok"] = ok
    out["screen"] = screen(s)
    s.send("n", "n")
    s.wait_stable(0.5, 10)
    out["screen_after_n"] = screen(s)
    s.close("q")
print(json.dumps(out, indent=1))
