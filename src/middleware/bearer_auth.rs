use http::{StatusCode, header};
use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc};

use crate::{
    middleware::Next,
    responder::Responder,
    types::{Request, Response},
};

/// Configuration for Bearer Authentication middleware.
///
/// This struct allows you to configure static tokens or a custom verification function
/// to authenticate incoming requests using the Bearer token scheme.
pub struct Config<C, F>
where
    F: Fn(&str) -> Option<C> + Send + Sync + 'static,
    C: Send + Sync + 'static,
{
    /// Optional set of static tokens for authentication.
    tokens: Option<HashSet<String>>,
    /// Optional custom verification function for dynamic token validation.
    verify: Option<F>,
    /// Phantom data to associate the generic type `C` without storing it.
    _phantom: std::marker::PhantomData<C>,
}

/// Implementation of the `Config` struct, providing methods to configure
/// static tokens, custom verification functions, or a combination of both.
impl<C, F> Config<C, F>
where
    F: Fn(&str) -> Option<C> + Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
{
    /// Creates a configuration with a single static token.
    ///
    /// # Arguments
    /// * `token` - A token string to be used for authentication.
    pub fn static_token(token: impl Into<String>) -> Self {
        Self {
            tokens: Some([token.into()].into()),
            verify: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a configuration with multiple static tokens.
    ///
    /// # Arguments
    /// * `tokens` - An iterator of token strings to be used for authentication.
    pub fn static_tokens<I>(tokens: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            tokens: Some(tokens.into_iter().map(Into::into).collect()),
            verify: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a configuration with a custom verification function.
    ///
    /// # Arguments
    /// * `f` - A function that takes a token string and returns an optional value of type `C`.
    pub fn with_verify(f: F) -> Self {
        Self {
            tokens: None,
            verify: Some(f),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a configuration with both static tokens and a custom verification function.
    ///
    /// # Arguments
    /// * `tokens` - An iterator of token strings to be used for authentication.
    /// * `f` - A function that takes a token string and returns an optional value of type `C`.
    pub fn static_tokens_with_verify<I>(tokens: I, f: F) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            tokens: Some(tokens.into_iter().map(Into::into).collect()),
            verify: Some(f),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Converts the configuration into a middleware function.
    ///
    /// The middleware checks the `Authorization` header for a Bearer token and validates it
    /// against the static tokens or the custom verification function. If the token is valid,
    /// the request is passed to the next middleware; otherwise, a 401 Unauthorized response is returned.
    pub fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let tokens = self.tokens.map(Arc::new);
        let verify = self.verify.map(Arc::new);

        move |mut req: Request, next: Next| {
            let tokens = tokens.clone();
            let verify = verify.clone();

            Box::pin(async move {
                // Extract the Bearer token from the `Authorization` header.
                let tok = req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.strip_prefix("Bearer "))
                    .map(str::trim);

                // Match the extracted token and validate it.
                match tok {
                    None => {}
                    Some(t) => {
                        // Check if the token exists in the static token set.
                        if let Some(set) = &tokens {
                            if set.contains(t) {
                                return next.run(req).await.into_response();
                            }
                        }
                        // If a custom verification function is provided, use it to validate the token.
                        if let Some(v) = verify.as_ref() {
                            if let Some(claims) = v(t) {
                                req.extensions_mut().insert(claims);
                                return next.run(req).await.into_response();
                            }
                        }
                    }
                }

                // Return a 401 Unauthorized response if the token is invalid or missing.
                (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, "Bearer")],
                )
                    .into_response()
            })
        }
    }
}
