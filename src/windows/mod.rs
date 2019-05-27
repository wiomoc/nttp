use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fmt::{Debug, Formatter};
use std::iter::once;
use std::marker::PhantomData;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use winapi::ctypes::c_void;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::libloaderapi::{FreeLibrary, LoadLibraryW};
use winapi::um::winbase::{
    FormatMessageA, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_HMODULE,
    FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use winapi::um::winhttp::{
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetStatusCallback, HINTERNET,
    LPURL_COMPONENTS, URL_COMPONENTS, WINHTTP_CALLBACK_FLAG_ALL_COMPLETIONS,
    WINHTTP_CALLBACK_FLAG_REDIRECT, WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE,
    WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE, WINHTTP_CALLBACK_STATUS_READ_COMPLETE,
    WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE, WINHTTP_FLAG_ASYNC, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_RAW_HEADERS_CRLF, WINHTTP_QUERY_STATUS_CODE,
};

mod punycode;

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
    WinAPI(u32),
}

unsafe impl Send for Response {}

unsafe impl Send for Error {}

fn to_wide_string(string: &str) -> Vec<u16> {
    OsStr::new(string).encode_wide().chain(once(0)).collect()
}

fn win_result_bool(status: i32) -> Result<(), Error> {
    if status == 1 {
        Ok(())
    } else {
        Err(Error::WinAPI(unsafe { GetLastError() }))
    }
}

fn win_result_ptr<T>(ptr: *mut T) -> Result<*mut T, Error> {
    if ptr.is_null() {
        Err(Error::WinAPI(unsafe { GetLastError() }))
    } else {
        Ok(ptr)
    }
}

impl Session {
    pub fn new() -> Session {
        let session =
            win_result_ptr(unsafe { WinHttpOpen(wstrz!("nttp").as_ptr(), 1, null(), null(), 0) })
                .unwrap();

        Session { session }
    }

    pub fn request<'s, 'd>(
        &'s self,
        method: &str,
        url: &str,
    ) -> Result<RequestBuilder<'s, 'd>, Error> {
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

fn create_request(
    session: HINTERNET,
    method: &str,
    url: &str,
) -> Result<(HINTERNET, HINTERNET), Error> {
    let url = to_wide_string(url);
    let method = to_wide_string(method);
    unsafe {
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

        win_result_bool(WinHttpCrackUrl(
            url.as_ptr(),
            0,
            0,
            &mut url_component as LPURL_COMPONENTS,
        ))?;

        let host = punycode::encode(url_component.lpszHostName, url_component.dwHostNameLength);

        let connection = win_result_ptr(WinHttpConnect(
            session,
            host.as_ptr(),
            url_component.nPort,
            0,
        ))?;

        let request = WinHttpOpenRequest(
            connection,
            method.as_ptr(),
            url_component.lpszUrlPath,
            null(),
            null(),
            null_mut(),
            0,
        );

        Ok((connection, request))
    }
}

impl AsyncSession {
    pub fn new() -> AsyncSession {
        let session = win_result_ptr(unsafe {
            WinHttpOpen(
                wstrz!("nttp").as_ptr(),
                1,
                null(),
                null(),
                WINHTTP_FLAG_ASYNC,
            )
        })
        .unwrap();

        AsyncSession { session }
    }

    pub fn request<'s>(
        &'s self,
        method: &str,
        url: &str,
    ) -> Result<AsyncRequestBuilder<'s>, Error> {
        AsyncRequestBuilder::new(self.session, method, url)
    }
}

impl<'s, 'd> RequestBuilder<'s, 'd> {
    fn new(session: HINTERNET, method: &str, url: &str) -> Result<RequestBuilder<'s, 'd>, Error> {
        let (connection, request) = create_request(session, method, url)?;

        Ok(RequestBuilder {
            connection,
            request,
            body: Cow::Borrowed(&[]),
            _session_marker: PhantomData,
        })
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        let header = to_wide_string(format!("{}: {}", key, value).as_str());
        win_result_bool(unsafe {
            WinHttpAddRequestHeaders(
                self.request,
                header.as_ptr(),
                MINUS_ONE,
                WINHTTP_ADDREQ_FLAG_ADD,
            )
        })
        .unwrap();
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
            win_result_bool(WinHttpSendRequest(
                self.request,
                null(),
                0,
                self.body.as_ptr() as *mut c_void,
                self.body.len() as u32,
                self.body.len() as u32,
                0,
            ))?;

            win_result_bool(WinHttpReceiveResponse(self.request, null_mut()))?;

            let mut data_avaliable: u32 = 0;
            win_result_bool(WinHttpQueryDataAvailable(
                self.request,
                &mut data_avaliable as *mut u32,
            ))?;

            let mut body = vec![0u8; data_avaliable as usize];
            let mut data_read: u32 = 0;
            win_result_bool(WinHttpReadData(
                self.request,
                body.as_mut_ptr() as *mut c_void,
                data_avaliable,
                &mut data_read as *mut u32,
            ))?;

            let (status_code, headers) = read_headers(self.request)?;

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

fn read_headers(request: HINTERNET) -> Result<(u32, HashMap<String, String>), Error> {
    let (status_code, headers_raw) = unsafe {
        let mut status_code: u32 = 0;
        let i32_size: u32 = 4;

        win_result_bool(WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status_code as *mut u32 as *mut c_void,
            &i32_size as *const u32 as *mut u32,
            null_mut(),
        ))?;

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
        win_result_bool(WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            null_mut(),
            headers_raw.as_mut_ptr() as *mut c_void,
            &mut header_size as *mut u32,
            null_mut(),
        ))?;
        (status_code, headers_raw)
    };

    let mut headers = HashMap::new();

