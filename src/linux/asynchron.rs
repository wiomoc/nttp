use crate::linux::{parse_header, Error, Response, SendMutRef};
use curl::easy::Easy;
use curl::multi::{EasyHandle, Multi, WaitFd};
use libc::{c_void, close, fcntl, pipe2, read, write, O_CLOEXEC, O_NONBLOCK};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr::null_mut;
use std::time::Duration;
use std::{mem, thread};

pub(crate) struct Sender<T> {
    fd: i32,
    phantom: PhantomData<T>,
}

pub(crate) struct Receiver<T> {
    fd: i32,
    phantom: PhantomData<T>,
}

unsafe impl<T> Send for Sender<T> where T: Send {}

unsafe impl<T> Send for Receiver<T> where T: Send {}

macro_rules! syscall {
    ($c:expr) => {
        unsafe {
            if $c == -1 {
                let errno = *libc::__errno_location();
                //let errno = *libc::__error();
                panic!("Errno: {} at {}", errno, stringify!($c));
            }
            $c
        }
    };
}

pub(crate) fn create<T>() -> (Sender<T>, Receiver<T>) {
    let mut fds = [0; 2];
    syscall!(pipe2(fds.as_mut_ptr(), O_CLOEXEC | O_NONBLOCK));
    (
        Sender {
            fd: fds[1],
            phantom: PhantomData,
        },
        Receiver {
            fd: fds[0],
            phantom: PhantomData,
        },
    )
}

impl<T> Sender<T> {
    pub(crate) fn send(&self, obj: T) -> Result<(), ()> {
        let ptr = Box::into_raw(Box::new(obj));

        let bytes_written = syscall!(write(
            self.fd,
            &ptr as *const *mut T as *const c_void,
            size_of::<*mut T>()
        ));

        if bytes_written as usize == size_of::<*mut T>() {
            Ok(())
        } else {
            Err(())
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        syscall!(close(self.fd));
    }
}

impl<T> Receiver<T> {
    pub(crate) fn recv(&self) -> Result<Box<T>, ()> {
        let mut ptr: *mut T = null_mut();
        let bytes_read = syscall!(read(
            self.fd,
            &mut ptr as *mut *mut T as *mut c_void,
            size_of::<*mut T>()
        ));
        if bytes_read as usize == size_of::<*mut T>() {
            Ok(unsafe { Box::<T>::from_raw(ptr) })
        } else {
            Err(())
        }
    }

    pub(crate) fn fd(&self) -> i32 {
        self.fd
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        syscall!(close(self.fd));
    }
}

pub struct AsyncSession {
    sender: Sender<Message>,
}

pub struct RequestBuilder<'a> {
    session: &'a AsyncSession,
    easy: Easy,
}

type CallbackFn = Fn(Result<Response, Error>) + Send;

pub struct Exchange {
    handle: Option<EasyHandle>,
    callback: Box<CallbackFn>,
    body: Vec<u8>,
    response_headers: HashMap<String, String>,
}

unsafe impl Send for Exchange {}

unsafe impl Sync for Exchange {}

enum Message {
    Easy(Easy, Box<Exchange>),
    Quit,
}

impl AsyncSession {
    pub fn new() -> AsyncSession {
        let (tx, rx) = create::<Message>();

        thread::spawn(move || {
            let mut multi = Multi::new();
            multi.pipelining(true, true).unwrap();
            let mut quit = false;
            loop {
                let mut fd = WaitFd::new();
                fd.set_fd(rx.fd());
                fd.poll_on_read(!quit);
                let mut fds = [fd];

                multi.wait(&mut fds, Duration::from_secs(10)).unwrap();
                if fds[0].received_read() {
                    if let Ok(message) = rx.recv() {
                        match *message {
                            Message::Easy(mut easy, mut exchange) => {
                                let headers_ = SendMutRef(&mut exchange.response_headers);
                                let mut first = true;
                                easy.header_function(move |input| {
                                    parse_header(input, &mut first, unsafe { headers_.deref() })
                                })
                                .unwrap();

                                let body_ = SendMutRef(&mut exchange.body);
                                easy.write_function(move |input| {
                                    let body = unsafe { body_.deref() };
                                    body.extend_from_slice(input);
                                    Ok(input.len())
                                })
                                .unwrap();

                                let mut handle = multi.add(easy).unwrap();
                                handle
                                    .set_token(&*exchange as *const Exchange as usize)
                                    .unwrap();
                                exchange.handle = Some(handle);
                                mem::forget(exchange);
                            }
                            Message::Quit => {
                                quit = true;
                            }
                        }
                    }
                }

                let running_handles = multi.perform().unwrap();
                multi.messages(|message| {
                    if let Some(result) = message.result() {
                        let exchange = unsafe {
                            Box::from_raw(message.token().unwrap() as *mut i32 as *mut Exchange)
                        };

                        let mut easy = multi.remove(exchange.handle.unwrap()).unwrap();

                        if let Err(err) = result {
                            (exchange.callback)(Err(Error(err)));
                        } else {
                            let status_code = easy.response_code().unwrap();
                            let response = Response {
                                body: exchange.body,
                                headers: exchange.response_headers,
                                status_code,
                            };
                            (exchange.callback)(Ok(response));
                        }
                    }
                });
                if running_handles == 0 && quit {
                    break;
                }
            }
            multi.close().unwrap();
        });
        AsyncSession { sender: tx }
    }

    pub fn request(&self, method: &str, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, method, url)
    }

    fn send(&self, easy: Easy, exchange: Exchange) {
        self.sender.send(Message::Easy(easy, Box::new(exchange))).unwrap();
    }
}

impl<'a> RequestBuilder<'a> {
    fn new(session: &'a AsyncSession, method: &str, url: &str) -> RequestBuilder<'a> {
        let mut easy = Easy::new();
        easy.url(url).unwrap();
        easy.custom_request(method).unwrap();

        RequestBuilder { session, easy }
    }

    pub fn send<T>(self, callback: T)
    where
        T: Fn(Result<Response, Error>) + Send + 'static,
    {
        self.session.send(
            self.easy,
            Exchange {
                handle: None,
                callback: Box::new(callback),
                body: Vec::new(),
                response_headers: HashMap::new(),
            },
        );
    }
}

impl Drop for AsyncSession {
    fn drop(&mut self) {
        self.sender.send(Message::Quit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::thread;

    #[test]
    fn happy_path() {
        let session = AsyncSession::new();
        let (tx, rx) = channel();

        let tx_ = tx.clone();
        session
            .request("GET", "https://www.httpbin.org/get")
            .send(move |res| {
                let _res = res.unwrap();
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
            .request("DELETE", "https://www.httpbin.org/delete")
            .send(move |res| {
                let _res = res.unwrap();
                tx_.send(()).unwrap();
            });

        let tx_ = tx.clone();
        session.request("GET", "moz://a").send(move |res| {
            assert!(res.is_err());
            tx_.send(()).unwrap();
        });

        thread::sleep(Duration::from_millis(100));
        drop(session);

        for _i in 0..4 {
            rx.recv().unwrap();
        }
    }
}
