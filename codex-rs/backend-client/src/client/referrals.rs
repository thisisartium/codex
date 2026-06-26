//! Backend client operations for the persistent Codex referral invite.

use super::Client;
use super::PathStyle;
use super::RequestError;
use crate::types::PersistentReferralInviteEligibility;
use crate::types::ReferralEligibilityRules;
use crate::types::ReferralInviteResponse;
use reqwest::RequestBuilder;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::HeaderValue;
use serde::Serialize;

const PERSISTENT_REFERRAL_INVITE_KEY: &str = "codex_referral_persistent_invite";

#[derive(Serialize)]
struct PersistentReferralInviteEligibilityQuery {
    referral_key: &'static str,
    requested_referrals: u8,
    supports_rewardless_invites: bool,
}

#[derive(Serialize)]
struct ReferralEligibilityRulesQuery {
    referral_key: &'static str,
}

#[derive(Serialize)]
struct CreatePersistentReferralInviteRequest<'a> {
    referral_key: &'static str,
    emails: [&'a str; 1],
}

impl Client {
    /// Returns eligibility for one rewarded invite from the persistent Codex referral program.
    ///
    /// Eligibility is only available through a ChatGPT backend-api base URL. Rewardless invites
    /// are deliberately excluded from this client flow.
    pub async fn get_persistent_referral_invite_eligibility(
        &self,
    ) -> std::result::Result<PersistentReferralInviteEligibility, RequestError> {
        let (url, request) = self.persistent_referral_invite_eligibility_request()?;
        let (body, content_type) = self
            .exec_request_detailed(request, "GET", &url)
            .await
            .map_err(redact_referral_response_error)?;
        self.decode_json(&url, &content_type, &body)
            .map_err(RequestError::from)
    }

    /// Returns the display rules and consent requirement for the persistent referral program.
    pub async fn get_persistent_referral_invite_eligibility_rules(
        &self,
    ) -> std::result::Result<ReferralEligibilityRules, RequestError> {
        let (url, request) = self.persistent_referral_invite_eligibility_rules_request();
        let (body, content_type) = self
            .exec_request_detailed(request, "GET", &url)
            .await
            .map_err(redact_referral_response_error)?;
        self.decode_json(&url, &content_type, &body)
            .map_err(RequestError::from)
    }

    /// Sends one email invite through the persistent Codex referral program.
    pub async fn create_persistent_referral_invite(
        &self,
        email: &str,
    ) -> std::result::Result<ReferralInviteResponse, RequestError> {
        let (url, request) = self.create_persistent_referral_invite_request(email);
        let (body, content_type) = self
            .exec_request_detailed(request, "POST", &url)
            .await
            .map_err(redact_referral_response_error)?;
        self.decode_referral_invite_response(&url, &content_type, &body)
    }

    fn persistent_referral_invite_eligibility_request(
        &self,
    ) -> std::result::Result<(String, RequestBuilder), RequestError> {
        let url = match self.path_style {
            PathStyle::ChatGptApi => {
                format!("{}/referrals/invite/eligibility", self.base_url)
            }
            PathStyle::CodexApi => {
                return Err(anyhow::anyhow!(
                    "persistent referral invite eligibility requires a ChatGPT backend-api base URL"
                )
                .into());
            }
        };
        let request = self.http.get(&url).headers(self.headers()).query(
            &PersistentReferralInviteEligibilityQuery {
                referral_key: PERSISTENT_REFERRAL_INVITE_KEY,
                requested_referrals: 1,
                supports_rewardless_invites: false,
            },
        );
        Ok((url, request))
    }

    fn persistent_referral_invite_eligibility_rules_request(&self) -> (String, RequestBuilder) {
        let url = match self.path_style {
            PathStyle::CodexApi => {
                format!("{}/api/codex/referrals/eligibility_rules", self.base_url)
            }
            PathStyle::ChatGptApi => {
                format!("{}/wham/referrals/eligibility_rules", self.base_url)
            }
        };
        let request =
            self.http
                .get(&url)
                .headers(self.headers())
                .query(&ReferralEligibilityRulesQuery {
                    referral_key: PERSISTENT_REFERRAL_INVITE_KEY,
                });
        (url, request)
    }

    fn create_persistent_referral_invite_request(&self, email: &str) -> (String, RequestBuilder) {
        let url = match self.path_style {
            PathStyle::CodexApi => format!("{}/api/codex/referrals/invite", self.base_url),
            PathStyle::ChatGptApi => format!("{}/wham/referrals/invite", self.base_url),
        };
        let request = self
            .http
            .post(&url)
            .headers(self.headers())
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .json(&CreatePersistentReferralInviteRequest {
                referral_key: PERSISTENT_REFERRAL_INVITE_KEY,
                emails: [email],
            });
        (url, request)
    }

    fn decode_referral_invite_response(
        &self,
        url: &str,
        content_type: &str,
        body: &str,
    ) -> std::result::Result<ReferralInviteResponse, RequestError> {
        serde_json::from_str(body).map_err(|error| {
            anyhow::anyhow!(
                "Decode error for {url}: {error}; content-type={content_type}; response body omitted because it can contain referral email addresses"
            )
            .into()
        })
    }
}

fn redact_referral_response_error(error: RequestError) -> RequestError {
    match error {
        RequestError::UnexpectedStatus {
            method,
            url,
            status,
            content_type,
            body: _,
        } => RequestError::UnexpectedStatus {
            method,
            url,
            status,
            content_type,
            body: "omitted because it can contain referral email addresses".to_string(),
        },
        RequestError::Other(error) => RequestError::Other(error),
    }
}

#[cfg(test)]
#[path = "referrals_tests.rs"]
mod tests;
