use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};

const S_ISUID: u32 = 0o4000;
const S_ISGID: u32 = 0o2000;
const S_ISVTX: u32 = 0o1000;
const S_IWOTH: u32 = 0o0002;
const S_IXANY: u32 = 0o0111;
const S_ISVTX_IWOTH: u32 = S_ISVTX | S_IWOTH;

pub struct LsColors {
    indicators: HashMap<String, Style>,
    patterns: Vec<(String, Style)>,
}

impl LsColors {
    pub fn new() -> Self {
        let mut indicators = HashMap::new();
        let mut patterns = Vec::new();

        if let Ok(env) = std::env::var("LS_COLORS") {
            for pair in env.split(':') {
                if let Some((k, v)) = pair.split_once('=') {
                    let style = parse_sgr(v);
                    if let Some(suffix) = k.strip_prefix('*') {
                        patterns.push((suffix.to_string(), style));
                    } else {
                        indicators.insert(k.to_string(), style);
                    }
                }
            }
        }

        Self {
            indicators,
            patterns,
        }
    }

    fn indicator(&self, key: &str) -> Style {
        self.indicators.get(key).copied().unwrap_or_default()
    }

    pub fn style_for(
        &self,
        name: &str,
        mode: Option<u32>,
        is_dir: bool,
        is_symlink: bool,
        is_broken: bool,
    ) -> Style {
        if is_symlink {
            if is_broken {
                return self.indicator("or");
            }
            return self.indicator("ln");
        }

        if is_dir {
            return self.dir_style(mode);
        }

        self.file_style(name, mode)
    }

    fn dir_style(&self, mode: Option<u32>) -> Style {
        if let Some(m) = mode {
            if m & S_ISVTX_IWOTH == S_ISVTX_IWOTH {
                return self.indicator("tw");
            }
            if m & S_IWOTH != 0 {
                return self.indicator("ow");
            }
            if m & S_ISVTX != 0 {
                return self.indicator("st");
            }
        }
        self.indicator("di")
    }

    fn file_style(&self, name: &str, mode: Option<u32>) -> Style {
        if let Some(m) = mode {
            if m & S_ISUID != 0 {
                return self.indicator("su");
            }
            if m & S_ISGID != 0 {
                return self.indicator("sg");
            }
            if m & S_ISVTX_IWOTH == S_ISVTX_IWOTH {
                return self.indicator("tw");
            }
            if m & S_IWOTH != 0 {
                return self.indicator("ow");
            }
            if m & S_ISVTX != 0 {
                return self.indicator("st");
            }
        }

        for (suffix, style) in &self.patterns {
            if name.ends_with(suffix) {
                return *style;
            }
        }

        if let Some(m) = mode
            && m & S_IXANY != 0
        {
            return self.indicator("ex");
        }

        self.indicator("fi")
    }
}

fn parse_sgr(sgr: &str) -> Style {
    if sgr.is_empty() {
        return Style::default();
    }

    let codes: Vec<u8> = sgr.split(';').filter_map(|s| s.parse().ok()).collect();
    let mut style = Style::default();
    let mut i = 0;

    while i < codes.len() {
        match codes[i] {
            0 => {}
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            5 | 6 => style = style.add_modifier(Modifier::SLOW_BLINK),
            7 => style = style.add_modifier(Modifier::REVERSED),
            8 => style = style.add_modifier(Modifier::HIDDEN),
            9 => style = style.add_modifier(Modifier::CROSSED_OUT),
            30..=37 => {
                style = style.fg(color_from_ansi(codes[i] - 30));
            }
            40..=47 => {
                style = style.bg(color_from_ansi(codes[i] - 40));
            }
            90..=97 => {
                style = style.fg(bright_color(codes[i] - 90));
            }
            100..=107 => {
                style = style.bg(bright_color(codes[i] - 100));
            }
            38 => {
                i += 1;
                if i < codes.len() {
                    match codes[i] {
                        5 => {
                            i += 1;
                            if i < codes.len() {
                                style = style.fg(Color::Indexed(codes[i]));
                            }
                        }
                        2 if i + 3 < codes.len() => {
                            style = style.fg(Color::Rgb(codes[i + 1], codes[i + 2], codes[i + 3]));
                            i += 3;
                        }
                        _ => {}
                    }
                }
            }
            48 => {
                i += 1;
                if i < codes.len() {
                    match codes[i] {
                        5 => {
                            i += 1;
                            if i < codes.len() {
                                style = style.bg(Color::Indexed(codes[i]));
                            }
                        }
                        2 if i + 3 < codes.len() => {
                            style = style.bg(Color::Rgb(codes[i + 1], codes[i + 2], codes[i + 3]));
                            i += 3;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    style
}

fn color_from_ansi(idx: u8) -> Color {
    match idx {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::White,
        _ => Color::Reset,
    }
}

fn bright_color(idx: u8) -> Color {
    match idx {
        0 => Color::Gray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        7 => Color::White,
        _ => Color::Reset,
    }
}
