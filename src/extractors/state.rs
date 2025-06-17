use std::{
    convert::Infallible,
    ops::{Deref, DerefMut},
};

use http::request::Parts;

use crate::{handler::FromRequestParts, types::AppState};

pub trait FromRef<T> {
    fn from_ref(state: &T) -> Self;
}

impl<T> FromRef<T> for T
where
    T: Clone,
{
    fn from_ref(input: &T) -> Self {
        input.clone()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct State<S>(pub S);

impl<OuterState, InnerState> FromRequestParts<OuterState> for State<InnerState>
where
    InnerState: FromRef<OuterState>,
    OuterState: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &OuterState,
    ) -> Result<Self, Self::Rejection> {
        let inner_state = InnerState::from_ref(state);
        Ok(Self(inner_state))
    }
}

impl Default for State<()> {
    fn default() -> Self {
        State(())
    }
}

impl<S> Deref for State<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> DerefMut for State<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<S: AppState> From<S> for State<S> {
    fn from(state: S) -> Self {
        State(state)
    }
}
