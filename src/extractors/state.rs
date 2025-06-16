use std::ops::{Deref, DerefMut};

use crate::types::AppState;

#[derive(Debug, Clone)]
pub struct State<S>(pub S);

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
