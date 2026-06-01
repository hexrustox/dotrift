use std::time::{SystemTime, UNIX_EPOCH};

use miette::{Context, Result, miette};

pub fn epoch_ms(time: Option<SystemTime>) -> Result<i64> {
    let t = time.unwrap_or_else(SystemTime::now);
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|e| miette!(e))
        .wrap_err("system clock is before epoch")
}
