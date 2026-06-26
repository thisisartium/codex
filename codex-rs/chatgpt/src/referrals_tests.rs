use super::*;
use base64::Engine;
use codex_login::AuthCredentialsStoreMode;
use codex_login::AuthKeyringBackendKind;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn external_auth_changes_require_explicit_reload() {
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth(home.path(), "user-a", "account-a");
    let session = test_session(&home, "http://unused.invalid/backend-api").await;
    let identity_a = session.load_chatgpt_auth().await.unwrap().1;

    write_chatgpt_auth(home.path(), "user-b", "account-b");
    assert_eq!(session.load_chatgpt_auth().await.unwrap().1, identity_a);

    session.reload().await;
    assert_eq!(
        session.load_chatgpt_auth().await.unwrap().1,
        ReferralIdentity::new("user-b".to_string(), "account-b".to_string())
    );
}

#[tokio::test]
async fn user_identity_mismatch_blocks_invite_before_network() {
    let server = MockServer::start().await;
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth(home.path(), "user-b", "shared-account");
    let session = test_session(&home, &format!("{}/backend-api", server.uri())).await;

    let result = session
        .send_persistent_referral_invite(
            &ReferralIdentity::new("user-a".to_string(), "shared-account".to_string()),
            "friend@example.com",
        )
        .await;

    assert_eq!(result, Err(ReferralError::AccountChanged));
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .is_empty()
    );
}

#[tokio::test]
async fn gated_eligibility_is_hidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/referrals/invite/eligibility"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth(home.path(), "user-a", "account-a");
    let session = test_session(&home, &format!("{}/backend-api", server.uri())).await;

    assert_eq!(
        session.load_persistent_referral_invite_offer().await,
        Ok(None)
    );
}

#[tokio::test]
async fn gated_rules_are_hidden() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/referrals/invite/eligibility"))
        .respond_with(ResponseTemplate::new(200).set_body_json(eligibility_response()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/backend-api/wham/referrals/eligibility_rules"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth(home.path(), "user-a", "account-a");
    let session = test_session(&home, &format!("{}/backend-api", server.uri())).await;

    assert_eq!(
        session.load_persistent_referral_invite_offer().await,
        Ok(None)
    );
}

#[tokio::test]
async fn invite_http_error_exposes_only_the_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/backend-api/wham/referrals/invite"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"failed_emails":["friend@example.com"]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth(home.path(), "user-a", "account-a");
    let session = test_session(&home, &format!("{}/backend-api", server.uri())).await;
    let identity = ReferralIdentity::new("user-a".to_string(), "account-a".to_string());

    let error = session
        .send_persistent_referral_invite(&identity, "friend@example.com")
        .await
        .unwrap_err();

    assert_eq!(
        error,
        ReferralError::InviteRequestFailed {
            status_code: Some(400)
        }
    );
    assert!(!error.to_string().contains("friend@example.com"));
    assert!(!format!("{error:?}").contains("friend@example.com"));
}

