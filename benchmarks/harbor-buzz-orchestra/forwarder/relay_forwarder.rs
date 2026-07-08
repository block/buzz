//! Loopback TCP forwarder for the benchmark task container.
//!
//! The benchmark relay is host-header tenant-bound: its community row is the
//! authority of its own `RELAY_URL` (e.g. `localhost:3600`), and a request
//! presenting any other `Host` fails closed. Agents inside a Harbor task
//! container can only reach the host-published relay via the Docker host
//! alias (`host.docker.internal`), which would present the wrong `Host`.
//!
//! So the container runtime uploads this forwarder next to the agent stack:
//! agents dial `ws://localhost:<port>` — presenting the exact `Host` the
//! community row expects — and the forwarder bridges the byte stream to the
//! host gateway. Transparent to everything above TCP (WebSocket, the buzz
//! CLI, git-over-HTTP). std-only; compiled with plain `rustc` against the
//! musl target, so it runs on any Linux task image.
//!
//! Usage: `relay-forwarder <listen-addr> <target-addr>`

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let (listen, target) = match (args.next(), args.next()) {
        (Some(listen), Some(target)) => (listen, target),
        _ => {
            eprintln!("usage: relay-forwarder <listen-addr> <target-addr>");
            std::process::exit(2);
        }
    };
    let listener = TcpListener::bind(&listen)?;
    // Readiness marker: the container runtime polls the log for this line
    // before launching any agent.
    println!("forwarding {listen} -> {target}");
    for client in listener.incoming() {
        let Ok(client) = client else { continue };
        let target = target.clone();
        thread::spawn(move || bridge(client, &target));
    }
    Ok(())
}

/// Connect upstream and pump bytes both ways until either side closes.
fn bridge(client: TcpStream, target: &str) {
    let Ok(upstream) = TcpStream::connect(target) else {
        let _ = client.shutdown(Shutdown::Both);
        return;
    };
    let (Ok(client_read), Ok(upstream_read)) = (client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    let downstream = thread::spawn(move || pipe(upstream_read, client));
    pipe(client_read, upstream);
    let _ = downstream.join();
}

/// Copy until EOF or error, then half-close the write side so protocols
/// layered on TCP (WebSocket close handshakes) terminate cleanly.
fn pipe(mut from: TcpStream, mut to: TcpStream) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(Shutdown::Write);
}
