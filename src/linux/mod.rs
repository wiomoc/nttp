use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

mod asynchron;
mod sync;

pub use asynchron::*;
pub use sync::*;

#[derive(Copy, Clone)]
pub(crate) struct SendMutRef<T>(*mut T);

impl<T> SendMutRef<T> {
    pub(crate) fn new(obj: &mut T) -> SendMutRef<T> {
        SendMutRef(obj as *mut T)
    }

    pub(crate) unsafe fn deref(&self) -> &'static mut T {
        &mut *self.0
    }
}

unsafe impl<T> Send for SendMutRef<T> where T: Send {}

#[derive(Copy, Clone)]
struct SendSlice(*const u8, usize);

impl SendSlice {
    fn new(obj: &[u8]) -> SendSlice {
        SendSlice(obj.as_ptr(), obj.len())
    }

    unsafe fn deref(&self) -> &'static [u8] {
        std::slice::from_raw_parts(self.0, self.1)
    }
}

unsafe impl Send for SendSlice {}

pub struct Response {
    body: Vec<u8>,
    status_code: u32,
    headers: HashMap<String, String>,
}

pub struct Headers<'a> {
    headers: &'a HashMap<String, String>,
}

pub struct Error(curl::Error);

impl Response {
    pub fn headers(&self) -> Headers {
        Headers {
            headers: &self.headers,
        }
    }

    pub fn status_code(&self) -> u32 {
        self.status_code
    }

    pub fn body(&self) -> &[u8] {
        &self.body
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

unsafe impl Send for Error {}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        self.0.fmt(f)
    }
}
