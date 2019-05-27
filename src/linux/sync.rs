use curl::easy::Easy;
use curl::easy::List;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use crate::imp::{Error, Response, SendMutRef, SendSlice};
use std::marker::PhantomData;

pub struct Session {}

pub struct RequestBuilder<'s, 'd> {
    easy: Easy,
    headers: List,
    _session_marker: PhantomData<&'s Session>,
    _data_marker: PhantomData<&'d u8>,
}

impl Session {
    pub fn new() -> Session {
        Session {}
    }

    pub fn request<'s, 'd>(
        &'s self,
        method: &str,
        url: &str,
    ) -> Result<RequestBuilder<'s, 'd>, Error> {
        RequestBuilder::new(method, url)
    }
}

impl<'s, 'd> RequestBuilder<'s, 'd> {
    pub fn new(method: &str, url: &str) -> Result<RequestBuilder<'s, 'd>, Error> {
        let mut easy = Easy::new();
        easy.url(url).map_err(Error)?;
        match method {
            "GET" => easy.get(true),
            "POST" => easy.post(true),
            "PUT" => easy.put(true),
            _ => easy.custom_request(method),
        }
        .map_err(Error)?;

        Ok(RequestBuilder {
            easy,
            headers: List::new(),
            _session_marker: PhantomData,
            _data_marker: PhantomData,
        })
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers
            .append(format!("{}: {}", key, value).as_str())
            .unwrap();
        self
    }

    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        let mut data = Cursor::new(data);
        self.easy
            .read_function(move |out| Ok(data.read(out).unwrap()))
            .unwrap();
        self
    }

    pub fn body_bytes(mut self, data: &'d [u8]) -> Self {
        let mut pos = 0;
        let data_ = SendSlice::new(data);

        self.easy
            .read_function(move |mut out| {
                let data = unsafe { data_.deref() };
                let written = out.write(&data[pos..]).unwrap();
                pos += written;
                Ok(written)
            })
            .unwrap();
        self
    }

    pub fn send(mut self) -> Result<Response, Error> {
        self.easy.http_headers(self.headers).unwrap();

        let mut response_body = Vec::new();
        let response_body_ = SendMutRef::new(&mut response_body);

        self.easy
            .write_function(move |input| {
                let response_body = unsafe { response_body_.deref() };
                response_body.extend_from_slice(input);
                Ok(input.len())
            })
            .map_err(Error)?;

        let mut headers = HashMap::new();
        let headers_ = SendMutRef(&mut headers);
        let mut first = true;
        self.easy
            .header_function(move |input| {
                parse_header(input, &mut first, unsafe { headers_.deref() })
            })
            .map_err(Error)?;

        self.easy.perform().map_err(Error)?;

        let status_code = self.easy.response_code().map_err(Error)?;

        Ok(Response {
            status_code,
            headers,
            body: response_body,
        })
    }
}

pub(crate) fn parse_header(
    input: &[u8],
    first: &mut bool,
    headers: &mut HashMap<String, String>,
) -> bool {
    if *first || input == b"\r\n" {
        *first = false;
        return true;
    }
    if let Some(seperator_pos) = input.iter().position(|x| *x == b':') {
        let (key, value) = input.split_at(seperator_pos);
        let key = String::from_utf8_lossy(key);
        // Remove ": "
        let value = value.split_at(2).1;
        // Remove "\r\n"
        let value = String::from_utf8_lossy(value.split_at(value.len() - 2).0);

        headers.insert(key.into_owned(), value.into_owned());
        true
    } else {
        false
    }
}
