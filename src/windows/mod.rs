use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::{Debug, Formatter};
use std::iter::once;
use std::marker::PhantomData;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use winapi::ctypes::c_void;
use winapi::um::winhttp::{
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetStatusCallback, HINTERNET,
    LPURL_COMPONENTS, URL_COMPONENTS, WINHTTP_CALLBACK_FLAG_ALL_COMPLETIONS,
    WINHTTP_CALLBACK_FLAG_REDIRECT, WINHTTP_CALLBACK_STATUS_CONNECTED_TO_SERVER,
    WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE, WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE,
    WINHTTP_CALLBACK_STATUS_READ_COMPLETE, WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE,
    WINHTTP_FLAG_ASYNC, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_RAW_HEADERS_CRLF, WINHTTP_QUERY_STATUS_CODE,
};

const WINHTTP_ADDREQ_FLAG_ADD: u32 = 0x20000000;
const MINUS_ONE: u32 = 0xFFFFFFFF;

pub struct Session {
    session: HINTERNET,
}

pub struct AsyncSession {
    session: HINTERNET,
}

pub struct RequestBuilder<'s, 'd> {
    connection: HINTERNET,
    request: HINTERNET,
    body: Cow<'d, [u8]>,
    _session_marker: PhantomData<&'s Session>,
}

pub struct AsyncRequestBuilder<'s> {
    connection: HINTERNET,
    request: HINTERNET,
    body: Vec<u8>,
    _session_marker: PhantomData<&'s Session>,
}

pub struct Response {
    body: Vec<u8>,
    status_code: u32,
    headers: HashMap<String, String>,
}

pub struct Headers<'a> {
    headers: &'a HashMap<String, String>,
}

pub enum Error {
    InvalidHeader,
}

unsafe impl Send for Response {}

unsafe impl Send for Error {}

fn to_wide_string(string: &str) -> Vec<u16> {
    OsStr::new(string).encode_wide().chain(once(0)).collect()
}

impl Session {
    pub fn new() -> Session {
        let agent = to_wide_string("nttp");
        let session = unsafe { WinHttpOpen(agent.as_ptr(), 1, null(), null(), 0) };

        Session { session }
    }

    pub fn request<'s, 'd>(&'s self, method: &str, url: &str) -> RequestBuilder<'s, 'd> {
        RequestBuilder::new(self.session, method, url)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.session);
        }
    }
}

impl AsyncSession {
    pub fn new() -> AsyncSession {
        let agent = to_wide_string("nttp");
        let session = unsafe { WinHttpOpen(agent.as_ptr(), 1, null(), null(), WINHTTP_FLAG_ASYNC) };

        AsyncSession { session }
    }

    pub fn request<'s>(&'s self, method: &str, url: &str) -> AsyncRequestBuilder<'s> {
        AsyncRequestBuilder::new(self.session, method, url)
    }
}