    for header in String::from_utf16(&headers_raw[..])
        .map_err(|_| Error::InvalidHeader)?
        .lines()
        .skip(1)
        .filter(|x| !x.is_empty())
    {
        if header != "\0" {
            if let Some(seperator_pos) = header.find(':') {
                let (key, value) = header.split_at(seperator_pos);
                // Remove ": "
                let value = value.split_at(2).1;
                // Remove "\r\n"
                let value = value.split_at(value.len()).0;

                headers.insert(key.to_string(), value.to_string());
            } else {
                return Err(Error::InvalidHeader);
            }
        }
    }

    Ok((status_code, headers))
}

impl<'s> AsyncRequestBuilder<'s> {
    fn new(session: HINTERNET, method: &str, url: &str) -> Result<AsyncRequestBuilder<'s>, Error> {
        let (_, request) = create_request(session, method, url)?;

        Ok(AsyncRequestBuilder {
            request,
            body: vec![],
            _session_marker: PhantomData,
        })
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        let header = to_wide_string(format!("{}: {}", key, value).as_str());
        win_result_bool(unsafe {
            WinHttpAddRequestHeaders(
                self.request,
                header.as_ptr(),
                MINUS_ONE,
                WINHTTP_ADDREQ_FLAG_ADD,
            )
        })
        .unwrap();
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
        _info_len: u32,
    ) {
        let mut exchange = Box::from_raw(context as *mut Exchange);

        match status {
            WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE => {
                win_result_bool(WinHttpReceiveResponse(exchange.request, null_mut())).unwrap();
                mem::forget(exchange);
            }
            WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE => {
                let (status_code, headers) = read_headers(exchange.request).unwrap();
                exchange.status_code = status_code;
                exchange.headers = Some(headers);
                win_result_bool(WinHttpQueryDataAvailable(exchange.request, null_mut())).unwrap();
                mem::forget(exchange);
            }
            WINHTTP_CALLBACK_STATUS_DATA_AVAILABLE => {
                let available_bytes = *(info as *mut u32);
                if available_bytes == 0 {
                    let response = Response {
                        status_code: exchange.status_code,
                        headers: exchange.headers.unwrap(),
                        body: exchange.response_body,
                    };

                    WinHttpSetStatusCallback(exchange.request, None, 0, 0);
                    WinHttpCloseHandle(exchange.request);
                    WinHttpCloseHandle(connection);
                    (exchange.callback)(Ok(response));
                } else {
                    exchange.response_body.reserve(available_bytes as usize);

                    let mut bytes_read: u32 = 0;

                    win_result_bool(WinHttpReadData(
                        exchange.request,
                        exchange
                            .response_body
                            .as_ptr()
                            .offset(exchange.response_body.len() as isize)
                            as *mut c_void,
                        available_bytes,
                        &mut bytes_read as *mut u32,
                    ))
                    .unwrap();

                    let response_body = Vec::from_raw_parts(
                        exchange.response_body.as_mut_ptr(),
                        exchange.response_body.len() + bytes_read as usize,
                        exchange.response_body.capacity(),
                    );

                    mem::forget(exchange.response_body);
                    exchange.response_body = response_body;
                    mem::forget(exchange);
                }
            }

            WINHTTP_CALLBACK_STATUS_READ_COMPLETE => {
                win_result_bool(WinHttpQueryDataAvailable(exchange.request, null_mut())).unwrap();
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
            request_body: self.body,
            response_body: Vec::new(),
        };

        unsafe {
            WinHttpSetStatusCallback(
                self.request,
                Some(AsyncRequestBuilder::winhttp_callback::<T>),
                WINHTTP_CALLBACK_FLAG_ALL_COMPLETIONS | WINHTTP_CALLBACK_FLAG_REDIRECT,
                0,
            );

            win_result_bool(WinHttpSendRequest(
                self.request,
                null(),
                0,
                exchange.request_body.as_ptr() as *mut c_void,
                exchange.request_body.len() as u32,
                exchange.request_body.len() as u32,
                Box::into_raw(Box::new(exchange)) as usize,
            ))
            .unwrap();
        };
    }
}

struct Exchange {
    callback: Box<Fn(Result<Response, Error>) + Send + 'static>,
    request: HINTERNET,
    status_code: u32,
    headers: Option<HashMap<String, String>>,
    request_body: Vec<u8>,
    response_body: Vec<u8>,
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
        match self {
            Error::WinAPI(code) => {
                let mut buffer: *mut i8 = null_mut();

                let error_from_winhttp = *code >= 12001 && *code <= 12156; // 12001 to 12156 are WinHTTP errors
                let dll = if error_from_winhttp {
                    unsafe { LoadLibraryW(wstrz!("wininet.dll").as_ptr()) }
                } else {
                    null_mut()
                };
                let result = if unsafe {
                    FormatMessageA(
                        FORMAT_MESSAGE_ALLOCATE_BUFFER
                            | if error_from_winhttp {
                                FORMAT_MESSAGE_FROM_HMODULE
                            } else {
                                FORMAT_MESSAGE_FROM_SYSTEM
                            }
                            | FORMAT_MESSAGE_IGNORE_INSERTS,
                        dll as *const c_void,
                        *code,
                        0x400, // Userdefault locale
                        &mut buffer as *mut *mut i8 as *mut i8,
                        0,
                        null_mut(),
                    )
                } == 1
                {
                    f.write_fmt(format_args!(
                        "Getting error {} while formating error message {}",
                        unsafe { GetLastError() },
                        *code
                    ))
                } else {
                    f.write_str(&unsafe { CString::from_raw(buffer) }.to_string_lossy())
                };

                if error_from_winhttp {
                    unsafe { FreeLibrary(dll) };
                }

                result
            }

            Error::InvalidHeader => f.write_str("Received Header had invalid format"),
        }
    }
}
