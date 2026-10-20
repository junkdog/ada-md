//! The image worker's time from job to ready (O-54, CS-1 of the clean-slate plan): each image's
//! decode and encode as the TUI's worker runs them, at the size a block image takes in a window of
//! `--cells` (100×50 by default), and the first frame that paints it: its bytes, and the time to
//! write it to the terminal, which takes in the image as it reads them. Run it in the terminal to
//! measure: it asks that terminal for its protocol and font size, as ada does, and draws there.
//! The results go to stderr.
//!
//! ```zsh
//! cargo build --release --example image_ready
//! kitty sh -c 'target/release/examples/image_ready bench/b2/root/corpus/img/photo.jpg 2> out.txt'
//! ```

use std::cell::Cell;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use ada::app::{Computed, Msg};
use ada::grid::{Grid, Screen};
use ada::images::{self, ImageKey, ImageMode, Job, Probe};
use ratatui::layout::{Rect, Size};

fn main() {
    let mut paths = Vec::new();
    let (mut cells, mut runs) = (Size::new(100, 50), 15);
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cells" => {
                let v = args.next().unwrap_or_default();
                let (w, h) = v.split_once('x').expect("--cells WxH");
                cells = Size::new(w.parse().unwrap(), h.parse().unwrap());
            }
            "--runs" => runs = args.next().and_then(|n| n.parse().ok()).expect("--runs N"),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let _ = ratatui::crossterm::terminal::enable_raw_mode();
    let picker = images::detect(ImageMode::Auto).expect("images on");
    let _ = ratatui::crossterm::terminal::disable_raw_mode();
    let graphics = images::graphics_msg(&picker);
    eprintln!(
        "terminal: {graphics:?} capabilities {:?}",
        picker.capabilities()
    );
    let font = picker.font_size();
    let font = Size::new(font.width, font.height);
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
        let (mut ms, mut cpu): (Vec<f64>, Vec<f64>) = (0..runs)
            .map(|_| {
                let (t, c) = (Instant::now(), cpu_ms());
                std::hint::black_box(job.run(&picker));
                (t.elapsed().as_secs_f64() * 1e3, cpu_ms() - c)
            })
            .unzip();
        ms.sort_by(f64::total_cmp);
        cpu.sort_by(f64::total_cmp);
        let Msg::ImageReady {
            image: Computed(Some(Ok(image))),
            ..
        } = job.run(&picker)
        else {
            panic!("{}: no image", path.display());
        };
        let area = Rect::new(0, 0, cells.width, cells.height);
        let written = Rc::new(Cell::new(0));
        let mut screen = Screen::new(Counted(io::stdout(), written.clone()), cells);
        print!("\x1b[2J");
        let t = Instant::now();
        screen
            .draw(|grid: &mut Grid| images::render(&image, grid, area, 0, 0))
            .unwrap();
        let draw = t.elapsed().as_secs_f64() * 1e3;
        let bytes = written.get();
        print!("\x1b[2J\x1b[H");
        eprintln!(
            "{} {w}x{h} px -> {}x{} cells: median {:.2} ms (CPU {:.2}), min {:.2}, max {:.2} ({runs} runs); first frame {bytes} bytes, written in {draw:.2} ms",
            path.file_name().unwrap().to_string_lossy(),
            size.width,
            size.height,
            ms[ms.len() / 2],
            cpu[cpu.len() / 2],
            ms[0],
            ms[ms.len() - 1],
        );
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
