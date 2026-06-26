use codex_backend_client::Client as BackendClient;
pub use codex_backend_client::PersistentReferralInviteEligibility;
pub use codex_backend_client::ReferralEligibilityRules;
pub use codex_backend_client::ReferralGrantAction;
pub use codex_backend_client::ReferralInvite;
pub use codex_backend_client::ReferralInviteResponse;
use codex_backend_client::RequestError as BackendRequestError;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::RwLock;

const REFERRAL_GATED_STATUS_CODE: u16 = 403;
const REFERRAL_UNAUTHORIZED_STATUS_CODE: u16 = 401;

/// A currently available referral offer and the account for which it was loaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistentReferralInviteOffer {
    pub expected_identity: ReferralIdentity,
    pub eligibility: PersistentReferralInviteEligibility,
    pub rules: ReferralEligibilityRules,
}

/// The ChatGPT user and workspace that own a referral offer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralIdentity {
    chatgpt_user_id: String,
    account_id: String,
}

impl ReferralIdentity {
    pub fn new(chatgpt_user_id: String, account_id: String) -> Self {
        Self {
            chatgpt_user_id,
            account_id,
        }
    }
}

/// Auth state pinned to the current client session.
///
/// The pinned referral identity advances only when the owning client updates the shared auth
/// state and calls [`Self::sync_identity`]. This keeps referral requests on the same identity as
/// the embedded app-server session.
pub struct ReferralSession {
    auth_manager: Arc<AuthManager>,
    chatgpt_base_url: String,
    pinned_identity: RwLock<Option<ReferralIdentity>>,
}

impl ReferralSession {
    pub fn from_auth_manager(auth_manager: Arc<AuthManager>, chatgpt_base_url: String) -> Self {
        let pinned_identity = auth_manager
            .auth_cached()
            .and_then(|auth| Self::identity_from_auth(&auth).ok());
        Self {
            auth_manager,
            chatgpt_base_url,
            pinned_identity: RwLock::new(pinned_identity),
        }
    }

    #[cfg(test)]
    async fn reload(&self) {
        self.auth_manager.reload().await;
        self.sync_identity().await;
    }

    pub async fn sync_identity(&self) {
        let identity = self
            .auth_manager
            .auth_cached()
            .and_then(|auth| Self::identity_from_auth(&auth).ok());
        *self.pinned_identity.write().await = identity;
    }

    /// Loads the rewarded persistent-invite offer for the pinned ChatGPT session.
    ///
    /// The backend gate is treated as ordinary unavailability so callers can omit the referral UI.
    pub async fn load_persistent_referral_invite_offer(
        &self,
    ) -> Result<Option<PersistentReferralInviteOffer>, ReferralError> {
        let (eligibility, identity) = match self
            .request_with_auth_recovery(/*expected_identity*/ None, |client| async move {
                client.get_persistent_referral_invite_eligibility().await
            })
            .await
        {
            Ok(result) => result,
            Err(ReferralRequestFailure::Backend(error)) if is_expected_gated_response(&error) => {
                return Ok(None);
            }
            Err(ReferralRequestFailure::Backend(error)) => {
                return Err(ReferralError::EligibilityRequestFailed {
                    status_code: error.status().map(|status| status.as_u16()),
                });
            }
            Err(ReferralRequestFailure::ClientBuild) => {
                return Err(ReferralError::EligibilityRequestFailed { status_code: None });
            }
            Err(ReferralRequestFailure::Session(error)) => return Err(error),
        };
        if !eligibility.should_show {
            return Ok(None);
        }

        let (rules, _) = match self
            .request_with_auth_recovery(Some(&identity), |client| async move {
                client
                    .get_persistent_referral_invite_eligibility_rules()
                    .await
            })
            .await
        {
            Ok(result) => result,
            Err(ReferralRequestFailure::Backend(error)) if is_expected_gated_response(&error) => {
                return Ok(None);
            }
            Err(ReferralRequestFailure::Backend(error)) => {
                return Err(ReferralError::RulesRequestFailed {
                    status_code: error.status().map(|status| status.as_u16()),
                });
            }
            Err(ReferralRequestFailure::ClientBuild) => {
                return Err(ReferralError::RulesRequestFailed { status_code: None });
            }
            Err(ReferralRequestFailure::Session(error)) => return Err(error),
        };

        Ok(Some(PersistentReferralInviteOffer {
            expected_identity: identity,
            eligibility,
            rules,
        }))
    }

    /// Sends one invite after confirming that the pinned identity still owns the displayed offer.
    pub async fn send_persistent_referral_invite(
        &self,
        expected_identity: &ReferralIdentity,
        email: &str,
    ) -> Result<ReferralInviteResponse, ReferralError> {
        let email = email.to_string();
        self.request_with_auth_recovery(Some(expected_identity), move |client| {
            let email = email.clone();
            async move { client.create_persistent_referral_invite(&email).await }
        })
        .await
        .map(|(response, _)| response)
        .map_err(|failure| match failure {
            ReferralRequestFailure::Session(error) => error,
            ReferralRequestFailure::ClientBuild => {
                ReferralError::InviteRequestFailed { status_code: None }
            }
            ReferralRequestFailure::Backend(error) => ReferralError::InviteRequestFailed {
                status_code: error.status().map(|status| status.as_u16()),
            },
        })
    }

