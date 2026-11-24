//! Fetching: curl with a time limit (no TLS stack in this program), every
//! answer kept in `~/.cache/nirutoday/` so the window opens with the last
//! data at once and keeps it offline. `NIRUTODAY_FIXTURES=DIR` answers each
//! request from `DIR/<key>` instead (tests).
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const AGENT: &str = concat!(
    "nirutoday/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/nirucon/nirutoday)"
);

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::theme::home().join(".cache"))
        .join("nirutoday")
}

fn fixtures() -> Option<PathBuf> {
    std::env::var_os("NIRUTODAY_FIXTURES").map(PathBuf::from)
}

/// A request: GET `url`, or POST `body` (sent on stdin, so a key in it never
/// shows in the process list).
pub struct Req<'a> {
    pub key: &'a str,
    pub url: &'a str,
    pub body: Option<&'a str>,
}

/// Fetch and cache. On failure the error; the cached copy stays.
pub fn fetch(r: &Req) -> Result<String, String> {
    if let Some(dir) = fixtures() {
        return std::fs::read_to_string(dir.join(r.key))
            .map_err(|_| format!("no fixture {}", r.key));
    }
    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsSL",
        "--compressed",
        "--connect-timeout",
        "8",
        "--max-time",
        "25",
        "-A",
        AGENT,
    ]);
    if r.body.is_some() {
        cmd.args([
            "-X",
            "POST",
            "-H",
            "Content-Type: text/xml",
            "--data-binary",
            "@-",
        ]);
    }
    cmd.arg(r.url)
        .stdin(if r.body.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("curl: {e}"))?;
    if let (Some(body), Some(mut stdin)) = (r.body, child.stdin.take()) {
        let _ = stdin.write_all(body.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().trim_start_matches("curl: ").to_string();
        return Err(if err.is_empty() {
            "download failed".into()
        } else {
            err
        });
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    store(r.key, &text);
    Ok(text)
}

fn store(key: &str, text: &str) {
    let dir = cache_dir();
    // Private: it may hold your calendars.
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = dir.join(format!(".{key}.{}", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(key));
    }
}

/// The cached answer for `key` and when it was fetched.
pub fn cached(key: &str) -> Option<(String, i64)> {
    if let Some(dir) = fixtures() {
        return std::fs::read_to_string(dir.join(key))
            .ok()
            .map(|t| (t, now()));
    }
    let p = cache_dir().join(key);
    let age = std::fs::metadata(&p).ok()?.modified().ok()?;
    let at = age
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;
    Some((std::fs::read_to_string(p).ok()?, at))
}

/// Remember small state (notified ids) next to the cache.
pub fn read_state(name: &str) -> String {
    std::fs::read_to_string(cache_dir().join(name)).unwrap_or_default()
}
pub fn write_state(name: &str, text: &str) {
    store(name, text);
}
