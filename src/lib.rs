use std::{io, mem};

pub use http;
use http::{header, HeaderValue};
use lunatic::{
    net::{TcpListener, TcpStream, ToSocketAddrs},
    Mailbox, Process,
};
pub use submillisecond_macros::*;

use crate::core::UriReader;
pub use crate::error::{BoxError, Error};
use crate::params::Params;
use crate::response::IntoResponse;
pub use crate::response::Response;

#[macro_use]
pub(crate) mod macros;

pub mod core;
pub mod defaults;
mod error;
pub mod extract;
pub mod guard;
pub mod handler;
pub mod json;
pub mod params;
pub mod request_context;
pub mod response;
pub mod template;

/// Signature of router function generated by the [`router!`] macro.
pub type Router = fn(Request, Params, UriReader) -> Result<Response, RouteError>;

/// Type alias for [`http::Request`] whose body defaults to [`String`].
pub type Request<T = Vec<u8>> = http::Request<T>;

#[derive(Clone, Copy)]
pub struct Application {
    router: Router,
}

impl Application {
    pub fn new(router: Router) -> Self {
        Application { router }
    }

    pub fn merge_extensions(request: &mut Request, params: &mut Params) {
        let extensions = request.extensions_mut();
        match extensions.get_mut::<Params>() {
            Some(ext_params) => {
                ext_params.merge(params.clone());
            }
            None => {
                extensions.insert(params.clone());
            }
        };
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

                    let path = request.uri().path().to_string();
                    let http_version = request.version();

                    let params = Params::new();
                    let reader = UriReader::new(path);
                    let mut response =
                        handler(request, params, reader).unwrap_or_else(|err| err.into_response());

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

pub trait Middleware {
    fn before(&mut self, req: &mut Request);
    fn after(&self, res: &mut Response);
}

#[derive(Debug)]
pub enum RouteError {
    ExtractorError(Response),
    RouteNotMatch(Request),
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            RouteError::ExtractorError(resp) => resp,
            RouteError::RouteNotMatch(_) => defaults::err_404(),
        }
    }
}
