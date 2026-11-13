//! The image worker's time from job to ready (O-54, CS-1 of the clean-slate plan): each image's
//! decode and encode as the TUI's worker runs them, at the size a block image takes in a window of
//! `--cells` (100×50 by default), and the first frame that paints it: its bytes, the time to
//! paint and write it, and the time until the terminal answers a device status query sent after
//! it, which takes in the terminal's decode of the image (IMG-002). Run it in the terminal to
//! measure: it asks that terminal for its protocol, font size and kitty's shared memory, as ada
//! does, and draws there; `--stream` sends kitty images in the output whatever the probe found.
//! The results go to stderr.
//!
//! ```zsh
//! cargo build --release --example image_ready
//! kitty sh -c 'target/release/examples/image_ready bench/b2/root/corpus/img/photo.jpg 2> out.txt'
//! ```

use std::cell::Cell;
use std::env;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use ada::app::{Computed, Msg};
use ada::geom::{Rect, Size};
use ada::grid::{Grid, Screen};
use ada::images::{self, ImageMode, Wire};

mod common;

fn main() {
    let mut paths = Vec::new();
    let (mut cells, mut runs, mut stream) = (Size::new(100, 50), 15, false);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cells" => cells = common::cells_arg(args.next()),
            "--runs" => runs = args.next().and_then(|n| n.parse().ok()).expect("--runs N"),
            "--stream" => stream = true,
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let _ = crossterm::terminal::enable_raw_mode();
    let graphics = images::detect(ImageMode::Auto).expect("images on");
    let _ = crossterm::terminal::disable_raw_mode();
    eprintln!("terminal: {graphics:?}");
    let wire = if stream { Wire::Stream } else { graphics.wire };
    eprintln!("kitty images: {}", wire.name());
    let encoder = images::Encoder::new(graphics.proto, graphics.cell).wire(wire);
    let font = graphics.cell;
    for path in paths {
        let Some((job, (w, h))) = common::image_job(&path, font, cells) else {
            continue;
        };
        let size = job.key.size;
        let (mut ms, mut cpu): (Vec<f64>, Vec<f64>) = (0..runs)
            .map(|_| {
                let (t, c) = (Instant::now(), common::cpu());
                std::hint::black_box(job.run(&encoder));
                let ms = t.elapsed().as_secs_f64() * 1e3;
                (ms, common::cpu().saturating_sub(c).as_secs_f64() * 1e3)
            })
            .unzip();
        ms.sort_by(f64::total_cmp);
        cpu.sort_by(f64::total_cmp);
        // each run a new image (a new id), painted by a new screen's first frame
        let (mut draw, mut answered, mut bytes) = (Vec::new(), Vec::new(), 0);
        for _ in 0..runs {
            let Msg::ImageReady {
                image: Computed(Some(Ok(image))),
                ..
            } = job.run(&encoder)
            else {
                panic!("{}: no image", path.display());
            };
            let area = Rect::new(0, 0, cells.width, cells.height);
            let written = Rc::new(Cell::new(0));
            let mut screen = Screen::new(Counted(io::stdout(), written.clone()), cells);
            print!("\x1b[2J");
            io::stdout().flush().unwrap();
            let _ = crossterm::terminal::enable_raw_mode();
            let t = Instant::now();
            screen
                .draw(|grid: &mut Grid| images::render(&image, grid, area, 0, 0))
                .unwrap();
            draw.push(t.elapsed().as_secs_f64() * 1e3);
            bytes = written.get();
            print!("\x1b[5n");
            io::stdout().flush().unwrap();
            status_reply();
            answered.push(t.elapsed().as_secs_f64() * 1e3);
            let _ = crossterm::terminal::disable_raw_mode();
            print!("\x1b[2J\x1b[H");
        }
        draw.sort_by(f64::total_cmp);
        answered.sort_by(f64::total_cmp);
        eprintln!(
            "{} {w}x{h} px -> {}x{} cells: median {:.2} ms (CPU {:.2}), min {:.2}, max {:.2} ({runs} runs); first frame {bytes} bytes, painted and written in {:.2} ms, answered at {:.2} ms (medians)",
            path.file_name().unwrap().to_string_lossy(),
            size.width,
            size.height,
            ms[ms.len() / 2],
            cpu[cpu.len() / 2],
            ms[0],
            ms[ms.len() - 1],
            draw[draw.len() / 2],
            answered[answered.len() / 2],
        );
    }
}

/// Read the tty up to the device status reply, `ESC [ 0 n`.
fn status_reply() {
    let mut stdin = io::stdin().lock();
    let (mut got, mut b) = (Vec::new(), [0u8; 1]);
    while stdin.read_exact(&mut b).is_ok() {
        got.push(b[0]);
        if got.ends_with(b"\x1b[0n") {
            break;
        }
    }
}

/// The terminal, counting what's written to it.
struct Counted(io::Stdout, Rc<Cell<usize>>);

impl Write for Counted {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.0.write(buf)?;
        self.1.set(self.1.get() + n);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
