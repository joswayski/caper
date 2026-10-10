//! A loopback HTTP endpoint that lets the bundled FFmpeg (built without TLS)
//! stream a signed CDN URL for the player. Each request is forwarded with its
//! `Range`, so seeking reads only what it needs, through the app's own TLS.
//!
//! Only 127.0.0.1 is bound, and a route is an unguessable token mapped to one
//! URL, so other local programs cannot use it to reach arbitrary hosts.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Request heads larger than this are refused.
const MAX_HEAD_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct MediaProxy {
    port: u16,
    routes: Arc<Mutex<HashMap<String, String>>>,
}

impl MediaProxy {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let routes: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(None)
            .build()
            .map_err(std::io::Error::other)?;
        let shared = routes.clone();
        std::thread::Builder::new()
            .name("media-proxy".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let routes = shared.clone();
                    let client = client.clone();
                    std::thread::spawn(move || {
                        let _ = serve(stream, &routes, &client);
                    });
                }
            })?;
        Ok(Self { port, routes })
    }

    /// A loopback URL for `url`; [`MediaProxy::update`] swaps in a refreshed one.
    pub fn route(&self, url: &str) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.update(&token, url);
        format!("http://127.0.0.1:{}/media/{token}", self.port)
    }

    pub fn update(&self, token: &str, url: &str) {
        self.routes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(token.to_owned(), url.to_owned());
    }

    pub fn remove(&self, local: &str) {
        if let Some(token) = local.rsplit('/').next() {
            self.routes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(token);
        }
    }
}

struct Request {
    head: bool,
    token: String,
    range: Option<String>,
}

fn parse(reader: &mut impl BufRead) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let token = parts.next()?.strip_prefix("/media/")?.to_owned();
    let mut range = None;
    let mut read = line.len();
    loop {
        let mut header = String::new();
        let count = reader.read_line(&mut header).ok()?;
        read += count;
        if count == 0 || read > MAX_HEAD_BYTES {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("range")
        {
            range = Some(value.trim().to_owned());
        }
    }
    matches!(method, "GET" | "HEAD").then_some(Request {
        head: method == "HEAD",
        token,
        range,
    })
}

fn serve(
    stream: TcpStream,
    routes: &Mutex<HashMap<String, String>>,
    client: &reqwest::blocking::Client,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let Some(request) = parse(&mut reader) else {
        return writer.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let url = routes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&request.token)
        .cloned();
    let Some(url) = url else {
        return writer.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let mut upstream = if request.head {
        client.head(url)
    } else {
        client.get(url)
    };
    if let Some(range) = &request.range {
        upstream = upstream.header(reqwest::header::RANGE, range);
    }
    let Ok(mut response) = upstream.send() else {
        return writer.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let status = response.status();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\nAccept-Ranges: bytes\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for name in [
        reqwest::header::CONTENT_TYPE,
        reqwest::header::CONTENT_LENGTH,
        reqwest::header::CONTENT_RANGE,
    ] {
        if let Some(value) = response
            .headers()
            .get(&name)
            .and_then(|value| value.to_str().ok())
        {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("\r\n");
    writer.write_all(head.as_bytes())?;
    if !request.head {
        // The player closing the connection (a seek) ends this copy.
        let _ = std::io::copy(&mut response as &mut dyn Read, &mut writer);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-file origin that honours `Range: bytes=a-b`.
    fn origin(body: &'static [u8]) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                let mut range = None;
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        let (start, end) = value.trim().split_once('-').unwrap();
                        range = Some((
                            start.parse::<usize>().unwrap(),
                            end.parse::<usize>().unwrap(),
                        ));
                    }
                    line.clear();
                }
                let response = match range {
                    Some((start, end)) => [
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\n\r\n",
                            end - start + 1,
                            body.len()
                        )
                        .into_bytes(),
                        body[start..=end].to_vec(),
                    ]
                    .concat(),
                    None => [
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        )
                        .into_bytes(),
                        body.to_vec(),
                    ]
                    .concat(),
                };
                let _ = writer.write_all(&response);
            }
        });
        format!("http://127.0.0.1:{port}/original/x?exp=1&sig=s")
    }

    fn get(url: &str, range: Option<&str>) -> (u16, String, Vec<u8>) {
        let client = reqwest::blocking::Client::new();
        let mut request = client.get(url);
        if let Some(range) = range {
            request = request.header("range", range);
        }
        let response = request.send().unwrap();
        let status = response.status().as_u16();
        let content_range = response
            .headers()
            .get("content-range")
            .map(|value| value.to_str().unwrap().to_owned())
            .unwrap_or_default();
        (status, content_range, response.bytes().unwrap().to_vec())
    }

    #[test]
    fn routes_forward_ranges_and_unknown_tokens_are_refused() {
        let proxy = MediaProxy::start().unwrap();
        let local = proxy.route(&origin(b"0123456789"));
        assert!(local.starts_with("http://127.0.0.1:"));
        assert_eq!(
            get(&local, None),
            (200, String::new(), b"0123456789".to_vec())
        );
        assert_eq!(
            get(&local, Some("bytes=2-5")),
            (206, "bytes 2-5/10".into(), b"2345".to_vec())
        );
        let unknown = format!("{}/media/nope", local.rsplit_once("/media/").unwrap().0);
        assert_eq!(get(&unknown, None).0, 404);
        proxy.remove(&local);
        assert_eq!(get(&local, None).0, 404);
    }
}
