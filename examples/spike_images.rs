//! Spike A: images in a scrolling viewport.
//!
//! A fake document of numbered text rows with one tall image embedded at row 12, rendered with
//! ratatui-image's `SlicedProtocol` (kitty: unicode placeholders; sixel: band stripping;
//! halfblocks: plain cells). Scroll past it and look for flicker, tearing at the top and bottom
//! edges, and placements left behind after the image has scrolled out or the window resized.
//!
//! ```zsh
//! cargo run --release --example spike_images -- ~/opt/birb-vault/20-areas/journal/neumann-cat-photo.png
//! cargo run --release --example spike_images -- photo.jpg --protocol sixel --rows 80
//! cargo run --release --example spike_images -- photo.jpg --headless   # smoke test, no terminal
//! ```
//!
//! Keys: `j`/`k` row · `d`/`u` half page · `space`/`b` page · `g`/`G` top/bottom ·
//! `a` autoscroll (bounces at ~60 rows/s) · `A` autoscroll as fast as frames draw · `q` quit.
//!
//! The loop blocks in `event::read()` unless autoscroll is on, so it is idle-free like preen.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{env, process};

use image::DynamicImage;
use image::imageops::FilterType;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use ratatui_image::Resize;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};

const IMAGE_ROW: i32 = 12;
const LEFT_MARGIN: u16 = 8;

struct Args {
    image: PathBuf,
    protocol: Option<ProtocolType>,
    rows: Option<u16>,
    headless: bool,
}

fn parse_args() -> Args {
    let mut image = None;
    let mut protocol = None;
    let mut rows = None;
    let mut headless = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--protocol" => {
                protocol = Some(match args.next().as_deref() {
                    Some("kitty") => ProtocolType::Kitty,
                    Some("sixel") => ProtocolType::Sixel,
                    Some("iterm2") => ProtocolType::Iterm2,
                    Some("halfblocks") => ProtocolType::Halfblocks,
                    _ => usage(),
                });
            }
            "--rows" => {
                rows = Some(
                    args.next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(|| usage()),
                );
            }
            "--headless" => headless = true,
            "-h" | "--help" => usage(),
            _ if image.is_none() => image = Some(PathBuf::from(arg)),
            _ => usage(),
        }
    }
    let Some(image) = image else { usage() };
    Args {
        image,
        protocol,
        rows,
        headless,
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: spike_images <image> [--protocol kitty|sixel|iterm2|halfblocks] [--rows N] [--headless]"
    );
    process::exit(2);
}

struct App {
    picker: Picker,
    image: DynamicImage,
    /// Rebuilt when the viewport width changes; keyed by the width it was built for.
    sliced: Option<(u16, SlicedProtocol)>,
    rows_cap: Option<u16>,
    scroll: i32,
    autoscroll: Option<Autoscroll>,
    encode_time: Duration,
    last_draw: Duration,
    max_draw: Duration,
    frames: u64,
}

#[derive(Clone, Copy)]
struct Autoscroll {
    dir: i32,
    interval: Duration,
}

impl App {
    fn image_size(&self) -> Size {
        self.sliced
            .as_ref()
            .map(|(_, s)| s.size())
            .unwrap_or_default()
    }

    fn doc_rows(&self, viewport: u16) -> i32 {
        IMAGE_ROW + i32::from(self.image_size().height) + 2 * i32::from(viewport)
    }

    fn max_scroll(&self, viewport: u16) -> i32 {
        (self.doc_rows(viewport) - i32::from(viewport)).max(0)
    }

