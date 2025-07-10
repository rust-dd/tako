use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use http::{StatusCode, header::AUTHORIZATION};
use jwt_simple::prelude::*;
use serde::de::DeserializeOwned;

use crate::{
    middleware::{IntoMiddleware, Next},
    responder::Responder,
    types::{Request, Response},
};

pub enum AnyVerifyKey {
    HS256(Arc<HS256Key>),
    HS384(Arc<HS384Key>),
    HS512(Arc<HS512Key>),
    Blake2b(Arc<Blake2bKey>),

    RS256(Arc<RS256PublicKey>),
    RS384(Arc<RS384PublicKey>),
    RS512(Arc<RS512PublicKey>),

    PS256(Arc<PS256PublicKey>),
    PS384(Arc<PS384PublicKey>),
    PS512(Arc<PS512PublicKey>),

    ES256(Arc<ES256PublicKey>),
    ES256K(Arc<ES256kPublicKey>),
    ES384(Arc<ES384PublicKey>),

    EdDSA(Arc<Ed25519PublicKey>),
}

impl AnyVerifyKey {
    pub fn alg_id(&self) -> &'static str {
        match self {
            Self::HS256(_) => "HS256",
            Self::HS384(_) => "HS384",
            Self::HS512(_) => "HS512",
            Self::Blake2b(_) => "BLAKE2B",

            Self::RS256(_) => "RS256",
            Self::RS384(_) => "RS384",
            Self::RS512(_) => "RS512",

            Self::PS256(_) => "PS256",
            Self::PS384(_) => "PS384",
            Self::PS512(_) => "PS512",

            Self::ES256(_) => "ES256",
            Self::ES256K(_) => "ES256K",
            Self::ES384(_) => "ES384",

            Self::EdDSA(_) => "EdDSA",
        }
    }

    pub fn verify<C>(&self, token: &str) -> Result<JWTClaims<C>, jwt_simple::Error>
    where
        C: Serialize + DeserializeOwned,
    {
        let opts = VerificationOptions::default();
        match self {
            Self::HS256(k) => k.verify_token::<C>(token, Some(opts)),
            Self::HS384(k) => k.verify_token::<C>(token, Some(opts)),
            Self::HS512(k) => k.verify_token::<C>(token, Some(opts)),
            Self::Blake2b(k) => k.verify_token::<C>(token, Some(opts)),

            Self::RS256(k) => k.verify_token::<C>(token, Some(opts)),
            Self::RS384(k) => k.verify_token::<C>(token, Some(opts)),
            Self::RS512(k) => k.verify_token::<C>(token, Some(opts)),

            Self::PS256(k) => k.verify_token::<C>(token, Some(opts)),
            Self::PS384(k) => k.verify_token::<C>(token, Some(opts)),
            Self::PS512(k) => k.verify_token::<C>(token, Some(opts)),

            Self::ES256(k) => k.verify_token::<C>(token, Some(opts)),
            Self::ES256K(k) => k.verify_token::<C>(token, Some(opts)),
            Self::ES384(k) => k.verify_token::<C>(token, Some(opts)),

            Self::EdDSA(k) => k.verify_token::<C>(token, Some(opts)),
        }
    }
}

pub struct JwtAuth<T>
where
    T: DeserializeOwned + Send + Sync + 'static,
{
    keys: Arc<HashMap<&'static str, AnyVerifyKey>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> JwtAuth<T>
where
    T: DeserializeOwned + Send + Sync + 'static,
{
    pub fn new(keys: HashMap<&'static str, AnyVerifyKey>) -> Self {
        Self {
            keys: Arc::new(keys),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> IntoMiddleware for JwtAuth<T>
where
    T: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let keys = self.keys.clone();

        move |mut req: Request, next: Next| {
            let keys = keys.clone();

            Box::pin(async move {
                let token = match req
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .map(str::trim)
                {
                    Some(t) => t,
                    None => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            "Missing or invalid Authorization header",
                        )
                            .into_response();
                    }
                };

                let token_meta = match jwt_simple::token::Token::decode_metadata(token) {
                    Ok(h) => h,
                    Err(_) => {
                        return (StatusCode::UNAUTHORIZED, "Cannot decode JWT header")
                            .into_response();
                    }
                };

                let alg = &token_meta.algorithm();
                let verify_key = match keys.get(alg) {
                    Some(k) => k,
                    None => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            format!("Algorithm {} not allowed", alg),
                        )
                            .into_response();
                    }
                };

                let claims = match verify_key.verify::<T>(token) {
                    Ok(c) => c,
                    Err(e) => {
                        return (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e))
                            .into_response();
                    }
                };

                req.extensions_mut().insert(claims);
                next.run(req).await.into_response()
            })
        }
    }
}