impl<'s, 'd> RequestBuilder<'s, 'd> {
    fn new(session: HINTERNET, method: &str, url: &str) -> RequestBuilder<'s, 'd> {
        unsafe {
            let url = to_wide_string(url);
            let mut url_component = URL_COMPONENTS {
                dwStructSize: (mem::size_of::<URL_COMPONENTS>() as u32),
                lpszScheme: null_mut(),
                dwSchemeLength: 0,
                nScheme: 0,
                lpszHostName: null_mut(),
                dwHostNameLength: MINUS_ONE,
                nPort: 0,
                lpszUserName: null_mut(),
                dwUserNameLength: 0,
                lpszPassword: null_mut(),
                dwPasswordLength: 0,
                lpszUrlPath: null_mut(),
                dwUrlPathLength: MINUS_ONE,
                lpszExtraInfo: null_mut(),
                dwExtraInfoLength: 0,
            };

            WinHttpCrackUrl(url.as_ptr(), 0, 0, &mut url_component as LPURL_COMPONENTS);
            //TODO Punycode
            if url_component.lpszHostName.is_null() {
                panic!("Invalid Url");
            }

            // lpszHostName is a pointer in `url` so we're able to add an '\0' at dwHostNameLength, because dwHostNameLength >= `url.len()`
            let host = std::slice::from_raw_parts_mut(
                url_component.lpszHostName,
                url_component.dwHostNameLength as usize + 1,
            );
            host[host.len() - 1] = 0;

            let connection = WinHttpConnect(session, host.as_ptr(), url_component.nPort, 0);

            let method = to_wide_string(method);
            let request = WinHttpOpenRequest(
                connection,
                method.as_ptr(),
                url_component.lpszUrlPath.offset(1),
                null(),
                null(),
                null_mut(),
                0,
            );

            RequestBuilder {
                connection,
                request,
                body: Cow::Borrowed(&[]),
                _session_marker: PhantomData,
            }
        }
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        let header = to_wide_string(format!("{}: {}", key, value).as_str());
        unsafe {
            WinHttpAddRequestHeaders(
                self.request,
                header.as_ptr(),
                MINUS_ONE,
                WINHTTP_ADDREQ_FLAG_ADD,
            );
        }
        self
    }

    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        self.body = Cow::Owned(data);
        self
    }

    pub fn body_bytes(mut self, data: &'d [u8]) -> Self {
        self.body = Cow::Borrowed(data);
        self
    }

    pub fn send(mut self) -> Result<Response, Error> {
        let (status_code, headers, body) = unsafe {
            WinHttpSendRequest(
                self.request,
                null(),
                0,
                self.body.as_ptr() as *mut c_void,
                self.body.len() as u32,
                self.body.len() as u32,
                0,
            );

            WinHttpReceiveResponse(self.request, null_mut());

            let mut data_avaliable: u32 = 0;
            WinHttpQueryDataAvailable(self.request, &mut data_avaliable as *mut u32);

            let mut body = vec![0u8; data_avaliable as usize];
            let mut data_read: u32 = 0;
            WinHttpReadData(
                self.request,
                body.as_mut_ptr() as *mut c_void,
                data_avaliable,
                &mut data_read as *mut u32,
            );

            let (status_code, headers) = read_headers(self.request);

            WinHttpCloseHandle(self.request);
            WinHttpCloseHandle(self.connection);

            (status_code, headers, body)
        };

        Ok(Response {
            body,
            status_code,
            headers,
        })
    }
}

fn read_headers(request: HINTERNET) -> (u32, HashMap<String, String>) {
    let (status_code, headers_raw) = unsafe {
        let mut status_code: u32 = 0;
        let i32_size: u32 = 4;

        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status_code as *mut u32 as *mut c_void,
            &i32_size as *const u32 as *mut u32,
            null_mut(),
        );

        let mut header_size: u32 = 0;
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            null_mut(),
            null_mut(),
            &header_size as *const u32 as *mut u32,
            null_mut(),
        );

        let mut headers_raw = vec![0u16; header_size as usize / 2];
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            null_mut(),
            headers_raw.as_mut_ptr() as *mut c_void,
            &mut header_size as *mut u32,
            null_mut(),
        );
        (status_code, headers_raw)
    };

    let mut headers = HashMap::new();

    String::from_utf16(&headers_raw[..])
        .unwrap()
        .lines()
        .skip(1)
        .filter(|x| !x.is_empty())
        .for_each(|header| {
            if let Some(seperator_pos) = header.find(':') {
                let (key, value) = header.split_at(seperator_pos);
                // Remove ": "
                let value = value.split_at(2).1;
                // Remove "\r\n"
                let value = value.split_at(value.len()).0;

                headers.insert(key.to_string(), value.to_string());
            }
        });

    (status_code, headers)
}

