#![allow(missing_docs)]

use crate::{authorization::AuthorizationState, media::MediaState, route::{RouteKind, RouteState}, SessionPlan};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionTransitionError {
    #[error("authorization is required")]
    AuthorizationRequired,
    #[error("authorization was denied or revoked")]
    AuthorizationDenied,
    #[error("session is already closed")]
    Closed,
    #[error("route migration is not in progress")]
    RouteMigrationNotInProgress,
}

#[derive(Debug, Clone)]
pub struct RemoteSessionAggregate {
    plan: SessionPlan,
    authorization: AuthorizationState,
    route: RouteState,
    media: MediaState,
    granted_scopes: Vec<String>,
    last_failure: Option<String>,
    closed: bool,
}

impl RemoteSessionAggregate {
    pub fn new(plan: SessionPlan) -> Self {
        Self {
            plan,
            authorization: AuthorizationState::Pending,
            route: RouteState::Idle,
            media: MediaState::Idle,
            granted_scopes: Vec::new(),
            last_failure: None,
            closed: false,
        }
    }

    pub fn plan(&self) -> &SessionPlan { &self.plan }
    pub fn authorization_state(&self) -> &AuthorizationState { &self.authorization }
    pub fn route_state(&self) -> &RouteState { &self.route }
    pub fn media_state(&self) -> &MediaState { &self.media }
    pub fn granted_scopes(&self) -> &[String] { &self.granted_scopes }
    pub fn last_failure(&self) -> Option<&str> { self.last_failure.as_deref() }

    pub fn authorize(&mut self, scopes: Vec<String>, policy_revision: u64) -> Result<(), SessionTransitionError> {
        if self.closed { return Err(SessionTransitionError::Closed); }
        self.granted_scopes = scopes;
        self.authorization = AuthorizationState::Granted { policy_revision };
        Ok(())
    }

    pub fn deny_authorization(&mut self, reason: impl Into<String>) {
        self.authorization = AuthorizationState::Denied { reason: reason.into() };
    }

    pub fn begin_route_migration(&mut self, kind: RouteKind) -> Result<(), SessionTransitionError> {
        self.ensure_authorized()?;
        self.route = RouteState::Establishing(kind);
        Ok(())
    }

    pub fn complete_route_migration(&mut self, kind: RouteKind) -> Result<(), SessionTransitionError> {
        if self.route != RouteState::Establishing(kind) {
            return Err(SessionTransitionError::RouteMigrationNotInProgress);
        }
        self.route = RouteState::Active(kind);
        Ok(())
    }

    pub fn start_media(&mut self) -> Result<(), SessionTransitionError> {
        self.ensure_authorized()?;
        self.media = MediaState::Starting;
        Ok(())
    }

    pub fn mark_streaming(&mut self) -> Result<(), SessionTransitionError> {
        self.ensure_authorized()?;
        self.media = MediaState::Streaming;
        Ok(())
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.media = MediaState::Stopped;
    }

    fn ensure_authorized(&self) -> Result<(), SessionTransitionError> {
        if self.closed { return Err(SessionTransitionError::Closed); }
        match self.authorization {
            AuthorizationState::Pending => Err(SessionTransitionError::AuthorizationRequired),
            AuthorizationState::Denied { .. } | AuthorizationState::Revoked { .. } => Err(SessionTransitionError::AuthorizationDenied),
            AuthorizationState::Granted { .. } => Ok(()),
        }
    }
}
