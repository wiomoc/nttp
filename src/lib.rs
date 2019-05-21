#[cfg(target_os = "macos")]
#[macro_use]
extern crate objc;
#[cfg(target_os = "macos")]
extern crate block;
#[cfg(target_os = "macos")]
extern crate objc_foundation;
#[cfg(target_os = "macos")]
extern crate objc_id;

#[cfg(target_os = "linux")]
extern crate curl;
#[cfg(target_os = "linux")]
extern crate libc;

#[cfg(target_os = "windows")]
extern crate winapi;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::ops::Deref;
    use std::thread;
    use std::thread::JoinHandle;

    #[test]
    fn happy_path() {
        let mut request_headers = HashMap::new();
        request_headers.insert("Head", "value");
        request_headers.insert("Head-Head", "value1");
        request_headers.insert("Accept", "*/*");
        request_headers.insert("Accept-Language", "en-en");
        request_headers.insert("Content-Type", "application/x-www-form-urlencoded");
        request_headers.insert("User-Agent", "nttp");

        let mut response_headers = HashMap::new();
        response_headers.insert("Head-Res", "response");
        response_headers.insert("Header", "res");
        response_headers.insert("Content-Length", "3");

        let join_handle = http_request_verifier(vec![
            HttpExchange {
                request_method: "POST",
                request_body: "ABC".as_bytes(),
                request_headers: request_headers.clone(),
                response_status_code: 404,
                response_reason_phase: "NOT FOUND",
                response_body: "XYZ".as_bytes(),
                response_headers: response_headers.clone(),
            },
            HttpExchange {
                request_method: "GET",
                request_body: &[],
                request_headers,
                response_status_code: 200,
                response_reason_phase: "OK",
                response_body: "890".as_bytes(),
                response_headers,
            },
        ]);

        let body = "ABC".as_bytes();

        let session = Session::new();

        let response = session
            .request("POST", "http://localhost:45362/test")
            .header("Head", "value")
            .header("Head-Head", "value1")
            .header("User-Agent", "nttp")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept-Language", "en-en")
            .header("Accept", "*/*")
            .body_bytes(body)
            .send()
            .unwrap();

        assert_eq!(response.status_code(), 404);
        assert_eq!(response.body(), "XYZ".as_bytes());
        assert_eq!(response.headers().list().len(), 3);
        assert_eq!(response.headers().get("Head-Res").unwrap(), "response");
        assert_eq!(response.headers().get("Header").unwrap(), "res");
        assert_eq!(response.headers().get("Content-Length").unwrap(), "3");

        let response = session
            .request("GET", "http://localhost:45362/test")
            .header("Head", "value")
            .header("Head-Head", "value1")
            .header("User-Agent", "nttp")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept-Language", "en-en")
            .header("Accept", "*/*")
            .send()
            .unwrap();

        join_handle.join().unwrap();

        assert_eq!(response.status_code(), 200);
        assert_eq!(response.body(), "890".as_bytes());
        assert_eq!(response.headers().list().len(), 3);
        assert_eq!(response.headers().get("Head-Res").unwrap(), "response");
        assert_eq!(response.headers().get("Header").unwrap(), "res");
        assert_eq!(response.headers().get("Content-Length").unwrap(), "3");
    }

    #[test]
    #[should_panic]
    fn error_unsupported_url() {
        Session::new().request("GET", "moz://a").send().unwrap();
    }

    struct HttpExchange {
        request_method: &'static str,
        request_body: &'static [u8],
        request_headers: HashMap<&'static str, &'static str>,
        response_status_code: u16,
        response_reason_phase: &'static str,
        response_body: &'static [u8],
        response_headers: HashMap<&'static str, &'static str>,
    }

    fn http_request_verifier(exchanges: Vec<HttpExchange>) -> JoinHandle<()> {
        thread::spawn(move || {
            let listener = TcpListener::bind(("127.0.0.1", 45362)).unwrap();
            for exchange in exchanges {
                let (mut socket, _) = listener.accept().unwrap();

                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut buffer = String::new();
                reader.read_line(&mut buffer).unwrap();
                assert_eq!(
                    buffer,
                    format!("{} /test HTTP/1.1\r\n", exchange.request_method)
                );

                let mut not_found_headers = exchange.request_headers.clone();
                loop {
                    buffer = String::new();
                    reader.read_line(&mut buffer).unwrap();
                    if buffer == "\r\n" {
                        break;
                    }
                    let (key, value) = buffer.split_at(buffer.find(':').unwrap());
                    // Remove ": "
                    let value = value.split_at(2).1;
                    // Remove "\r\n"
                    let value = value.split_at(value.len() - 2).0;
                    not_found_headers
                        .remove(key)
                        .map(|actual_value| assert_eq!(actual_value, value));
                }

                assert_eq!(not_found_headers.len(), 0);

                let mut body_buffer = vec![0u8; exchange.request_body.len()];
                reader.read_exact(&mut body_buffer).unwrap();
                assert_eq!(body_buffer.deref(), exchange.request_body);

                write!(
                    socket,
                    "HTTP/1.1 {} {}\r\n",
                    exchange.response_status_code, exchange.response_reason_phase
                )
                .unwrap();

                exchange.response_headers.iter().for_each(|(key, value)| {
                    write!(socket, "{}: {}\r\n", key, value).unwrap();
                });

                write!(socket, "\r\n").unwrap();
                socket.write(exchange.response_body).unwrap();
            }
        })
    }
}
