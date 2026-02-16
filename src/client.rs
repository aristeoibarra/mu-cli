use crate::db;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub fn send_command(cmd: &str) -> Result<String, String> {
    let sock_path = db::data_dir().join("mu.sock");
    let mut stream = UnixStream::connect(&sock_path)
        .map_err(|_| "daemon not running. use 'mu play' first".to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

    // If cmd looks like JSON (starts with {), send as-is. Otherwise wrap it.
    let msg = if cmd.starts_with('{') {
        cmd.to_string()
    } else {
        format!(r#"{{"cmd":"{cmd}"}}"#)
    };

    stream
        .write_all(msg.as_bytes())
        .map_err(|e| format!("send failed: {e}"))?;
    stream
        .write_all(b"\n")
        .map_err(|e| format!("send failed: {e}"))?;
    stream.flush().ok();

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("read failed: {e}"))?;
    Ok(response.trim().to_string())
}

pub fn daemon_running() -> bool {
    let sock_path = db::data_dir().join("mu.sock");
    UnixStream::connect(&sock_path).is_ok()
}