#[tokio::test]
async fn unauthorized_response_reloads_same_identity_and_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/backend-api/wham/referrals/invite"))
        .and(header("authorization", "Bearer old-access-token"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/backend-api/wham/referrals/invite"))
        .and(header("authorization", "Bearer new-access-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(invite_response()))
        .expect(1)
        .mount(&server)
        .await;
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth_with_access_token(home.path(), "user-a", "account-a", "old-access-token");
    let session = test_session(&home, &format!("{}/backend-api", server.uri())).await;
    write_chatgpt_auth_with_access_token(home.path(), "user-a", "account-a", "new-access-token");

    let response = session
        .send_persistent_referral_invite(
            &ReferralIdentity::new("user-a".to_string(), "account-a".to_string()),
            "friend@example.com",
        )
        .await
        .expect("same-identity token reload should retry");

    assert_eq!(
        response,
        ReferralInviteResponse {
            referral_id: "referral-1".to_string(),
            email: "friend@example.com".to_string(),
            invites: vec![ReferralInvite {
                referral_id: "referral-1".to_string(),
                email: "friend@example.com".to_string(),
                invite_url: "https://chatgpt.com/invite/referral-1".to_string(),
            }],
            has_rewards: Some(true),
        }
    );
    server.verify().await;
}

#[tokio::test]
async fn unauthorized_response_does_not_install_or_retry_after_user_change() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/backend-api/wham/referrals/invite"))
        .and(header("authorization", "Bearer user-a-access-token"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/backend-api/wham/referrals/invite"))
        .and(header("authorization", "Bearer user-b-access-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(invite_response()))
        .expect(0)
        .mount(&server)
        .await;
    let home = TempDir::new().expect("temp auth home");
    write_chatgpt_auth_with_access_token(
        home.path(),
        "user-a",
        "shared-account",
        "user-a-access-token",
    );
    let session = test_session(&home, &format!("{}/backend-api", server.uri())).await;
    write_chatgpt_auth_with_access_token(
        home.path(),
        "user-b",
        "shared-account",
        "user-b-access-token",
    );

    let result = session
        .send_persistent_referral_invite(
            &ReferralIdentity::new("user-a".to_string(), "shared-account".to_string()),
            "friend@example.com",
        )
        .await;

    assert_eq!(result, Err(ReferralError::ChatGptAuthenticationRequired));
    let cached_auth = session
        .auth_manager
        .auth_cached()
        .expect("original auth should remain cached");
    assert_eq!(
        ReferralSession::identity_from_auth(&cached_auth),
        Ok(ReferralIdentity::new(
            "user-a".to_string(),
            "shared-account".to_string(),
        ))
    );
    server.verify().await;
}

async fn test_session(home: &TempDir, base_url: &str) -> ReferralSession {
    let auth_manager = AuthManager::shared(
        home.path().to_path_buf(),
        /*enable_codex_api_key_env*/ false,
        AuthCredentialsStoreMode::File,
        /*forced_chatgpt_workspace_id*/ None,
        Some(base_url.to_string()),
        AuthKeyringBackendKind::default(),
        /*auth_route_config*/ None,
    )
    .await;
    ReferralSession::from_auth_manager(auth_manager, base_url.to_string())
}

fn write_chatgpt_auth(home: &std::path::Path, user_id: &str, account_id: &str) {
    write_chatgpt_auth_with_access_token(home, user_id, account_id, &format!("access-{user_id}"));
}

fn write_chatgpt_auth_with_access_token(
    home: &std::path::Path,
    user_id: &str,
    account_id: &str,
    access_token: &str,
) {
    let header = json!({"alg": "none", "typ": "JWT"});
    let payload = json!({
        "email": format!("{user_id}@example.com"),
        "https://api.openai.com/auth": {
            "chatgpt_user_id": user_id,
            "chatgpt_account_id": account_id,
            "chatgpt_plan_type": "pro"
        }
    });
    let encode = |value: &serde_json::Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(value).expect("JWT JSON"))
    };
    let id_token = format!("{}.{}.c2ln", encode(&header), encode(&payload));
    let auth = json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": format!("refresh-{user_id}"),
            "account_id": account_id
        },
        "last_refresh": "2099-01-01T00:00:00Z"
    });
    std::fs::write(
        home.join("auth.json"),
        serde_json::to_vec_pretty(&auth).expect("auth JSON"),
    )
    .expect("write auth file");
}

fn eligibility_response() -> serde_json::Value {
    json!({
        "should_show": true,
        "remaining_referrals": 1,
        "grant_action": "rate_limit_reset_credit",
        "grant_amount": 1,
        "has_rewards": true,
        "title": "Limited time offer",
        "description": "You both receive a reset."
    })
}

fn invite_response() -> serde_json::Value {
    json!({
        "referral_id": "referral-1",
        "email": "friend@example.com",
        "invites": [{
            "referral_id": "referral-1",
            "email": "friend@example.com",
            "invite_url": "https://chatgpt.com/invite/referral-1"
        }],
        "has_rewards": true
    })
}
