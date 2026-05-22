use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};

pub struct LsColors {
    indicators: HashMap<String, String>,
    patterns: Vec<(String, String)>,
}

impl LsColors {
    pub fn new() -> Self {
        let mut indicators = HashMap::new();
        let mut patterns = Vec::new();

        if let Ok(env) = std::env::var("LS_COLORS") {
            for pair in env.split(':') {
                if let Some((k, v)) = pair.split_once('=') {
                    if k.starts_with('*') {
                        patterns.push((k.to_string(), v.to_string()));
                    } else {
                        indicators.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }

        Self {
            indicators,
            patterns,
        }
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
                return parse_sgr(self.indicators.get("or").map(String::as_str).unwrap_or(""));
            }
            return parse_sgr(self.indicators.get("ln").map(String::as_str).unwrap_or(""));
        }

        if is_dir {
            return self.dir_style(mode);
        }

        self.file_style(name, mode)
    }

    fn dir_style(&self, mode: Option<u32>) -> Style {
        if let Some(m) = mode {
            let tw = m & 0o1002 == 0o1002;
            let ow = m & 0o0002 != 0;
            let st = m & 0o1000 != 0;

            if tw {
                return parse_sgr(self.indicators.get("tw").map(String::as_str).unwrap_or(""));
            }
            if ow {
                return parse_sgr(self.indicators.get("ow").map(String::as_str).unwrap_or(""));
            }
            if st {
                return parse_sgr(self.indicators.get("st").map(String::as_str).unwrap_or(""));
            }
        }
        parse_sgr(self.indicators.get("di").map(String::as_str).unwrap_or(""))
    }

    fn file_style(&self, name: &str, mode: Option<u32>) -> Style {
        if let Some(m) = mode {
            if m & 0o4000 != 0 {
                return parse_sgr(self.indicators.get("su").map(String::as_str).unwrap_or(""));
            }
            if m & 0o2000 != 0 {
                return parse_sgr(self.indicators.get("sg").map(String::as_str).unwrap_or(""));
            }
            if m & 0o1002 == 0o1002 {
                return parse_sgr(self.indicators.get("tw").map(String::as_str).unwrap_or(""));
            }
            if m & 0o0002 != 0 {
                return parse_sgr(self.indicators.get("ow").map(String::as_str).unwrap_or(""));
            }
            if m & 0o1000 != 0 {
                return parse_sgr(self.indicators.get("st").map(String::as_str).unwrap_or(""));
            }
        }

        for (key, sgr) in &self.patterns {
            if let Some(suffix) = key.strip_prefix('*')
                && name.ends_with(suffix)
            {
                return parse_sgr(sgr);
            }
        }

        if let Some(m) = mode
            && m & 0o0111 != 0
        {
            return parse_sgr(self.indicators.get("ex").map(String::as_str).unwrap_or(""));
        }

        parse_sgr(self.indicators.get("fi").map(String::as_str).unwrap_or(""))
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
