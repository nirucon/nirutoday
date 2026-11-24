//! "Listen": a stream or a broadcast plays in NIRUAMP. NIRUAMP is started
//! when it is not running (it starts NPLAY's service itself); then the
//! address goes to NPLAY's control socket as `playback.play_external`
//! (newline-delimited JSON-RPC 2.0). Runs off the window's thread.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// NPLAY's socket: `NPLAY_SERVICE_SOCKET`, else `$XDG_RUNTIME_DIR/nplay`,
/// `$XDG_STATE_HOME/nplay/run` or `~/.local/state/nplay/run`, `control.sock`.
pub fn socket_path() -> PathBuf {
    let get = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(p) = get("NPLAY_SERVICE_SOCKET") {
        return p;
    }
    let dir = get("XDG_RUNTIME_DIR")
        .map(|d| d.join("nplay"))
        .or_else(|| get("XDG_STATE_HOME").map(|d| d.join("nplay/run")))
        .unwrap_or_else(|| crate::theme::home().join(".local/state/nplay/run"));
    dir.join("control.sock")
}

fn running(name: &str) -> bool {
    let uid = unsafe { libc::getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.flatten().any(|e| {
        use std::os::unix::fs::MetadataExt;
        e.file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
            && e.metadata().map(|m| m.uid() == uid).unwrap_or(false)
            && std::fs::read_to_string(e.path().join("comm"))
                .map(|c| c.trim() == name)
                .unwrap_or(false)
    })
}

fn spawn(program: &str) -> bool {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(program);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().is_ok()
}

fn call(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    id: u32,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let req = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let mut line = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|e| format!("NPLAY: {e}"))?;
    loop {
        let mut buf = String::new();
        if reader
            .read_line(&mut buf)
            .map_err(|e| format!("NPLAY: {e}"))?
            == 0
        {
            return Err("NPLAY closed the connection".into());
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&buf) else {
            continue;
        };
        // Notifications (state pushes) have no id.
        if v.get("id").and_then(|x| x.as_u64()) != Some(u64::from(id)) {
            continue;
        }
        if let Some(e) = v.get("error") {
            return Err(e
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("NPLAY refused")
                .to_string());
        }
        return Ok(v.get("result").cloned().unwrap_or_default());
    }
}

/// Play `url` titled `title` in NIRUAMP. Ok with what happened.
pub fn play(url: &str, title: &str) -> Result<String, String> {
    let mut started = false;
    if !running("niruamp") {
        if !spawn("niruamp") {
            return Err("NIRUAMP is not installed (nirux extras install niruamp)".into());
        }
        started = true;
    }
    let path = socket_path();
    let deadline = Instant::now() + Duration::from_secs(if started { 15 } else { 6 });
    let mut stream = loop {
        match UnixStream::connect(&path) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("NPLAY's service did not answer ({e})")),
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(8)));
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let _ = call(
        &mut stream,
        &mut reader,
        1,
        "initialize",
        serde_json::json!({"client": {"name": "nirutoday", "version": env!("CARGO_PKG_VERSION")}, "protocol_versions": [1]}),
    );
    call(
        &mut stream,
        &mut reader,
        2,
        "playback.play_external",
        serde_json::json!({"url": url, "title": title}),
    )?;
    Ok(if started {
        format!("{title}: NIRUAMP started, playing")
    } else {
        format!("{title}: playing in NIRUAMP")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talks_json_rpc_to_the_socket() {
        let dir = std::env::temp_dir().join(format!("nirutoday-play-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut seen = Vec::new();
            for _ in 0..2 {
                let mut l = String::new();
                r.read_line(&mut l).unwrap();
                let v: serde_json::Value = serde_json::from_str(&l).unwrap();
                seen.push(v.clone());
                // A state push first, then the answer.
                w.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"state.changed\",\"params\":{}}\n")
                    .unwrap();
                let answer = serde_json::json!({"jsonrpc":"2.0","id":v["id"],"result":{"ok":true}});
                w.write_all(format!("{answer}\n").as_bytes()).unwrap();
            }
            seen
        });
        let mut s = UnixStream::connect(&sock).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        call(&mut s, &mut r, 1, "initialize", serde_json::json!({})).unwrap();
        call(
            &mut s,
            &mut r,
            2,
            "playback.play_external",
            serde_json::json!({"url": "https://x/219.mp3", "title": "P4 Väst"}),
        )
        .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(seen[1]["method"], "playback.play_external");
        assert_eq!(seen[1]["params"]["title"], "P4 Väst");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
