use std::{convert::Infallible, mem};

use crate::Request;

use super::FromRequest;

impl FromRequest for Vec<u8> {
    type Rejection = Infallible;

    fn from_request(req: &mut Request) -> Result<Self, Self::Rejection> {
        let body = mem::take(req.body_mut());
        Ok(body)
    }
}
