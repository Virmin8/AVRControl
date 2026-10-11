//! Tiny line-based IPC with the running background instance: a Unix socket
//! on Linux, a loopback TCP port on Windows. Also acts as the single-instance
//! lock. Requests: `trigger <id>`, `reload`, `reconnect`, `settings`,
//! `configure-shortcuts`, `quit`, `ping`.

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};

#[cfg(windows)]
use std::net::{TcpListener as Listener, TcpStream as Stream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("avrcontrol.sock")
}

#[cfg(windows)]
const ADDR: &str = "127.0.0.1:47823";

fn connect() -> std::io::Result<Stream> {
    #[cfg(unix)]
    return Stream::connect(socket_path());
    #[cfg(windows)]
    return Stream::connect_timeout(&ADDR.parse().unwrap(), Duration::from_millis(500));
}

/// Sends one request to the running instance and returns its reply.
pub fn request(cmd: &str) -> Result<String> {
    request_with_timeout(cmd, Duration::from_secs(5))
}

pub fn request_with_timeout(cmd: &str, timeout: Duration) -> Result<String> {
    let mut stream = connect().context("AVR Control is not running")?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(format!("{cmd}\n").as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    let reply = reply.trim().to_string();
    match reply.strip_prefix("error: ") {
        Some(e) => bail!("{e}"),
        None => Ok(reply),
    }
}

/// Runs a receiver query through the background service when it's running,
/// so it reuses its connection: some receivers (e.g. the DTR-50.7) only allow
/// one client and drop the other. Connects directly otherwise.
pub fn ask_receiver<T: serde::de::DeserializeOwned>(
    request: &str,
    host: &str,
    port: u16,
    arg: &str,
    timeout: Duration,
    direct: impl FnOnce(&mut crate::eiscp::Receiver) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if is_running() {
        let json = request_with_timeout(
            format!("{request} {} {port} {arg}", host.trim()).trim(),
            timeout,
        )?;
        return Ok(serde_json::from_str(&json)?);
    }
    let mut r = crate::eiscp::Receiver::new(host, port);
    r.connect()?;
    let result = direct(&mut r);
    r.disconnect();
    result
}

pub fn is_running() -> bool {
    request("ping").is_ok()
}

/// Claims the IPC endpoint and answers requests on a background thread.
/// Fails if another instance already owns it.
pub fn serve(handler: impl Fn(&str) -> Result<String> + Send + Sync + 'static) -> Result<()> {
    let listener = bind()?;
    let handler = std::sync::Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // One thread per request: a slow one (feature detection) must
            // not hold up hotkey triggers.
            let handler = handler.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let reply = match handler(line.trim()) {
                    Ok(r) => r,
                    Err(e) => format!("error: {e:#}"),
                };
                let mut stream = &stream;
                let _ = stream.write_all(format!("{reply}\n").as_bytes());
            });
        }
    });
    Ok(())
}

fn bind() -> Result<Listener> {
    #[cfg(unix)]
    {
        let path = socket_path();
        match Listener::bind(&path) {
            Ok(l) => Ok(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if is_running() {
                    bail!("AVR Control is already running");
                }
                // Left over from a crash.
                std::fs::remove_file(&path)?;
                Ok(Listener::bind(&path)?)
            }
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(windows)]
    {
        Listener::bind(ADDR).map_err(|_| anyhow::anyhow!("AVR Control is already running"))
    }
}

/// Removes the socket file on clean shutdown.
pub fn cleanup() {
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket_path());
}
