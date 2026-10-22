//! What the examples share. A directory with no `main.rs`, so cargo doesn't take it for an example.

// each example uses its own share of these
#![allow(dead_code)]

use std::cell::RefCell;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use std::{env, fs};

use ada::geom::Size;
use ada::images::{self, ImageKey, Job, Probe};

/// This thread's CPU time.
pub fn cpu() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// A writer whose bytes the example keeps a handle on, to count or inspect them.
#[derive(Clone, Default)]
pub struct Shared(pub Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `[--min-kb N] <file-or-dir>...`: the roots, and the smallest file in KB to measure.
pub fn markdown_args(usage: fn() -> !) -> (Vec<PathBuf>, u64) {
    let mut roots = Vec::new();
    let mut min_kb = 0u64;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--min-kb" => {
                min_kb = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| usage());
            }
            "-h" | "--help" => usage(),
            _ => roots.push(PathBuf::from(arg)),
        }
    }
    if roots.is_empty() {
        usage();
    }
    (roots, min_kb)
}

/// The markdown files under `path`, or `path` itself, hidden files and directories left out.
pub fn collect_markdown(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            collect_markdown(&p, out);
        }
    } else if path.extension().is_some_and(|e| e == "md") {
        out.push(path.to_path_buf());
    }
}

/// `--cells WxH`'s value.
pub fn cells_arg(value: Option<String>) -> Size {
    let v = value.unwrap_or_default();
    let (w, h) = v.split_once('x').expect("--cells WxH");
    Size::new(w.parse().unwrap(), h.parse().unwrap())
}

/// The job encoding the image at `path` to fit `cells` at the `font` size, and the image's size
/// in pixels. `None`, said on stderr, when it isn't an image.
pub fn image_job(path: &Path, font: Size, cells: Size) -> Option<(Job, (u32, u32))> {
    let Probe::Image { w, h, .. } = images::probe(path) else {
        eprintln!("{}: not an image", path.display());
        return None;
    };
    let job = Job {
        key: ImageKey {
            path: path.to_owned(),
            mtime: None,
            size: images::fit((w, h), font, cells),
        },
        urgent: true,
    };
    Some((job, (w, h)))
}
