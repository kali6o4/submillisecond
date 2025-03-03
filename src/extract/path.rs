//! Extractor that will get captures from the URL and parse them using
//! [`serde`].

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use http::StatusCode;
use serde::de::DeserializeOwned;

use self::de::PercentDecodedStr;
use crate::extract::rejection::*;
use crate::extract::FromRequest;
use crate::params::Params;
use crate::response::IntoResponse;
use crate::{RequestContext, Response};

#[doc(hidden)]
pub mod de;

/// Extractor that will get captures from the URL and parse them using
/// [`serde`].
///
/// Any percent encoded parameters will be automatically decoded. The decoded
/// parameters must be valid UTF-8, otherwise `Path` will fail and return a `400
/// Bad Request` response.
///
/// # Example
///
/// ```
/// use submillisecond::{router, extract::Path};
/// use uuid::Uuid;
///
/// fn users_teams_show(
///     Path((user_id, team_id)): Path<(Uuid, Uuid)>,
/// ) {
///     // ...
/// }
///
/// router! {
///     GET "/users/:user_id/team/:team_id" => users_teams_show
/// }
/// ```
///
/// If the path contains only one parameter, then you can omit the tuple.
///
/// ```
/// use submillisecond::{router, extract::Path};
/// use uuid::Uuid;
///
/// fn user_info(
///     Path(user_id): Path<Uuid>,
/// ) {
///     // ...
/// }
///
/// router! {
///     GET "/users/:user_id" => user_info
/// }
/// ```
///
/// Path segments also can be deserialized into any type that implements
/// [`serde::Deserialize`]. This includes tuples and structs:
///
/// ```
/// use serde::Deserialize;
/// use submillisecond::{router, extract::Path};
/// use uuid::Uuid;
///
/// // Path segment labels will be matched with struct field names
/// #[derive(Deserialize)]
/// struct Params {
///     user_id: Uuid,
///     team_id: Uuid,
/// }
///
/// fn users_teams_show(
///     Path(Params { user_id, team_id }): Path<Params>,
/// ) {
///     // ...
/// }
///
/// // When using tuples the path segments will be matched by their position in the route
/// fn users_teams_create(
///     Path((user_id, team_id)): Path<(String, String)>,
/// ) {
///     // ...
/// }
///
/// router! {
///     GET "/users/:user_id/team/:team_id" => users_teams_show
///     POST "/users/:user_id/team/:team_id" => users_teams_create
/// }
/// ```
///
/// If you wish to capture all path parameters you can use `HashMap` or `Vec`:
///
/// ```
/// use submillisecond::{router, extract::Path};
/// use std::collections::HashMap;
///
/// fn params_map(
///     Path(params): Path<HashMap<String, String>>,
/// ) {
///     // ...
/// }
///
/// fn params_vec(
///     Path(params): Path<Vec<(String, String)>>,
/// ) {
///     // ...
/// }
///
/// router! {
///     GET "/users/:user_id/team/:team_id" => params_map
///     POST "/users/:user_id/team/:team_id" => params_vec
/// }
/// ```
///
/// # Providing detailed rejection output
///
/// If the URI cannot be deserialized into the target type the request will be
/// rejected and an error response will be returned.
///
/// [`serde`]: https://crates.io/crates/serde
/// [`serde::Deserialize`]: https://docs.rs/serde/1.0.143/serde/trait.Deserialize.html
#[derive(Debug)]
pub struct Path<T>(pub T);

impl<T> Deref for Path<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Path<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> FromRequest for Path<T>
where
    T: DeserializeOwned,
{
    type Rejection = PathRejection;

    fn from_request(req: &mut RequestContext) -> Result<Self, Self::Rejection> {
        let params = req
            .extensions_mut()
            .get::<Params>()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                if let Some(decoded) = PercentDecodedStr::new(v) {
                    Ok((Arc::from(k), decoded))
                } else {
                    Err(PathRejection::FailedToDeserializePathParams(
                        FailedToDeserializePathParams(PathDeserializationError {
                            kind: ErrorKind::InvalidUtf8InPathParam { key: k.to_string() },
                        }),
                    ))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        T::deserialize(de::PathDeserializer::new(&*params))
            .map_err(|err| {
                PathRejection::FailedToDeserializePathParams(FailedToDeserializePathParams(err))
            })
            .map(Path)
    }
}

// this wrapper type is used as the deserializer error to hide the
// `serde::de::Error` impl which would otherwise be public if we used
// `ErrorKind` as the error directly
#[doc(hidden)]
#[derive(Debug)]
pub struct PathDeserializationError {
    pub(super) kind: ErrorKind,
}

impl PathDeserializationError {
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    pub(super) fn wrong_number_of_parameters() -> WrongNumberOfParameters<()> {
        WrongNumberOfParameters { got: () }
    }

    pub(super) fn unsupported_type(name: &'static str) -> Self {
        Self::new(ErrorKind::UnsupportedType { name })
    }
}

pub(super) struct WrongNumberOfParameters<G> {
    got: G,
}

impl<G> WrongNumberOfParameters<G> {
    #[allow(clippy::unused_self)]
    pub(super) fn got<G2>(self, got: G2) -> WrongNumberOfParameters<G2> {
        WrongNumberOfParameters { got }
    }
}

impl WrongNumberOfParameters<usize> {
    pub(super) fn expected(self, expected: usize) -> PathDeserializationError {
        PathDeserializationError::new(ErrorKind::WrongNumberOfParameters {
            got: self.got,
            expected,
        })
    }
}

impl serde::de::Error for PathDeserializationError {
    #[inline]
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self {
            kind: ErrorKind::Message(msg.to_string()),
        }
    }
}

