//! A cell buffer that is drawn into and then flushed to the terminal, writing
//! only the rows that changed since the last frame.

use std::io::{self, Write};

use mta_graphing::render::{sgr, theme, ColorMode, Line, Style};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

pub struct Screen {
    pub w: usize,
    pub h: usize,
    cells: Vec<Cell>,
    prev: Vec<Cell>,
    full_redraw: bool,
    /// Background painted under everything.
    pub base: Style,
}

impl Screen {
    pub fn new(w: usize, h: usize) -> Screen {
        let base = Style::fg(theme::TEXT).on(theme::BG);
        let blank = Cell {
            ch: ' ',
            style: base,
        };
        Screen {
            w,
            h,
            cells: vec![blank; w * h],
            prev: vec![],
            full_redraw: true,
            base,
        }
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        *self = Screen::new(w, h);
    }

    pub fn clear(&mut self) {
        let blank = Cell {
            ch: ' ',
            style: self.base,
        };
        self.cells.iter_mut().for_each(|c| *c = blank);
    }

    /// Fill a rectangle with a background color.
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, style: Style) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.cells[yy * self.w + xx] = Cell { ch: ' ', style };
            }
        }
    }

    /// Write text at (x, y), clipped at `max_x`. Unset colors inherit from
    /// the cell underneath, so text keeps the panel background. Returns the
    /// next x.
    pub fn put_clip(
        &mut self,
        mut x: usize,
        y: usize,
        text: &str,
        style: Style,
        max_x: usize,
    ) -> usize {
        if y >= self.h {
            return x;
        }
        let max_x = max_x.min(self.w);
        for ch in text.chars() {
            if x >= max_x {
                break;
            }
            let cell = &mut self.cells[y * self.w + x];
            let mut st = style;
            if st.bg.is_none() {
                st.bg = cell.style.bg;
            }
            if st.fg.is_none() {
                st.fg = self.base.fg;
            }
            *cell = Cell { ch, style: st };
            x += 1;
        }
        x
    }

    pub fn put(&mut self, x: usize, y: usize, text: &str, style: Style) -> usize {
        self.put_clip(x, y, text, style, self.w)
    }

    /// Draw a styled line, clipped at `max_x`. Adds an ellipsis when cut.
    pub fn put_line(&mut self, x: usize, y: usize, line: &Line, max_x: usize) -> usize {
        let total: usize = line.iter().map(|s| s.text.chars().count()).sum();
        let max_x = max_x.min(self.w);
        let mut cx = x;
        if x + total <= max_x {
            for sp in line {
                cx = self.put_clip(cx, y, &sp.text, sp.style, max_x);
            }
            return cx;
        }
        let limit = max_x.saturating_sub(1);
        for sp in line {
            cx = self.put_clip(cx, y, &sp.text, sp.style, limit);
        }
        self.put_clip(cx, y, "…", Style::fg(theme::DIM), max_x)
    }

    pub fn flush(&mut self, out: &mut impl Write, mode: ColorMode) -> io::Result<()> {
        let mut s = String::with_capacity(self.w * self.h * 4);
        s.push_str("\x1b[?2026h");
        if self.full_redraw {
            s.push_str(&sgr(self.base, mode));
            s.push_str("\x1b[2J");
        }
        let mut cur: Option<Style> = None;
        for y in 0..self.h {
            let row = &self.cells[y * self.w..(y + 1) * self.w];
            if !self.full_redraw
                && self.prev.len() == self.cells.len()
                && row == &self.prev[y * self.w..(y + 1) * self.w]
            {
                continue;
            }
            s.push_str(&format!("\x1b[{};1H", y + 1));
            // Never write the bottom-right cell: some consoles scroll when it is filled.
            let n = if y + 1 == self.h {
                self.w.saturating_sub(1)
            } else {
                self.w
            };
            for c in &row[..n] {
                if cur != Some(c.style) {
                    s.push_str(&sgr(c.style, mode));
                    cur = Some(c.style);
                }
                s.push(c.ch);
            }
        }
        s.push_str("\x1b[0m\x1b[?2026l");
        out.write_all(s.as_bytes())?;
        out.flush()?;
        self.prev = self.cells.clone();
        self.full_redraw = false;
        Ok(())
    }
}
