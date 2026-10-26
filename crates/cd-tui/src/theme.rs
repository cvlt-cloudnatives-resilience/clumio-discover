use cd_model::{ProtectionStatus, Severity};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(0x0A, 0x0A, 0x0F);
pub const SURFACE_0: Color = Color::Rgb(0x15, 0x14, 0x1B);
pub const SURFACE_1: Color = Color::Rgb(0x17, 0x16, 0x1D);
pub const SURFACE_2: Color = Color::Rgb(0x1C, 0x1B, 0x23);
pub const TEXT: Color = Color::Rgb(0xF4, 0xF2, 0xFA);
pub const MUTE: Color = Color::Rgb(0xA4, 0xA1, 0xB3);
pub const DIM: Color = Color::Rgb(0x6E, 0x6B, 0x7E);
pub const HAIRLINE: Color = Color::Rgb(0x3A, 0x38, 0x44);
pub const HAIRLINE_STRONG: Color = Color::Rgb(0x51, 0x4E, 0x5E);
pub const CROCUS: Color = Color::Rgb(0x83, 0x48, 0x95);
pub const CROCUS_BRIGHT: Color = Color::Rgb(0xC0, 0x7F, 0xD6);
pub const LAVENDER: Color = Color::Rgb(0xC9, 0xA6, 0xFF);
pub const PINK: Color = Color::Rgb(0xE9, 0x2B, 0x62);
pub const OK: Color = Color::Rgb(0x3D, 0xD6, 0x8C);
pub const WARN: Color = Color::Rgb(0xF2, 0xA9, 0x3B);
pub const INDIGO: Color = Color::Rgb(0x6E, 0x74, 0xB7);
pub const GREY: Color = Color::Rgb(0xC4, 0xC7, 0xE3);
pub const ON_ACCENT: Color = Color::Rgb(0xFF, 0xFF, 0xFF);
pub const ACCENT_SOFT: Color = Color::Rgb(0x2F, 0x21, 0x37);

pub fn base() -> Style {
    Style::new().fg(TEXT).bg(BG)
}

pub fn panel() -> Style {
    Style::new().fg(TEXT).bg(SURFACE_1)
}

pub fn border(focused: bool) -> Style {
    Style::new().fg(if focused {
        CROCUS_BRIGHT
    } else {
        HAIRLINE_STRONG
    })
}

pub fn title() -> Style {
    Style::new().fg(TEXT).add_modifier(Modifier::BOLD)
}

pub fn header() -> Style {
    Style::new()
        .fg(MUTE)
        .bg(SURFACE_2)
        .add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::new()
        .fg(ON_ACCENT)
        .bg(CROCUS)
        .add_modifier(Modifier::BOLD)
}

pub fn selected_soft() -> Style {
    Style::new().bg(ACCENT_SOFT).add_modifier(Modifier::BOLD)
}

pub fn key() -> Style {
    Style::new().fg(LAVENDER).add_modifier(Modifier::BOLD)
}

pub fn status(status: ProtectionStatus) -> Color {
    match status {
        ProtectionStatus::AwsBackup => OK,
        ProtectionStatus::Unprotected => PINK,
    }
}

pub fn severity(severity: Severity) -> Color {
    match severity {
        Severity::High => PINK,
        Severity::Medium => WARN,
        Severity::Low => LAVENDER,
        Severity::Info => MUTE,
    }
}

pub fn truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v == "truecolor" || v == "24bit"
        })
        .unwrap_or(false)
}

pub fn downsample(buf: &mut Buffer) {
    for cell in buf.content.iter_mut() {
        cell.fg = to_indexed(cell.fg);
        cell.bg = to_indexed(cell.bg);
    }
}

pub fn to_indexed(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |c: u8| {
        (0..6)
            .min_by_key(|&i| (LEVELS[i] as i32 - c as i32).abs())
            .unwrap_or(0)
    };
    let dist = |(x, y, z): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(x, r) + d(y, g) + d(z, b)
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let cube_index = 16 + 36 * ri + 6 * gi + bi;

    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gray_step = (avg.saturating_sub(3) / 10).min(23) as u8;
    let level = 8 + 10 * gray_step;
    let gray = (level, level, level);

    if dist(gray) < dist(cube) {
        Color::Indexed(232 + gray_step)
    } else {
        Color::Indexed(cube_index as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_brand_colours_to_close_256_colour_indices() {
        assert_eq!(to_indexed(Color::Rgb(0, 0, 0)), Color::Indexed(16));
        assert_eq!(to_indexed(Color::Rgb(255, 255, 255)), Color::Indexed(231));
        assert_eq!(to_indexed(BG), Color::Indexed(232));
        assert_eq!(to_indexed(CROCUS), Color::Indexed(96));
        assert_eq!(to_indexed(PINK), Color::Indexed(161));
        assert_eq!(to_indexed(Color::Reset), Color::Reset);
    }
}
