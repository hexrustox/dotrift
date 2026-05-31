use std::sync::OnceLock;

mod ls_colors;
pub mod pager;
pub mod prompt;

pub fn is_unicode() -> bool {
    static UNICODE: OnceLock<bool> = OnceLock::new();
    *UNICODE.get_or_init(|| {
        ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .any(|s| s.contains("UTF-8") || s.contains("utf8"))
    })
}