impl fmt::Display for PathDeserializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}

impl std::error::Error for PathDeserializationError {}

/// The kinds of errors that can happen we deserializing into a [`Path`].
///
/// This type is obtained through [`FailedToDeserializePathParams::into_kind`]
/// and is useful for building more precise error messages.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The URI contained the wrong number of parameters.
    WrongNumberOfParameters {
        /// The number of actual parameters in the URI.
        got: usize,
        /// The number of expected parameters.
        expected: usize,
    },

    /// Failed to parse the value at a specific key into the expected type.
    ///
    /// This variant is used when deserializing into types that have named
    /// fields, such as structs.
    ParseErrorAtKey {
        /// The key at which the value was located.
        key: String,
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// Failed to parse the value at a specific index into the expected type.
    ///
    /// This variant is used when deserializing into sequence types, such as
    /// tuples.
    ParseErrorAtIndex {
        /// The index at which the value was located.
        index: usize,
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// Failed to parse a value into the expected type.
    ///
    /// This variant is used when deserializing into a primitive type (such as
    /// `String` and `u32`).
    ParseError {
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// A parameter contained text that, once percent decoded, wasn't valid
    /// UTF-8.
    InvalidUtf8InPathParam {
        /// The key at which the invalid value was located.
        key: String,
    },

    /// Tried to serialize into an unsupported type such as nested maps.
    ///
    /// This error kind is caused by programmer errors and thus gets converted
    /// into a `500 Internal Server Error` response.
    UnsupportedType {
        /// The name of the unsupported type.
        name: &'static str,
    },

    /// Catch-all variant for errors that don't fit any other variant.
    Message(String),
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Message(error) => error.fmt(f),
            ErrorKind::InvalidUtf8InPathParam { key } => write!(f, "Invalid UTF-8 in `{}`", key),
            ErrorKind::WrongNumberOfParameters { got, expected } => {
                write!(
                    f,
                    "Wrong number of path arguments for `Path`. Expected {} but got {}",
                    expected, got
                )?;

                if *expected == 1 {
                    write!(
                        f,
                        ". Note that multiple parameters must be extracted with a tuple `Path<(_, _)>` or a struct `Path<YourParams>`"
                    )?;
                }

                Ok(())
            }
            ErrorKind::UnsupportedType { name } => write!(f, "Unsupported type `{}`", name),
            ErrorKind::ParseErrorAtKey {
                key,
                value,
                expected_type,
            } => write!(
                f,
                "Cannot parse `{}` with value `{:?}` to a `{}`",
                key, value, expected_type
            ),
            ErrorKind::ParseError {
                value,
                expected_type,
            } => write!(f, "Cannot parse `{:?}` to a `{}`", value, expected_type),
            ErrorKind::ParseErrorAtIndex {
                index,
                value,
                expected_type,
            } => write!(
                f,
                "Cannot parse value at index {} with value `{:?}` to a `{}`",
                index, value, expected_type
            ),
        }
    }
}

/// Rejection type for [`Path`](super::Path) if the captured routes params
/// couldn't be deserialized into the expected type.
#[derive(Debug)]
pub struct FailedToDeserializePathParams(pub PathDeserializationError);

impl FailedToDeserializePathParams {
    /// Convert this error into the underlying error kind.
    pub fn into_kind(self) -> ErrorKind {
        self.0.kind
    }
}

impl IntoResponse for FailedToDeserializePathParams {
    fn into_response(self) -> Response {
        let (status, body) = match self.0.kind {
            ErrorKind::Message(_)
            | ErrorKind::InvalidUtf8InPathParam { .. }
            | ErrorKind::ParseError { .. }
            | ErrorKind::ParseErrorAtIndex { .. }
            | ErrorKind::ParseErrorAtKey { .. } => (
                StatusCode::BAD_REQUEST,
                format!("Invalid URL: {}", self.0.kind),
            ),
            ErrorKind::WrongNumberOfParameters { .. } | ErrorKind::UnsupportedType { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.0.kind.to_string())
            }
        };
        (status, body).into_response()
    }
}

impl fmt::Display for FailedToDeserializePathParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for FailedToDeserializePathParams {}
