//! Hand-rolled sd_notify (PRD R11): READY/WATCHDOG/STATUS notifications via a
//! unix datagram to `$NOTIFY_SOCKET`. No systemd crate.
//!
//! Invariant: when `$NOTIFY_SOCKET` is unset (not run under systemd) every
//! function is a no-op returning `false` — never an error. Notify is advisory:
//! any send failure is also a `false`, never a panic.

use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::UnixDatagram;

/// Notify READY=1. Returns false when not running under systemd.
pub fn sd_ready() -> bool {
    send("READY=1")
}

/// Ping WATCHDOG=1. Returns false when not running under systemd.
pub fn sd_watchdog() -> bool {
    send("WATCHDOG=1")
}

/// Set STATUS=… for `systemctl status`. Returns false when not running under
/// systemd.
pub fn sd_status(msg: &str) -> bool {
    send(&format!("STATUS={msg}"))
}

/// Send one newline-terminated datagram to `$NOTIFY_SOCKET` (filesystem path,
/// or abstract-namespace address with a leading `@`, per sd_notify(3)).
/// Advisory only: any failure — unset var, bad address, dead socket — is a
/// `false`, never a panic and never an error up the stack.
fn send(msg: &str) -> bool {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return false;
    };
    let Ok(socket) = UnixDatagram::unbound() else {
        return false;
    };
    let mut payload = msg.as_bytes().to_vec();
    payload.push(b'\n');

    let bytes = path.as_encoded_bytes();
    let sent = if bytes.first() == Some(&b'@') {
        // Abstract namespace: the name is everything after the leading '@'.
        match std::os::unix::net::SocketAddr::from_abstract_name(&bytes[1..]) {
            Ok(addr) => socket.send_to_addr(&payload, &addr).unwrap_or(0),
            Err(_) => 0,
        }
    } else {
        socket
            .send_to(&payload, std::path::Path::new(&path))
            .unwrap_or(0)
    };
    sent == payload.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unset `$NOTIFY_SOCKET` → every verb is a no-op returning `false`
    /// (the R11 invariant).
    #[test]
    fn unset_socket_is_noop_false() {
        std::env::remove_var("NOTIFY_SOCKET");
        assert!(!sd_ready());
        assert!(!sd_watchdog());
        assert!(!sd_status("test"));
    }

    /// A real (filesystem) notify socket receives exactly `READY=1\n` and
    /// `STATUS=…\n` datagrams.
    #[test]
    fn live_socket_receives_ready_and_status() {
        let dir = std::env::temp_dir().join(format!("afanctl-t4-notify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock_path = dir.join("notify.sock");
        let listener = UnixDatagram::bind(&sock_path).unwrap();
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();

        std::env::set_var("NOTIFY_SOCKET", &sock_path);
        assert!(sd_ready());
        let mut buf = [0u8; 64];
        let n = listener.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"READY=1\n");

        assert!(sd_status("holding 3000 rpm"));
        let n = listener.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"STATUS=holding 3000 rpm\n");

        std::env::remove_var("NOTIFY_SOCKET");
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Abstract-socket form (`NOTIFY_SOCKET=@name`): datagram reaches a
    /// listener bound to the same abstract name; no filesystem node involved.
    #[test]
    fn abstract_socket_form_reaches_listener() {
        let name = format!("afanctl-t4-abs-{}", std::process::id());
        let addr = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let listener = UnixDatagram::bind_addr(&addr).unwrap();
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();

        std::env::set_var("NOTIFY_SOCKET", format!("@{name}"));
        assert!(sd_watchdog());
        let mut buf = [0u8; 64];
        let n = listener.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"WATCHDOG=1\n");
        std::env::remove_var("NOTIFY_SOCKET");
    }

    /// Dead socket path (no listener bound) → `false`, no panic (advisory
    /// notify invariant).
    #[test]
    fn dead_socket_returns_false() {
        let dir = std::env::temp_dir().join(format!("afanctl-t4-dead-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock_path = dir.join("gone.sock");
        std::env::set_var("NOTIFY_SOCKET", &sock_path);
        assert!(!sd_ready());
        std::env::remove_var("NOTIFY_SOCKET");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
