use anyhow::Result;
use cookie::{Cookie, CookieJar as RawJar};
use http::{HeaderMap, header::COOKIE};

use crate::{extractors::FromRequest, types::Request};

pub struct CookieJar(RawJar);

impl CookieJar {
    pub fn new() -> Self {
        Self(RawJar::new())
    }

    pub fn from_headers(headers: &HeaderMap) -> Self {
        let mut jar = RawJar::new();

        if let Some(val) = headers.get(COOKIE).and_then(|v| v.to_str().ok()) {
            for s in val.split(';') {
                if let Ok(c) = Cookie::parse(s.trim()) {
                    jar.add_original(c.into_owned());
                }
            }
        }

        Self(jar)
    }

    pub fn add(&mut self, cookie: Cookie<'static>) {
        self.0.add(cookie);
    }

    pub fn remove(&mut self, name: &str) {
        self.0.remove(Cookie::from(name.to_owned()));
    }

    pub fn get(&self, name: &str) -> Option<&Cookie<'_>> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Cookie<'static>> {
        self.0.iter()
    }
}

impl<'a> FromRequest<'a> for CookieJar {
    fn from_request(req: &'a Request) -> Result<Self> {
        Ok(CookieJar::from_headers(req.headers()))
    }
}
