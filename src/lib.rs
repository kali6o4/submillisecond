use std::{io, mem};

pub use http;
use http::{header, HeaderValue};
use lunatic::net::{TcpListener, TcpStream, ToSocketAddrs};
use lunatic::{Mailbox, Process};
pub use submillisecond_macros::*;

pub use crate::error::*;
pub use crate::guard::*;
pub use crate::handler::*;
pub use crate::request::*;
use crate::response::{IntoResponse, Response};

#[macro_use]
pub(crate) mod macros;

#[cfg(feature = "cookie")]
pub mod cookies;
mod core;
pub mod defaults;
pub mod extract;
#[cfg(feature = "json")]
pub mod json;
pub mod params;
pub mod reader;
pub mod response;
#[cfg(feature = "cookie")]
pub mod session;
#[cfg(feature = "template")]
pub mod template;

mod error;
mod guard;
mod handler;
mod request;

/// Signature of router function generated by the [`router!`] macro.
pub type Router = fn(RequestContext) -> Response;

#[derive(Clone, Copy)]
pub struct Application {
    router: Router,
}

impl Application {
    pub fn new(router: Router) -> Self {
        Application { router }
    }

    pub fn serve<A: ToSocketAddrs>(self, addr: A) -> io::Result<()> {
        let listener = TcpListener::bind(addr)?;

        while let Ok((stream, _)) = listener.accept() {
            Process::spawn_link(
                (stream, self.router as *const () as usize),
                |(stream, handler_raw): (TcpStream, usize), _: Mailbox<()>| {
                    let handler = unsafe {
                        let pointer = handler_raw as *const ();
                        mem::transmute::<*const (), Router>(pointer)
                    };

                    let request = match core::parse_request(stream.clone()) {
                        Ok(request) => request,
                        Err(err) => {
                            if let Err(err) = core::write_response(stream, err.into_response()) {
                                eprintln!("[http reader] Failed to send response {:?}", err);
                            }
                            return;
                        }
                    };
                    let http_version = request.version();

                    let mut response =
                        Handler::handle(&handler, RequestContext::from(request)).into_response();

                    let content_length = response.body().len();
                    *response.version_mut() = http_version;
                    response
                        .headers_mut()
                        .append(header::CONTENT_LENGTH, HeaderValue::from(content_length));

                    if let Err(err) = core::write_response(stream, response) {
                        eprintln!("[http reader] Failed to send response {:?}", err);
                    }
                },
            );
        }

        Ok(())
    }
}
