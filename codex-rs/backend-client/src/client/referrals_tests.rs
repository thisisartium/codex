use super::*;
use crate::types::ReferralGrantAction;
use crate::types::ReferralInvite;
use pretty_assertions::assert_eq;
use reqwest::Method;
use reqwest::StatusCode;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn referral_requests_use_expected_paths_queries_and_body() {
    let chatgpt_client = test_client("https://chatgpt.com/backend-api", PathStyle::ChatGptApi);

    let (eligibility_url, eligibility_request) = chatgpt_client
        .persistent_referral_invite_eligibility_request()
        .unwrap();
    let eligibility_request = eligibility_request.build().unwrap();
    assert_eq!(
        eligibility_url,
        "https://chatgpt.com/backend-api/referrals/invite/eligibility"
    );
    assert_eq!(eligibility_request.method(), Method::GET);
    assert_eq!(
        eligibility_request
            .url()
            .query_pairs()
            .into_owned()
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::from([
            (
                "referral_key".to_string(),
                "codex_referral_persistent_invite".to_string(),
            ),
            ("requested_referrals".to_string(), "1".to_string()),
            (
                "supports_rewardless_invites".to_string(),
                "false".to_string(),
            ),
        ])
    );

    let (rules_url, rules_request) =
        chatgpt_client.persistent_referral_invite_eligibility_rules_request();
    let rules_request = rules_request.build().unwrap();
    assert_eq!(
        rules_url,
        "https://chatgpt.com/backend-api/wham/referrals/eligibility_rules"
    );
    assert_eq!(rules_request.method(), Method::GET);
    assert_eq!(
        rules_request
            .url()
            .query_pairs()
            .into_owned()
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::from([(
            "referral_key".to_string(),
            "codex_referral_persistent_invite".to_string(),
        )])
    );

    let (invite_url, invite_request) =
        chatgpt_client.create_persistent_referral_invite_request("friend@example.com");
    let invite_request = invite_request.build().unwrap();
    assert_eq!(
        invite_url,
        "https://chatgpt.com/backend-api/wham/referrals/invite"
    );
    assert_eq!(invite_request.method(), Method::POST);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            invite_request.body().unwrap().as_bytes().unwrap()
        )
        .unwrap(),
        json!({
            "referral_key": "codex_referral_persistent_invite",
            "emails": ["friend@example.com"]
        })
    );

    let codex_client = test_client("https://example.test", PathStyle::CodexApi);
    assert_eq!(
        codex_client
            .persistent_referral_invite_eligibility_rules_request()
            .0,
        "https://example.test/api/codex/referrals/eligibility_rules"
    );
    assert_eq!(
        codex_client
            .create_persistent_referral_invite_request("friend@example.com")
            .0,
        "https://example.test/api/codex/referrals/invite"
    );
}

#[test]
fn eligibility_is_unsupported_for_codex_api_base_urls() {
    let client = test_client("https://example.test", PathStyle::CodexApi);
    let Err(error) = client.persistent_referral_invite_eligibility_request() else {
        panic!("Codex API eligibility request unexpectedly succeeded");
    };
    assert_eq!(
        error.to_string(),
        "persistent referral invite eligibility requires a ChatGPT backend-api base URL"
    );
}

