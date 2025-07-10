use crate::{
    body::TakoBody,
    middleware::{IntoMiddleware, Next},
    responder::Responder,
    types::{Request, Response},
};
use base64::Engine;
use bytes::Bytes;
use http::{HeaderValue, StatusCode, header};
use http_body_util::Full;
use std::{collections::HashMap, marker::PhantomData, pin::Pin, sync::Arc};

/// Configuration for Basic Authentication middleware.
///
/// This struct allows you to configure static user credentials or a custom verification function
/// to authenticate incoming requests using the Basic authentication scheme.
pub struct BasicAuth<U, F>
where
    F: Fn(&str, &str) -> Option<U> + Send + Sync + 'static,
    U: Send + Sync + 'static,
{
    /// Optional map of static user credentials (username-password pairs).
    users: Option<Arc<HashMap<String, String>>>,
    /// Optional custom verification function for dynamic user validation.
    verify: Option<Arc<F>>,
    /// The authentication realm to be included in the `WWW-Authenticate` header.
    realm: &'static str,
    /// Phantom data to associate the generic type `U` without storing it.
    _phantom: PhantomData<U>,
}

impl<U, F> BasicAuth<U, F>
where
    F: Fn(&str, &str) -> Option<U> + Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
{
    /// Creates a configuration with a single static user credential.
    ///
    /// # Arguments
    /// * `user` - The username for authentication.
    /// * `pass` - The password for authentication.
    pub fn single(user: impl Into<String>, pass: impl Into<String>) -> Self {
        Self::multiple(std::iter::once((user, pass)))
    }

    /// Creates a configuration with multiple static user credentials.
    ///
    /// # Arguments
    /// * `pairs` - An iterator of username-password pairs for authentication.
    pub fn multiple<I, T, P>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (T, P)>,
        T: Into<String>,
        P: Into<String>,
    {
        Self {
            users: Some(Arc::new(
                pairs
                    .into_iter()
                    .map(|(u, p)| (u.into(), p.into()))
                    .collect(),
            )),
            verify: None,
            realm: "Restricted",
            _phantom: PhantomData,
        }
    }

    /// Creates a configuration with a custom verification function.
    ///
    /// # Arguments
    /// * `cb` - A function that takes a username and password and returns an optional value of type `U`.
    pub fn with_verify(cb: F) -> Self {
        Self {
            users: None,
            verify: Some(Arc::new(cb)),
            realm: "Restricted",
            _phantom: PhantomData,
        }
    }

    /// Creates a configuration with both static user credentials and a custom verification function.
    ///
    /// # Arguments
    /// * `pairs` - An iterator of username-password pairs for authentication.
    /// * `cb` - A function that takes a username and password and returns an optional value of type `U`.
    pub fn users_with_verify<I, S>(pairs: I, cb: F) -> Self
    where
        I: IntoIterator<Item = (S, S)>,
        S: Into<String>,
    {
        Self {
            users: Some(Arc::new(
                pairs
                    .into_iter()
                    .map(|(u, p)| (u.into(), p.into()))
                    .collect(),
            )),
            verify: Some(Arc::new(cb)),
            realm: "Restricted",
            _phantom: PhantomData,
        }
    }

    /// Sets the authentication realm for the `WWW-Authenticate` header.
    ///
    /// # Arguments
    /// * `r` - The realm string to be used.
    pub fn realm(mut self, r: &'static str) -> Self {
        self.realm = r;
        self
    }
}

impl<U, F> IntoMiddleware for BasicAuth<U, F>
where
    F: Fn(&str, &str) -> Option<U> + Clone + Send + Sync + 'static,
    U: Clone + Send + Sync + 'static,
{
    /// Converts the configuration into a middleware function.
    ///
    /// The middleware checks the `Authorization` header for Basic credentials and validates them
    /// against the static user credentials or the custom verification function. If the credentials
    /// are valid, the request is passed to the next middleware; otherwise, a 401 Unauthorized response is returned.
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let users = self.users;
        let verify = self.verify;
        let realm = self.realm;

        move |mut req: Request, next: Next| {
            let users = users.clone();
            let verify = verify.clone();

            Box::pin(async move {
                // Extract the Basic credentials from the `Authorization` header.
                let creds = req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.strip_prefix("Basic "))
                    .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
                    .and_then(|raw| String::from_utf8(raw).ok())
                    .and_then(|s| s.split_once(':').map(|(u, p)| (u.to_owned(), p.to_owned())));

                match creds {
                    Some((u, p)) => {
                        // Check if the credentials match the static user credentials.
                        if users
                            .as_ref()
                            .and_then(|map| map.get(&u))
                            .map(|pw| pw == &p)
                            .unwrap_or(false)
                        {
                            return next.run(req).await.into_response();
                        }

                        // If a custom verification function is provided, use it to validate the credentials.
                        if let Some(cb) = &verify {
                            if let Some(obj) = cb(&u, &p) {
                                req.extensions_mut().insert(obj);
                                return next.run(req).await.into_response();
                            }
                        }
                    }
                    None => {
                        return hyper::Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .header(
                                header::WWW_AUTHENTICATE,
                                HeaderValue::from_str(&format!("Basic realm=\"{realm}\"")).unwrap(),
                            )
                            .body(TakoBody::new(Full::from(Bytes::from(
                                "Missing credentials",
                            ))))
                            .unwrap()
                            .into_response();
                    }
                }

                // Return a 401 Unauthorized response if the credentials are invalid or missing.
                let mut res = Response::new(TakoBody::empty());
                *res.status_mut() = StatusCode::UNAUTHORIZED;
                res.headers_mut().append(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_str(&format!("Basic realm=\"{realm}\"")).unwrap(),
                );
                res
            })
        }
    }
}
