#[cfg(any(target_os = "macos", target_os = "ios"))]
#[macro_use]
extern crate objc;
#[cfg(any(target_os = "macos", target_os = "ios"))]
extern crate block;
#[cfg(any(target_os = "macos", target_os = "ios"))]
extern crate objc_foundation;
#[cfg(any(target_os = "macos", target_os = "ios"))]
extern crate objc_id;

#[cfg(target_os = "linux")]
extern crate curl;
#[cfg(target_os = "linux")]
extern crate libc;

#[cfg(target_os = "windows")]
extern crate winapi;

use std::fmt::{Debug, Formatter};

#[cfg(target_os = "linux")]
#[path = "linux/mod.rs"]
mod imp;
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "macos/mod.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows/mod.rs"]
mod imp;

pub struct AsyncSession(imp::AsyncSession);

pub struct Session(imp::Session);

pub struct AsyncRequestBuilder<'s>(imp::AsyncRequestBuilder<'s>);

pub struct RequestBuilder<'s, 'd>(imp::RequestBuilder<'s, 'd>);

pub struct Response(imp::Response);

pub struct Headers<'a>(imp::Headers<'a>);

pub struct Error(imp::Error);

unsafe impl Send for Response {}

unsafe impl Send for Error {}

impl AsyncSession {
    #[inline]
    pub fn new() -> AsyncSession {
        AsyncSession(imp::AsyncSession::new())
    }

    #[inline]
    pub fn request<'s, 'd>(&'s self, method: &str, url: &str) -> AsyncRequestBuilder<'s> {
        AsyncRequestBuilder(self.0.request(method, url))
    }
}

impl Session {
    #[inline]
    pub fn new() -> Session {
        Session(imp::Session::new())
    }

    #[inline]
    pub fn request<'s, 'd>(&'s self, method: &str, url: &str) -> RequestBuilder<'s, 'd> {
        RequestBuilder(self.0.request(method, url))
    }
}

impl<'s> AsyncRequestBuilder<'s> {
    #[inline]
    pub fn header(mut self, key: &str, value: &str) -> Self {
        AsyncRequestBuilder(self.0.header(key, value))
    }

    #[inline]
    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        AsyncRequestBuilder(self.0.body_vec(data))
    }

    #[inline]
    pub fn send<T>(mut self, callback: T)
    where
        T: Fn(Result<Response, Error>) + Send + 'static,
    {
        self.0
            .send(move |result| callback(result.map(Response).map_err(Error)))
    }
}

impl<'s, 'd> RequestBuilder<'s, 'd> {
    #[inline]
    pub fn header(mut self, key: &str, value: &str) -> Self {
        RequestBuilder(self.0.header(key, value))
    }

    #[inline]
    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        RequestBuilder(self.0.body_vec(data))
    }

    #[inline]
    pub fn body_bytes(mut self, data: &'d [u8]) -> Self {
        RequestBuilder(self.0.body_bytes(data))
    }

    #[inline]
    pub fn send(mut self) -> Result<Response, Error> {
        self.0.send().map(Response).map_err(Error)
    }
}

impl<'a> Response {
    #[inline]
    pub fn status_code(&self) -> u32 {
        self.0.status_code()
    }

    #[inline]
    pub fn body(&self) -> &[u8] {
        self.0.body()
    }

    #[inline]
    pub fn headers(&'a self) -> Headers<'a> {
        Headers(self.0.headers())
    }
}

impl<'a> Headers<'a> {
    #[inline]
    pub fn list(&self) -> Vec<&str> {
        self.0.list()
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key)
    }
}

impl Debug for Error {
    #[inline]
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::ops::Deref;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::thread::JoinHandle;
    use std::time::Duration;

    #[test]
    fn happy_path_sync() {
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
                request_method: "POST",
                request_body: "1234".as_bytes(),
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

        let body = b"1234".to_vec();

        let response = session
            .request("POST", "http://localhost:45362/test")
            .body_vec(body)
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
    fn happy_path_async() {
        let session = AsyncSession::new();
        let (tx, rx) = channel();

        let tx_ = tx.clone();
        session
            .request(
                "POST",
                "http://www.httpbin.org/anything?param1=val1&arg2=123",
            )
            .header("Head", "Value1")
            .body_vec("Hello bin!!".as_bytes().to_vec())
            .send(move |res| {
                let res = res.unwrap();
                eprintln!("{}", String::from_utf8_lossy(res.body()));
                eprintln!("{:?}", res.status_code());
                eprintln!("{:?}", res.headers().list());
                tx_.send(()).unwrap();
            });
        let tx_ = tx.clone();
        session
            .request("POST", "http://www.httpbin.org/post")
            .send(move |res| {
                let _res = res.unwrap();
                tx_.send(()).unwrap();
            });
        let tx_ = tx.clone();
        session
            .request("DELETE", "http://www.httpbin.org/delete")
            .send(move |res| {
                let _res = res.unwrap();
                tx_.send(()).unwrap();
            });

        thread::sleep(Duration::from_millis(100));
        drop(session);

        for _i in 0..3 {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }

    #[test]
    #[should_panic]
    fn error_unsupported_url_sync() {
        Session::new().request("GET", "moz://a").send().unwrap();
    }

    #[test]
    #[should_panic]
    fn error_unsupported_url_async() {
        let session = AsyncSession::new();
        let (tx, rx) = channel();

        session.request("GET", "moz://a").send(move |res| {
            tx.send(res).unwrap();
        });

        rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
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
