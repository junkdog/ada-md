//! `b2 flood`: scrolling at full speed. Keys go one per write at a fixed rate, or as fast as the tty's input
//! queue drains (RATE 0, 16 keys a write), as N x DOWN then N x UP, repeating, so the view never parks at an end.
//!
//! Frames are counted by the cursor hide (`\e[?25l`) that ratatui writes once per draw, so the frame columns mean
//! something only for ratatui viewers. Each key is timed to the end of the next frame. CPU and `write` calls come
//! from /proc per thread, over the flood and the drain after it. If the viewer accepts no key for `STALL` seconds,
//! it's killed (which also frees the blocked write) and the run reports `stalled`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::pty::{Session, now};
use crate::{base_env, unescape};

const STALL: f64 = 3.0;
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";

struct Thread {
    name: String,
    cpu_ns: u64,
    writes: u64,
}

/// The viewer's threads by tid; the main thread is named `main`, the rest `comm/tid`.
fn threads(pid: i32) -> BTreeMap<String, Thread> {
    let mut out = BTreeMap::new();
    let Ok(dir) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return out;
    };
    for e in dir.flatten() {
        let (p, tid) = (e.path(), e.file_name().to_string_lossy().into_owned());
        let read = |f: &str| fs::read_to_string(p.join(f)).unwrap_or_default();
        let name = if tid == pid.to_string() {
            "main".into()
        } else {
            format!("{}/{tid}", read("comm").trim())
        };
        let cpu_ns = read("schedstat")
            .split_whitespace()
            .next()
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        let writes = read("io")
            .lines()
            .find_map(|l| l.strip_prefix("syscw:")?.trim().parse().ok())
            .unwrap_or(0);
        out.insert(
            tid,
            Thread {
                name,
                cpu_ns,
                writes,
            },
        );
    }
    out
}

/// Quantiles of `v` (sorted here); null when empty.
fn quantiles(v: &mut [f64], qs: &[(&str, f64)]) -> Value {
    v.sort_by(f64::total_cmp);
    let q = |p: f64| {
        v.get(((v.len().max(1) - 1) as f64 * p).round() as usize)
            .copied()
    };
    Value::Object(qs.iter().map(|&(k, p)| (k.into(), json!(q(p)))).collect())
}