#[test]
fn referral_responses_decode_expected_fields() {
    let eligibility: PersistentReferralInviteEligibility = serde_json::from_value(json!({
        "should_show": true,
        "ineligible_reason": null,
        "ineligible_reason_code": null,
        "remaining_referrals": 3,
        "grant_action": "rate_limit_reset_credit",
        "grant_amount": 1,
        "has_rewards": true,
        "title": "Limited time offer",
        "description": "Invite a friend and earn a reset.",
        "modal_copy": { "send_button_label": "Send" }
    }))
    .unwrap();
    assert_eq!(
        eligibility,
        PersistentReferralInviteEligibility {
            should_show: true,
            ineligible_reason: None,
            ineligible_reason_code: None,
            remaining_referrals: Some(3),
            grant_action: ReferralGrantAction::RateLimitResetCredit,
            grant_amount: 1,
            has_rewards: true,
            title: Some("Limited time offer".to_string()),
            description: Some("Invite a friend and earn a reset.".to_string()),
        }
    );

    let rules: ReferralEligibilityRules = serde_json::from_value(json!({
        "rules": ["Your friend must be new to Codex."],
        "time_frame_rules": [{
            "invites_sent": 2,
            "invites_total": 5,
            "time_frame": "month",
            "type": "user"
        }],
        "requires_explicit_confirmation": true
    }))
    .unwrap();
    assert_eq!(
        rules,
        ReferralEligibilityRules {
            rules: vec!["Your friend must be new to Codex.".to_string()],
            requires_explicit_confirmation: true,
        }
    );

    let client = test_client("https://chatgpt.com/backend-api", PathStyle::ChatGptApi);
    let invite = client
        .decode_referral_invite_response(
            "https://chatgpt.com/backend-api/wham/referrals/invite",
            "application/json",
            &json!({
                "referral_id": "ref-123",
                "email": "friend@example.com",
                "invites": [{
                    "referral_id": "ref-123",
                    "email": "friend@example.com",
                    "invite_url": "https://chatgpt.com/referrals/ref-123"
                }]
            })
            .to_string(),
        )
        .unwrap();
    assert_eq!(
        invite,
        ReferralInviteResponse {
            referral_id: "ref-123".to_string(),
            email: "friend@example.com".to_string(),
            invites: vec![ReferralInvite {
                referral_id: "ref-123".to_string(),
                email: "friend@example.com".to_string(),
                invite_url: "https://chatgpt.com/referrals/ref-123".to_string(),
            }],
            has_rewards: None,
        }
    );

    let rewardless: ReferralInviteResponse = serde_json::from_value(json!({
        "referral_id": "ref-456",
        "email": "friend@example.com",
        "invites": [{
            "referral_id": "ref-456",
            "email": "friend@example.com",
            "invite_url": "https://chatgpt.com/referrals/ref-456"
        }],
        "has_rewards": false
    }))
    .unwrap();
    assert_eq!(rewardless.has_rewards, Some(false));
}

#[test]
fn referral_invite_decode_error_omits_response_body() {
    let client = test_client("https://chatgpt.com/backend-api", PathStyle::ChatGptApi);
    let error = client
        .decode_referral_invite_response(
            "https://chatgpt.com/backend-api/wham/referrals/invite",
            "application/json",
            r#"{"email":"friend@example.com","invites":not-json}"#,
        )
        .unwrap_err();
    let rendered = error.to_string();

    assert!(!rendered.contains("friend@example.com"));
    assert!(rendered.contains("response body omitted"));
}

#[test]
fn referral_invite_http_error_omits_response_body() {
    let error = redact_referral_response_error(RequestError::UnexpectedStatus {
        method: "POST".to_string(),
        url: "https://chatgpt.com/backend-api/wham/referrals/invite".to_string(),
        status: StatusCode::BAD_REQUEST,
        content_type: "application/json".to_string(),
        body: r#"{"failed_emails":["friend@example.com"]}"#.to_string(),
    });
    let rendered = error.to_string();

    assert_eq!(error.status(), Some(StatusCode::BAD_REQUEST));
    assert!(!rendered.contains("friend@example.com"));
    assert!(rendered.contains("body=omitted"));
}

fn test_client(base_url: &str, path_style: PathStyle) -> Client {
    Client {
        base_url: base_url.to_string(),
        http: reqwest::Client::new(),
        auth_provider: codex_model_provider::unauthenticated_auth_provider(),
        user_agent: None,
        chatgpt_account_id: None,
        chatgpt_account_is_fedramp: false,
        path_style,
    }
}
