use block::ConcreteBlock;
use core::borrow::Borrow;
use core::fmt::Write;
use objc::runtime::Object;
use objc_foundation::{
    INSData, INSDictionary, INSString, NSData, NSDictionary, NSObject, NSString,
};
use objc_id::Id;
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::mpsc::sync_channel;

pub struct AsyncSession {
    session: Id<NSObject>,
}

pub struct Session(AsyncSession);

pub struct AsyncRequestBuilder<'s> {
    session: &'s Id<NSObject>,
    request: Id<NSObject>,
}

pub struct RequestBuilder<'s, 'd> {
    inner: AsyncRequestBuilder<'s>,
    _data_marker: PhantomData<&'d [u8]>,
}

pub struct Response {
    data: Id<NSData>,
    response: Id<NSObject>,
}

pub struct Headers<'a> {
    headers: Id<NSDictionary<NSString, NSString>>,
    _marker: PhantomData<&'a u8>,
}

pub struct Error {
    error: Id<NSObject>,
}

unsafe impl Send for Response {}

unsafe impl Send for Error {}

impl AsyncSession {
    pub fn new() -> AsyncSession {
        unsafe {
            let configuration: *mut Object = msg_send![
                class!(NSURLSessionConfiguration),
                defaultSessionConfiguration
            ];
            let session: *mut NSObject = msg_send![
                class!(NSURLSession),
                sessionWithConfiguration: configuration
            ];

            AsyncSession {
                session: Id::from_ptr(session),
            }
        }
    }

    #[inline]
    pub fn request<'s>(
        &'s self,
        method: &str,
        url: &str,
    ) -> Result<AsyncRequestBuilder<'s>, Error> {
        Ok(AsyncRequestBuilder::new(&self.session, method, url))
    }
}

impl Drop for AsyncSession {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.session, finishTasksAndInvalidate];
        }
    }
}

impl Session {
    #[inline]
    pub fn new() -> Session {
        Session(AsyncSession::new())
    }

    #[inline]
    pub fn request<'s, 'd>(
        &'s self,
        method: &str,
        url: &str,
    ) -> Result<RequestBuilder<'s, 'd>, Error> {
        Ok(RequestBuilder::new(&self.0.session, method, url))
    }
}

impl<'s> AsyncRequestBuilder<'s> {
    fn new(session: &'s Id<NSObject>, method: &str, url: &str) -> AsyncRequestBuilder<'s> {
        unsafe {
            let url: *mut Object = msg_send![class!(NSURL), URLWithString: NSString::from_str(url)];
            let uninitialized_request: *mut Object = msg_send![class!(NSMutableURLRequest), alloc];
            let request: *mut NSObject = msg_send![uninitialized_request, initWithURL: url];
            msg_send![request, setHTTPMethod: NSString::from_str(method)];
            AsyncRequestBuilder {
                session,
                request: Id::<NSObject>::from_retained_ptr(request),
            }
        }
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        unsafe {
            msg_send![self.request, addValue:NSString::from_str(value) forHTTPHeaderField:NSString::from_str(key)];
        }
        self
    }

    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        unsafe {
            msg_send![self.request, setHTTPBody: NSData::from_vec(data)];
        }
        self
    }

    pub fn send<T>(mut self, callback: T)
    where
        T: Fn(Result<Response, Error>) + Send + 'static,
    {
        unsafe {
            let completion_handler = ConcreteBlock::new(
                move |data: *mut NSData, response: *mut NSObject, error: *mut NSObject| {
                    callback(if response.is_null() {
                        let error = Id::<NSObject>::from_ptr(error);
                        Result::Err(Error { error })
                    } else {
                        let data = Id::<NSData>::from_ptr(data);
                        let response = Id::<NSObject>::from_ptr(response);

                        Result::Ok(Response { data, response })
                    });
                },
            );

            let data_task: *mut Object = msg_send![self.session.deref(), dataTaskWithRequest: self.request completionHandler: completion_handler.copy()];
            let _: () = msg_send![data_task, resume];
        }
    }
}

impl<'s, 'd> RequestBuilder<'s, 'd> {
    #[inline]
    fn new(session: &'s Id<NSObject>, method: &str, url: &str) -> RequestBuilder<'s, 'd> {
        RequestBuilder {
            inner: AsyncRequestBuilder::new(session, method, url),
            _data_marker: PhantomData,
        }
    }

    #[inline]
    pub fn header(mut self, key: &str, value: &str) -> Self {
        RequestBuilder {
            inner: self.inner.header(key, value),
            _data_marker: PhantomData,
        }
    }

    #[inline]
    pub fn body_vec(mut self, data: Vec<u8>) -> Self {
        RequestBuilder {
            inner: self.inner.body_vec(data),
            _data_marker: PhantomData,
        }
    }

    pub fn body_bytes(mut self, data: &'d [u8]) -> Self {
        unsafe {
            let ns_data: *mut NSData = msg_send![class!(NSData), dataWithBytesNoCopy:data.as_ptr() length:data.len() freeWhenDone:false];
            msg_send![self.inner.request, setHTTPBody: ns_data];
        }
        self
    }

    pub fn send(mut self) -> Result<Response, Error> {
        let (tx, rx) = sync_channel(1);
        self.inner.send(move |result| tx.send(result).unwrap());

        let response = rx.recv().unwrap();
        response
    }
}

impl<'a> Response {
    pub fn status_code(&self) -> u32 {
        unsafe { msg_send![self.response, statusCode] }
    }

    pub fn body(&self) -> &[u8] {
        self.data.bytes()
    }

    pub fn headers(&'a self) -> Headers<'a> {
        let headers: Id<NSDictionary<NSString, NSString>> =
            unsafe { Id::from_ptr(msg_send![self.response, allHeaderFields]) };
        Headers {
            headers,
            _marker: PhantomData,
        }
    }
}

impl<'a> Headers<'a> {
    pub fn list(&self) -> Vec<&str> {
        self.headers.keys().iter().map(|key| key.as_str()).collect()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.headers
            .object_for(NSString::from_str(key).borrow())
            .map(NSString::as_str)
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        let domain: Id<NSString> = unsafe { msg_send![self.error, domain] };
        f.write_str(domain.as_str())?;

        let localized_description: Id<NSString> =
            unsafe { msg_send![self.error, localizedDescription] };
        f.write_char(' ')?;
        f.write_str(localized_description.as_str())?;
        Ok(())
    }
}
