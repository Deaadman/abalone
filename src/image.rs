//! Pictures in the terminal: an RGBA image drawn in half blocks, each cell
//! one pixel wide and two tall, so a square image looks square.
//!
//! ```
//! use shellkit::image::Image;
//!
//! let red = [255, 0, 0, 255];
//! let img = Image::new(4, 4, vec![red; 16]);
//! let rows = img.rows(); // 2 rows of 4 cells
//! assert_eq!(rows.len(), 2);
//! ```
//!
//! [`Image::cells`] takes a colour function for tinting or for what counts
//! as see-through; the cells are one piece per column, so marks can be put
//! on them by index. [`Cache`] keeps a resized image between frames.

use crossterm::style::Color;

use crate::style::{Row, Style};

/// Pixels as `[r, g, b, a]`, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[u8; 4]>,
}

impl Image {
    pub fn new(width: usize, height: usize, px: Vec<[u8; 4]>) -> Self {
        assert_eq!(px.len(), width * height, "an image needs width × height pixels");
        Image { width, height, px }
    }

    /// From RGBA bytes, four per pixel, as image decoders give them.
    pub fn from_rgba(width: usize, height: usize, bytes: &[u8]) -> Self {
        Image::new(width, height, bytes.as_chunks::<4>().0.to_vec())
    }

    /// Shrunk (or stretched) to `w`×`h` by averaging, colour weighted by alpha.
    pub fn resize(&self, w: usize, h: usize) -> Image {
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let (y0, y1) = span(y, h, self.height);
            for x in 0..w {
                let (x0, x1) = span(x, w, self.width);
                let (mut r, mut g, mut b, mut a, mut n) = (0f64, 0f64, 0f64, 0f64, 0f64);
                for py in y0..y1 {
                    for px in x0..x1 {
                        let p = self.px[py * self.width + px];
                        let alpha = p[3] as f64;
                        r += p[0] as f64 * alpha;
                        g += p[1] as f64 * alpha;
                        b += p[2] as f64 * alpha;
                        a += alpha;
                        n += 1.0;
                    }
                }
                out.push(if a > 0.0 {
                    [(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8]
                } else {
                    [0, 0, 0, 0]
                });
            }
        }
        Image::new(w, h, out)
    }

    /// Turned `turns` quarter turns clockwise.
    pub fn turn(&self, turns: u8) -> Image {
        let mut img = self.clone();
        for _ in 0..turns % 4 {
            let (w, h) = (img.width, img.height);
            let mut px = Vec::with_capacity(w * h);
            // Row y of the turned image is column y of the old one, bottom to top.
            for y in 0..w {
                for x in 0..h {
                    px.push(img.px[(h - 1 - x) * w + y]);
                }
            }
            img = Image::new(h, w, px);
        }
        img
    }

    /// The image in half blocks, `color` deciding each pixel's colour and
    /// `None` for see-through. One piece per column in every row; an odd
    /// last row of pixels is dropped.
    pub fn cells(&self, color: impl Fn([u8; 4]) -> Option<Color>) -> Vec<Row> {
        let w = self.width;
        (0..self.height / 2)
            .map(|r| {
                (0..w)
                    .map(|c| {
                        let top = color(self.px[2 * r * w + c]);
                        let bottom = color(self.px[(2 * r + 1) * w + c]);
                        match (top, bottom) {
                            (Some(t), Some(b)) => (
                                Style {
                                    fg: Some(t),
                                    bg: Some(b),
                                    bold: false,
                                },
                                "▀".to_owned(),
                            ),
                            (Some(t), None) => (Style::fg(t), "▀".to_owned()),
                            (None, Some(b)) => (Style::fg(b), "▄".to_owned()),
                            (None, None) => (Style::default(), " ".to_owned()),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// [`cells`](Self::cells) in the image's own colours; pixels under half
    /// opacity are see-through.
    pub fn rows(&self) -> Vec<Row> {
        self.cells(|[r, g, b, a]| (a >= 128).then_some(Color::Rgb { r, g, b }))
    }
}

/// Source pixels `[start, end)` for cell `i` of `cells` over `size` pixels.
fn span(i: usize, cells: usize, size: usize) -> (usize, usize) {
    let start = i * size / cells;
    let end = ((i + 1) * size / cells).max(start + 1).min(size);
    (start.min(size.saturating_sub(1)), end)
}

/// An image resized once per size, for a panel that redraws every frame.
#[derive(Debug, Clone, Default)]
pub struct Cache(Option<Image>);

impl Cache {
    /// `source` at `w`×`h`, resized only when the size changed.
    pub fn get(&mut self, source: &Image, w: usize, h: usize) -> &Image {
        if self.0.as_ref().is_none_or(|i| (i.width, i.height) != (w, h)) {
            self.0 = Some(source.resize(w, h));
        }
        self.0.as_ref().expect("just filled")
    }

    /// Like [`get`](Self::get), for a source made on demand (decoded from a
    /// file, say) only when a resize is needed.
    pub fn get_with<E>(&mut self, w: usize, h: usize, source: impl FnOnce() -> Result<Image, E>) -> Result<&Image, E> {
        if self.0.as_ref().is_none_or(|i| (i.width, i.height) != (w, h)) {
            self.0 = Some(source()?.resize(w, h));
        }
        Ok(self.0.as_ref().expect("just filled"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::row_text;

    #[test]
    fn turns_clockwise() {
        // 2 wide, 1 tall: [a b] turned once is a above b... on its side.
        let (a, b) = ([1, 0, 0, 255], [2, 0, 0, 255]);
        let img = Image::new(2, 1, vec![a, b]);
        let once = img.turn(1);
        assert_eq!((once.width, once.height), (1, 2));
        assert_eq!(once.px, [a, b]);
        assert_eq!(img.turn(4), img);
        let twice = img.turn(2);
        assert_eq!(twice.px, [b, a]);
    }

    #[test]
    fn draws_half_blocks() {
        let (on, off) = ([255, 255, 255, 255], [0, 0, 0, 0]);
        let img = Image::new(3, 2, vec![on, off, on, on, on, off]);
        let rows = img.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 3, "a piece per column");
        assert_eq!(row_text(&rows[0]), "▀▄▀");
        assert_eq!(rows[0][0].0.bg, Some(Color::Rgb { r: 255, g: 255, b: 255 }));
    }

    #[test]
    fn resizes_by_averaging() {
        let img = Image::new(2, 2, vec![[0, 0, 0, 255], [200, 0, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0]]);
        let one = img.resize(1, 1);
        assert_eq!(one.px, [[100, 0, 0, 127]]);
        let mut cache = Cache::default();
        assert_eq!(cache.get(&img, 1, 1), &one);
    }
}