impl<'s> AsyncRequestBuilder<'s> {
    fn new(session: HINTERNET, method: &str, url: &str) -> AsyncRequestBuilder<'s> {
        unsafe {
            let url = to_wide_string(url);
            let mut url_component = URL_COMPONENTS {
                dwStructSize: (mem::size_of::<URL_COMPONENTS>() as u32),
                lpszScheme: null_mut(),
                dwSchemeLength: 0,
                nScheme: 0,
                lpszHostName: null_mut(),
                dwHostNameLength: MINUS_ONE,
                nPort: 0,
                lpszUserName: null_mut(),
                dwUserNameLength: 0,
                lpszPassword: null_mut(),
                dwPasswordLength: 0,
                lpszUrlPath: null_mut(),
                dwUrlPathLength: MINUS_ONE,
                lpszExtraInfo: null_mut(),
                dwExtraInfoLength: 0,
            };

            WinHttpCrackUrl(url.as_ptr(), 0, 0, &mut url_component as LPURL_COMPONENTS);
            //TODO Punycode
            if url_component.lpszHostName.is_null() {
                panic!("Invalid Url");
            }

            // lpszHostName is a pointer in `url` so we're able to add an '\0' at dwHostNameLength, because dwHostNameLength >= `url.len()`
            let host = std::slice::from_raw_parts_mut(
                url_component.lpszHostName,
                url_component.dwHostNameLength as usize + 1,
            );
            host[host.len() - 1] = 0;

            let connection = WinHttpConnect(session, host.as_ptr(), url_component.nPort, 0);

            let method = to_wide_string(method);
            let request = WinHttpOpenRequest(
                connection,
                method.as_ptr(),
                url_component.lpszUrlPath.offset(1),
                null(),
                null(),
                null_mut(),
                0,
            );

            AsyncRequestBuilder {
                connection,
                request,
                body: vec![],
                _session_marker: PhantomData,
            }
        }
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        self
    }

    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        self.body = data;
        self
    }

    unsafe extern "system" fn winhttp_callback<T>(
        connection: HINTERNET,
        context: usize,
        status: u32,
        info: *mut c_void,
        info_len: u32,
    ) {
        let mut exchange = Box::from_raw(context as *mut Exchange);

        match status {
            WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE => {
                WinHttpReceiveResponse(exchange.request, null_mut());
                mem::forget(exchange);
            }
            WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE => {
                let (status_code, headers) = read_headers(exchange.request);
                exchange.status_code = status_code;
                exchange.headers = Some(headers);
                WinHttpQueryDataAvailable(exchange.request, null_mut());
                mem::forget(exchange);
            }
            WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE => {
                let available_bytes = *(info as *mut u32);
                if available_bytes == 0 {
                    let response = Response {
                        status_code: exchange.status_code,
                        headers: exchange.headers.unwrap(),
                        body: exchange.body,
                    };

                    WinHttpSetStatusCallback(exchange.request, None, 0, 0);
                    WinHttpCloseHandle(exchange.request);
                    (exchange.callback)(Ok(response));
                } else {
                    exchange.body.reserve(available_bytes as usize);

                    let mut bytes_read: u32 = 0;

                    WinHttpReadData(
                        exchange.request,
                        exchange.body.as_ptr().offset(exchange.body.len() as isize) as *mut c_void,
                        available_bytes,
                        &mut bytes_read as *mut u32,
                    );

                    let body = Vec::from_raw_parts(
                        exchange.body.as_mut_ptr(),
                        exchange.body.len() + bytes_read as usize,
                        exchange.body.capacity(),
                    );

                    mem::forget(exchange.body);
                    exchange.body = body;
                    mem::forget(exchange);
                }
            }

            WINHTTP_CALLBACK_STATUS_READ_COMPLETE => {
                WinHttpQueryDataAvailable(exchange.request, null_mut());
                mem::forget(exchange);
            }
            _ => {
                mem::forget(exchange);
            }
        }
    }

    pub fn send<T>(mut self, callback: T)
    where
        T: Fn(Result<Response, Error>) + Send + 'static,
    {
        let exchange = Exchange {
            callback: Box::new(callback),
            request: self.request,
            status_code: 0,
            headers: None,
            body: Vec::new(),
        };

        unsafe {
            WinHttpSetStatusCallback(
                self.request,
                Some(AsyncRequestBuilder::winhttp_callback::<T>),
                WINHTTP_CALLBACK_FLAG_ALL_COMPLETIONS | WINHTTP_CALLBACK_FLAG_REDIRECT,
                0,
            );

            WinHttpSendRequest(
                self.request,
                null(),
                0,
                self.body.as_ptr() as *mut c_void,
                self.body.len() as u32,
                self.body.len() as u32,
                Box::into_raw(Box::new(exchange)) as usize,
            );
        };
    }
}

struct Exchange {
    callback: Box<Fn(Result<Response, Error>) + Send + 'static>,
    request: HINTERNET,
    status_code: u32,
    headers: Option<HashMap<String, String>>,
    body: Vec<u8>,
}

impl Response {
    pub fn status_code(&self) -> u32 {
        self.status_code
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn headers(&self) -> Headers {
        Headers {
            headers: &self.headers,
        }
    }
}

impl<'a> Headers<'a> {
    pub fn list(&self) -> Vec<&str> {
        self.headers
            .keys()
            .into_iter()
            .map(|x| x.as_str())
            .collect()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).map(|x| x.as_str())
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        Ok(())
    }
}