    /// Re-encodes the image when the width changes. This is the cost preen would pay on the
    /// image worker; here it runs inline so it shows up in the status line.
    fn ensure_protocol(&mut self, area: Rect) -> Duration {
        if self.sliced.as_ref().is_some_and(|(w, _)| *w == area.width) {
            return Duration::ZERO;
        }
        let width = area.width.saturating_sub(LEFT_MARGIN + 2).max(1);
        let height = self
            .rows_cap
            .unwrap_or(area.height.saturating_mul(3))
            .max(1);
        let start = Instant::now();
        let sliced = SlicedProtocol::new_with_resize(
            &self.picker,
            self.image.clone(),
            Size::new(width, height),
            Resize::Fit(Some(FilterType::Triangle)),
        )
        .unwrap_or_else(|e| {
            ratatui::restore();
            eprintln!("encode failed: {e}");
            process::exit(1);
        });
        self.encode_time = start.elapsed();
        self.sliced = Some((area.width, sliced));
        self.encode_time
    }

    fn scroll_by(&mut self, delta: i32, viewport: u16) {
        self.scroll = (self.scroll + delta).clamp(0, self.max_scroll(viewport));
    }

    fn tick_autoscroll(&mut self, viewport: u16) {
        let max = self.max_scroll(viewport);
        let Some(auto) = &mut self.autoscroll else {
            return;
        };
        let next = self.scroll + auto.dir;
        if next < 0 || next > max {
            auto.dir = -auto.dir;
        }
        self.scroll = (self.scroll + auto.dir).clamp(0, max);
    }
}

fn main() {
    let args = parse_args();
    let image = image::ImageReader::open(&args.image)
        .and_then(image::ImageReader::with_guessed_format)
        .map_err(|e| e.to_string())
        .and_then(|r| r.decode().map_err(|e| e.to_string()))
        .unwrap_or_else(|e| {
            eprintln!("{}: {e}", args.image.display());
            process::exit(1);
        });

    if args.headless {
        headless(image, args.protocol, args.rows);
        return;
    }

    let mut terminal = ratatui::init();
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    if let Some(protocol) = args.protocol {
        picker.set_protocol_type(protocol);
    } else if picker.tmux_detected() && picker.protocol_type() == ProtocolType::Kitty {
        // preen-plan P6: kitty graphics don't work inside tmux
        picker.set_protocol_type(ProtocolType::Halfblocks);
    }

    let mut app = App {
        picker,
        image,
        sliced: None,
        rows_cap: args.rows,
        scroll: 0,
        autoscroll: None,
        encode_time: Duration::ZERO,
        last_draw: Duration::ZERO,
        max_draw: Duration::ZERO,
        frames: 0,
    };

    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("{e}");
        process::exit(1);
    }
    println!(
        "{:?} · {} frames · max draw {:.2} ms · encode {:.1} ms",
        app.picker.protocol_type(),
        app.frames,
        ms(app.max_draw),
        ms(app.encode_time)
    );
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    loop {
        draw(terminal, app)?;
        let viewport = terminal.size()?.height.saturating_sub(1);

        let event = match app.autoscroll {
            Some(auto) => {
                if event::poll(auto.interval)? {
                    Some(event::read()?)
                } else {
                    app.tick_autoscroll(viewport);
                    None
                }
            }
            None => Some(event::read()?),
        };
        let Some(Event::Key(key)) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let page = i32::from(viewport);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('j') | KeyCode::Down => app.scroll_by(1, viewport),
            KeyCode::Char('k') | KeyCode::Up => app.scroll_by(-1, viewport),
            KeyCode::Char('d') => app.scroll_by(page / 2, viewport),
            KeyCode::Char('u') => app.scroll_by(-page / 2, viewport),
            KeyCode::Char(' ') | KeyCode::PageDown => app.scroll_by(page, viewport),
            KeyCode::Char('b') | KeyCode::PageUp => app.scroll_by(-page, viewport),
            KeyCode::Char('g') | KeyCode::Home => app.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => app.scroll = app.max_scroll(viewport),
            KeyCode::Char('a') => {
                app.autoscroll = toggle(app.autoscroll, Duration::from_millis(16));
            }
            KeyCode::Char('A') => app.autoscroll = toggle(app.autoscroll, Duration::ZERO),
            _ => {}
        }
    }
}

fn toggle(auto: Option<Autoscroll>, interval: Duration) -> Option<Autoscroll> {
    match auto {
        Some(_) => None,
        None => Some(Autoscroll { dir: 1, interval }),
    }
}