    async fn load_chatgpt_auth(&self) -> Result<(CodexAuth, ReferralIdentity), ReferralError> {
        let (auth, identity) = Self::load_unpinned_chatgpt_auth(&self.auth_manager).await?;
        if self.pinned_identity.read().await.as_ref() != Some(&identity) {
            return Err(ReferralError::AccountChanged);
        }
        Ok((auth, identity))
    }

    async fn load_unpinned_chatgpt_auth(
        auth_manager: &AuthManager,
    ) -> Result<(CodexAuth, ReferralIdentity), ReferralError> {
        let auth = auth_manager
            .auth()
            .await
            .ok_or(ReferralError::ChatGptAuthenticationRequired)?;
        if !auth.is_chatgpt_auth() {
            return Err(ReferralError::ChatGptAuthenticationRequired);
        }
        let identity = Self::identity_from_auth(&auth)?;
        Ok((auth, identity))
    }

    fn identity_from_auth(auth: &CodexAuth) -> Result<ReferralIdentity, ReferralError> {
        if !auth.is_chatgpt_auth() {
            return Err(ReferralError::ChatGptAuthenticationRequired);
        }
        let chatgpt_user_id = auth
            .get_chatgpt_user_id()
            .filter(|user_id| !user_id.is_empty())
            .ok_or(ReferralError::UserIdUnavailable)?;
        let account_id = auth
            .get_account_id()
            .filter(|account_id| !account_id.is_empty())
            .ok_or(ReferralError::AccountIdUnavailable)?;
        Ok(ReferralIdentity::new(chatgpt_user_id, account_id))
    }

    async fn request_with_auth_recovery<T, F, Fut>(
        &self,
        expected_identity: Option<&ReferralIdentity>,
        mut request: F,
    ) -> Result<(T, ReferralIdentity), ReferralRequestFailure>
    where
        F: FnMut(BackendClient) -> Fut,
        Fut: Future<Output = Result<T, BackendRequestError>>,
    {
        let mut recovery = self.auth_manager.unauthorized_recovery();
        loop {
            let (auth, identity) = self
                .load_chatgpt_auth()
                .await
                .map_err(ReferralRequestFailure::Session)?;
            if expected_identity.is_some_and(|expected| expected != &identity) {
                return Err(ReferralRequestFailure::Session(
                    ReferralError::AccountChanged,
                ));
            }
            let client = self
                .referral_client(&auth)
                .map_err(|_| ReferralRequestFailure::ClientBuild)?;
            match request(client).await {
                Ok(response) => return Ok((response, identity)),
                Err(error) if is_unauthorized_response(&error) => {
                    if recovery.has_next() && recovery.next().await.is_ok() {
                        continue;
                    }
                    return Err(ReferralRequestFailure::Session(
                        ReferralError::ChatGptAuthenticationRequired,
                    ));
                }
                Err(error) => return Err(ReferralRequestFailure::Backend(error)),
            }
        }
    }

    fn referral_client(&self, auth: &CodexAuth) -> anyhow::Result<BackendClient> {
        BackendClient::from_auth(self.chatgpt_base_url.clone(), auth)
    }
}

enum ReferralRequestFailure {
    Session(ReferralError),
    ClientBuild,
    Backend(BackendRequestError),
}

/// A sanitized failure from the client-side referral flow.
///
/// The error intentionally excludes backend response bodies because they can contain email
/// addresses or other details that should not be surfaced through logs or UI error strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferralError {
    ChatGptAuthenticationRequired,
    UserIdUnavailable,
    AccountIdUnavailable,
    AccountChanged,
    UnsupportedClient,
    RequestTimedOut,
    EligibilityRequestFailed { status_code: Option<u16> },
    RulesRequestFailed { status_code: Option<u16> },
    InviteRequestFailed { status_code: Option<u16> },
}

impl fmt::Display for ReferralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ChatGptAuthenticationRequired => {
                "ChatGPT authentication is required for referral invites"
            }
            Self::UserIdUnavailable => "the active ChatGPT user could not be determined",
            Self::AccountIdUnavailable => "the active ChatGPT account could not be determined",
            Self::AccountChanged => {
                "the active ChatGPT account changed; review the referral offer again"
            }
            Self::UnsupportedClient => "referral invites are unavailable for this client session",
            Self::RequestTimedOut => "the referral request timed out",
            Self::EligibilityRequestFailed { .. } => {
                "referral invite eligibility could not be checked"
            }
            Self::RulesRequestFailed { .. } => "referral invite terms could not be loaded",
            Self::InviteRequestFailed { .. } => "the referral invite could not be sent",
        };
        f.write_str(message)
    }
}

impl std::error::Error for ReferralError {}

fn is_expected_gated_response(error: &BackendRequestError) -> bool {
    error
        .status()
        .is_some_and(|status| status.as_u16() == REFERRAL_GATED_STATUS_CODE)
}

fn is_unauthorized_response(error: &BackendRequestError) -> bool {
    error
        .status()
        .is_some_and(|status| status.as_u16() == REFERRAL_UNAUTHORIZED_STATUS_CODE)
}

#[cfg(test)]
#[path = "referrals_tests.rs"]
mod tests;
