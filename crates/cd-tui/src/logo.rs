use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

pub use crate::logo_data::{Logo, CLUMIO_MARK, CLUMIO_MARK_LARGE, CLUMIO_MARK_SMALL};

impl Logo {
    pub fn cells(&self) -> (u16, u16) {
        (self.width as u16, self.rows.len().div_ceil(2) as u16)
    }

    pub fn fits(&self, area: Rect) -> bool {
        let (w, h) = self.cells();
        area.width >= w && area.height >= h
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, bg: Color) {
        if !self.fits(area) {
            return;
        }
        let (w, h) = self.cells();
        for row in 0..h {
            let top = self.rows[2 * row as usize];
            let bottom = self.rows.get(2 * row as usize + 1).copied();
            for col in 0..w {
                let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) else {
                    continue;
                };
                let upper = over(top[col as usize], bg);
                let lower = over(bottom.map_or(0, |r| r[col as usize]), bg);
                cell.set_char('▀').set_fg(upper).set_bg(lower);
            }
        }
    }
}

fn over(pixel: u32, bg: Color) -> Color {
    let alpha = (pixel & 0xFF) as u16;
    let (br, bg_, bb) = match bg {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0, 0, 0),
    };
    let mix = |c: u32, b: u8| ((c as u16 * alpha + b as u16 * (255 - alpha)) / 255) as u8;
    Color::Rgb(
        mix((pixel >> 24) & 0xFF, br),
        mix((pixel >> 16) & 0xFF, bg_),
        mix((pixel >> 8) & 0xFF, bb),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    #[ignore]
    fn print_logos() {
        for logo in [&CLUMIO_MARK_LARGE, &CLUMIO_MARK, &CLUMIO_MARK_SMALL] {
            let (w, h) = logo.cells();
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            logo.render(area, &mut buf, theme::BG);
            for y in 0..h {
                let mut line = String::new();
                for x in 0..w {
                    let cell = &buf[(x, y)];
                    if let (Color::Rgb(fr, fg, fb), Color::Rgb(br, bg, bb)) = (cell.fg, cell.bg) {
                        line += &format!(
                            "\x1b[38;2;{fr};{fg};{fb}m\x1b[48;2;{br};{bg};{bb}m{}",
                            cell.symbol()
                        );
                    }
                }
                println!("{line}\x1b[0m");
            }
            println!();
        }
    }

    #[test]
    fn logos_have_even_pixel_rows_and_uniform_width() {
        for logo in [&CLUMIO_MARK_LARGE, &CLUMIO_MARK, &CLUMIO_MARK_SMALL] {
            assert_eq!(logo.rows.len() % 2, 0);
            assert!(logo.rows.iter().all(|r| r.len() == logo.width));
        }
        let widths: Vec<usize> = [&CLUMIO_MARK_LARGE, &CLUMIO_MARK, &CLUMIO_MARK_SMALL]
            .iter()
            .map(|l| l.width)
            .collect();
        assert_eq!(widths, vec![20, 14, 8]);
    }

    #[test]
    fn transparent_pixels_take_the_surface_colour() {
        assert_eq!(over(0x962a7500, theme::BG), theme::BG);
        assert_eq!(over(0x962a75ff, theme::BG), Color::Rgb(0x96, 0x2a, 0x75));
    }

    #[test]
    fn skips_drawing_when_the_area_is_too_small() {
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        CLUMIO_MARK.render(area, &mut buf, theme::BG);
        assert!(buf.content.iter().all(|c| c.symbol() == " "));
    }
}