pub fn cmd_flood(args: &[String]) -> io::Result<()> {
    let usage = || io::Error::other("usage: b2 flood CWD RATE SECS DOWN UP N -- CMD...");
    let sep = args.iter().position(|a| a == "--").ok_or_else(usage)?;
    let [cwd, rate, secs, down, up, n] = &args[..sep] else {
        return Err(usage());
    };
    let (rate, secs): (f64, f64) = (
        rate.parse().map_err(|_| usage())?,
        secs.parse().map_err(|_| usage())?,
    );
    let n: usize = n.parse().map_err(|_| usage())?;
    let (down, up) = (unescape(down), unescape(up));
    let key = |k: usize| {
        if (k / n.max(1)).is_multiple_of(2) {
            &down
        } else {
            &up
        }
    };

    let mut s = Session::spawn(&args[sep + 1..], &base_env(), cwd)?;
    s.wait_quiet(0.8, 20.0);
    let first_chunk = s.chunks.len();
    let before = threads(s.pid);

    let progress = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let stalled = Arc::new(AtomicBool::new(false));
    {
        let (progress, done, stalled, pid) =
            (progress.clone(), done.clone(), stalled.clone(), s.pid);
        thread::spawn(move || {
            let (mut seen, mut since) = (u64::MAX, now());
            while !done.load(Relaxed) {
                thread::sleep(Duration::from_millis(200));
                let p = progress.load(Relaxed);
                if p != seen {
                    (seen, since) = (p, now());
                } else if now() - since > STALL && !done.load(Relaxed) {
                    stalled.store(true, Relaxed);
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    return;
                }
            }
        });
    }

    let t0 = now();
    let mut sent: Vec<f64> = vec![];
    let mut keys = 0usize;
    if rate > 0.0 {
        loop {
            let due = t0 + keys as f64 / rate;
            if due >= t0 + secs {
                break;
            }
            // spin: a sleep's wakeup jitter is as large as the gaps at high rates (the driver has its own core)
            while now() < due {
                std::hint::spin_loop();
            }
            let t = now();
            if s.write(key(keys).as_bytes()).is_err() {
                break;
            }
            sent.push(t);
            keys += 1;
            progress.fetch_add(1, Relaxed);
        }
    } else {
        let mut buf = String::new();
        while now() < t0 + secs {
            buf.clear();
            (keys..keys + 16).for_each(|k| buf.push_str(key(k)));
            if s.write(buf.as_bytes()).is_err() {
                break;
            }
            keys += 16;
            progress.fetch_add(1, Relaxed);
        }
    }
    let t_end = now();
    done.store(true, Relaxed);
    let stalled = stalled.load(Relaxed);
    // a stalled viewer is dead by now, so this only collects what it wrote before
    s.wait_quiet(0.5, 60.0);
    let after = (!stalled).then(|| threads(s.pid));

    // frames end where a cursor hide completes, at the time of the chunk that completes it
    let mut frames: Vec<f64> = vec![];
    let mut bytes = 0usize;
    let mut carry: Vec<u8> = vec![];
    for (t, c) in &s.chunks[first_chunk..] {
        bytes += c.len();
        let win = [&carry[..], &c[..]].concat();
        let mut from = 0;
        while let Some(j) = win[from..]
            .windows(HIDE_CURSOR.len())
            .position(|w| w == HIDE_CURSOR)
        {
            if from + j + HIDE_CURSOR.len() > carry.len() {
                frames.push(*t);
            }
            from += j + 1;
        }
        carry = win[win.len().saturating_sub(HIDE_CURSOR.len() - 1)..].to_vec();
    }
    let last_out = s.chunks.last().map_or(t_end, |c| c.0).max(t_end);
    let per_frame = |x: f64| (!frames.is_empty()).then(|| x / frames.len() as f64);

    let mut to_frame: Vec<f64> = vec![];
    let mut j = 0;
    for &tk in &sent {
        while frames.get(j).is_some_and(|&f| f < tk) {
            j += 1;
        }
        if let Some(f) = frames.get(j) {
            to_frame.push((f - tk) * 1e3);
        }
    }
    let mut gaps: Vec<f64> = frames.windows(2).map(|w| (w[1] - w[0]) * 1e3).collect();

    let mut per_thread = Map::new();
    let mut cpu_ms = None;
    if let Some(after) = &after {
        let mut total = 0.0;
        for (tid, a) in after {
            let b = before.get(tid);
            let cpu = (a.cpu_ns - b.map_or(0, |b| b.cpu_ns)) as f64 / 1e6;
            total += cpu;
            if cpu > 0.0 {
                let writes = a.writes - b.map_or(0, |b| b.writes);
                per_thread.insert(a.name.clone(), json!({"cpu_ms": cpu, "writes": writes}));
            }
        }
        cpu_ms = Some(total);
    }
    let main_writes = per_thread.get("main").and_then(|m| m["writes"].as_u64());

    let rec = json!({
        "cmd": &args[sep + 1..], "rate": rate, "secs": secs, "keys": keys, "keys_per_s": keys as f64 / secs,
        "stalled": stalled,
        "frames": frames.len(),
        "fps": frames.iter().filter(|&&f| f <= t_end).count() as f64 / secs,
        "keys_per_frame": per_frame(keys as f64),
        "bytes_per_frame": per_frame(bytes as f64),
        "writes_per_frame": main_writes.and_then(|w| per_frame(w as f64)),
        "lag_ms": (last_out - t_end) * 1e3,
        "cpu_ms": cpu_ms,
        "cpu_per_frame_ms": cpu_ms.and_then(per_frame),
        "key_to_frame_ms": quantiles(&mut to_frame, &[("p50", 0.5), ("p99", 0.99), ("max", 1.0)]),
        "frame_gap_ms": quantiles(&mut gaps, &[("p50", 0.5), ("p99", 0.99)]),
        "threads": per_thread,
    });
    println!("{rec}");
    s.close("q");
    Ok(())
}