fn draw<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), B::Error> {
    // draw time excludes re-encoding, which preen does on a worker
    let start = Instant::now();
    let mut encode = Duration::ZERO;
    terminal.draw(|f| {
        let doc = Rect {
            height: f.area().height.saturating_sub(1),
            ..f.area()
        };
        encode = app.ensure_protocol(doc);
        app.scroll = app.scroll.min(app.max_scroll(doc.height));
        render(f, app, doc);
    })?;
    app.last_draw = start.elapsed().saturating_sub(encode);
    app.max_draw = app.max_draw.max(app.last_draw);
    app.frames += 1;
    Ok(())
}

fn render(f: &mut Frame<'_>, app: &App, doc: Rect) {
    let size = app.image_size();
    let image_rows = IMAGE_ROW..IMAGE_ROW + i32::from(size.height);
    let dim = Style::new().fg(Color::DarkGray);

    let lines: Vec<Line> = (0..i32::from(doc.height))
        .map(|r| {
            let row = app.scroll + r;
            if image_rows.contains(&row) {
                // rows reserved for the image; the image widget paints over them
                Line::from(format!("{row:>5} ┃")).style(dim)
            } else {
                Line::from(format!(
                    "{row:>5} │ The quick brown fox jumps over the lazy dog, row {row} of the document."
                ))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), doc);

    if let Some((_, sliced)) = &app.sliced {
        let y = IMAGE_ROW - app.scroll;
        let position = SignedPosition::from((
            LEFT_MARGIN as i16,
            y.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
        ));
        f.render_widget(SlicedImage::new(sliced, position), doc);
    }

    let status = format!(
        " {:?} · font {}×{} · image {}×{} cells · scroll {}/{} · draw {:.2} ms (max {:.2}) · encode {:.1} ms{}",
        app.picker.protocol_type(),
        app.picker.font_size().width,
        app.picker.font_size().height,
        size.width,
        size.height,
        app.scroll,
        app.max_scroll(doc.height),
        ms(app.last_draw),
        ms(app.max_draw),
        ms(app.encode_time),
        if app.autoscroll.is_some() {
            " · AUTO"
        } else {
            ""
        },
    );
    let bar = Rect {
        y: doc.bottom(),
        height: 1,
        ..f.area()
    };
    f.render_widget(Paragraph::new(status).black().on_yellow(), bar);
}

/// Smoke test without a terminal: scroll down and back up at two sizes, assert nothing panics,
/// and print the frame where the image straddles the top edge.
fn headless(image: DynamicImage, protocol: Option<ProtocolType>, rows: Option<u16>) {
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(protocol.unwrap_or(ProtocolType::Halfblocks));
    let mut app = App {
        picker,
        image,
        sliced: None,
        rows_cap: rows,
        scroll: 0,
        autoscroll: Some(Autoscroll {
            dir: 1,
            interval: Duration::ZERO,
        }),
        encode_time: Duration::ZERO,
        last_draw: Duration::ZERO,
        max_draw: Duration::ZERO,
        frames: 0,
    };

    let mut snapshot = None;
    for (w, h) in [(100, 30), (64, 20)] {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test backend");
        app.scroll = 0;
        app.autoscroll = Some(Autoscroll {
            dir: 1,
            interval: Duration::ZERO,
        });
        let max = app.doc_rows(h - 1);
        for _ in 0..2 * max {
            draw(&mut terminal, &mut app).expect("draw");
            if snapshot.is_none() && app.scroll == IMAGE_ROW + 2 {
                snapshot = Some(format!("{}", terminal.backend()));
            }
            app.tick_autoscroll(h - 1);
        }
    }
    if let Some(frame) = snapshot {
        println!("{frame}");
    }
    println!(
        "{:?} · {} frames · max draw {:.2} ms · encode {:.1} ms",
        app.picker.protocol_type(),
        app.frames,
        ms(app.max_draw),
        ms(app.encode_time)
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
