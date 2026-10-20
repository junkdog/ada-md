//! What a scroll past an image costs the UI thread (CS-3 of the clean-slate plan): each image
//! encoded by the worker for `--proto P` (sixel or iterm2) at the size a block image takes in
//! `--cells` (100×50 by default) of 10×20 pixels, then painted a row higher each frame until its
//! last row leaves the top, as a scroll by line paints it. Headless: the frames go to a buffer,
//! not a terminal. Prints the worker's time, and each frame's time (paint, diff and encode, on
//! this thread) and bytes, as medians over the frames and their worst.
//!
//! ```zsh
//! cargo build --release --example image_scroll
//! taskset -c 30 target/release/examples/image_scroll --proto sixel bench/b2/root/corpus/img/*
//! ```

use std::cell::RefCell;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use ada::app::{Computed, Msg};
use ada::grid::{Grid, Screen};
use ada::images::{self, Encoder, ImageKey, Job, Probe, Proto};
use ratatui::layout::{Rect, Size};

fn main() {
    let mut paths = Vec::new();
    let (mut cells, mut runs, mut proto) = (Size::new(100, 50), 7, Proto::Sixel);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cells" => {
                let v = args.next().unwrap_or_default();
                let (w, h) = v.split_once('x').expect("--cells WxH");
                cells = Size::new(w.parse().unwrap(), h.parse().unwrap());
            }
            "--runs" => runs = args.next().and_then(|n| n.parse().ok()).expect("--runs N"),
            "--proto" => {
                proto = match args.next().as_deref() {
                    Some("sixel") => Proto::Sixel,
                    Some("iterm2") => Proto::Iterm2,
                    p => panic!("--proto sixel|iterm2, not {p:?}"),
                }
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    // the font size half-blocks assume: 10×20
    let font = Size::new(10, 20);
    let encoder = Encoder::new(proto, font);
    for path in paths {
        let Probe::Image { w, h, .. } = images::probe(&path) else {
            eprintln!("{}: not an image", path.display());
            continue;
        };
        let size = images::fit((w, h), font, cells);
        let job = Job {
            key: ImageKey {
                path: path.clone(),
                mtime: None,
                size,
            },
            urgent: true,
        };
        let mut worker: Vec<f64> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                std::hint::black_box(job.run(&encoder));
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        worker.sort_by(f64::total_cmp);

        let (mut ms, mut cpu, mut bytes) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..runs {
            // a new image each run, so that nothing painted before is reused
            let Msg::ImageReady {
                image: Computed(Some(Ok(image))),
                ..
            } = job.run(&encoder)
            else {
                panic!("{}: no image", path.display());
            };
            let area = Rect::new(0, 0, cells.width, cells.height);
            let out = Shared::default();
            let mut screen = Screen::new(out.clone(), cells);
            let paint = |screen: &mut Screen<Shared>, y: i32| {
                screen
                    .draw(|grid: &mut Grid| images::render(&image, grid, area, 0, y))
                    .unwrap();
            };
            paint(&mut screen, 0);
            for y in 1..i32::from(size.height) {
                out.0.borrow_mut().clear();
                let (t, c) = (Instant::now(), cpu_ms());
                paint(&mut screen, -y);
                ms.push(t.elapsed().as_secs_f64() * 1e3);
                cpu.push(cpu_ms() - c);
                bytes.push(out.0.borrow().len());
            }
        }
        ms.sort_by(f64::total_cmp);
        cpu.sort_by(f64::total_cmp);
        bytes.sort_unstable();
        let median = |v: &[f64]| v.get(v.len() / 2).copied().unwrap_or_default();
        println!(
            "{} {w}x{h} px -> {}x{} cells: worker {:.2} ms; scroll frame median {:.2} ms (CPU {:.2}), max {:.2}, {} KB median, {} KB max ({} frames)",
            path.file_name().unwrap().to_string_lossy(),
            size.width,
            size.height,
            median(&worker),
            median(&ms),
            median(&cpu),
            ms.last().copied().unwrap_or_default(),
            bytes.get(bytes.len() / 2).copied().unwrap_or_default() / 1000,
            bytes.last().copied().unwrap_or_default() / 1000,
            ms.len(),
        );
    }
}

/// The frames written, kept for counting.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// This thread's CPU time, in milliseconds.
fn cpu_ms() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 * 1e3 + t.tv_nsec as f64 / 1e6
}
