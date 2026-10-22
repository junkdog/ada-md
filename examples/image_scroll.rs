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

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use ada::app::{Computed, Msg};
use ada::geom::{Rect, Size};
use ada::grid::{Grid, Screen};
use ada::images::{self, Encoder, Proto};

mod common;

use common::Shared;

fn main() {
    let mut paths = Vec::new();
    let (mut cells, mut runs, mut proto) = (Size::new(100, 50), 7, Proto::Sixel);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cells" => cells = common::cells_arg(args.next()),
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
        let Some((job, (w, h))) = common::image_job(&path, font, cells) else {
            continue;
        };
        let size = job.key.size;
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
                let (t, c) = (Instant::now(), common::cpu());
                paint(&mut screen, -y);
                ms.push(t.elapsed().as_secs_f64() * 1e3);
                cpu.push(common::cpu().saturating_sub(c).as_secs_f64() * 1e3);
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
