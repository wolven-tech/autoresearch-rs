//! Local-only product-web fixture server.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

const HOME: &str = include_str!("../../../examples/product-web/index.html");
const METADATA: &str = include_str!("../../../examples/product-web/metadata.html");
const MISSING: &str = include_str!("../../../examples/product-web/missing.html");

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:4419")?;
    for stream in listener.incoming() {
        serve(stream?)?;
    }
    Ok(())
}

fn serve(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(3)))?;
    let mut request = [0_u8; 4096];
    let count = stream.read(&mut request)?;
    let first_line = String::from_utf8_lossy(&request[..count]);
    let (status, body) = match first_line.split_whitespace().nth(1) {
        Some("/") => ("200 OK", HOME),
        Some("/metadata") => ("200 OK", METADATA),
        _ => ("404 Not Found", MISSING),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}
