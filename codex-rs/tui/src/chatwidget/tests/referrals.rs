use super::*;
use codex_app_server_protocol::RateLimitResetCreditsSummary;
use codex_chatgpt::referrals::PersistentReferralInviteEligibility;
use codex_chatgpt::referrals::PersistentReferralInviteOffer;
use codex_chatgpt::referrals::ReferralEligibilityRules;
use codex_chatgpt::referrals::ReferralError;
use codex_chatgpt::referrals::ReferralGrantAction;
use codex_chatgpt::referrals::ReferralIdentity;

#[tokio::test]
async fn referral_offer_appears_in_usage_without_moving_selection() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    set_chatgpt_auth(&mut chat);

    chat.dispatch_command(SlashCommand::Usage);
    let request_id = match rx.try_recv() {
        Ok(AppEvent::RefreshReferralInviteOffer { request_id }) => request_id,
        _ => panic!("expected referral offer refresh"),
    };
    assert!(!render_bottom_popup(&chat, /*width*/ 80).contains("Invite someone to Codex"));

    chat.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(chat.finish_referral_invite_offer_refresh(request_id, Ok(Some(referral_offer()))));
    assert_chatwidget_snapshot!(
        "usage_menu_with_referral_offer",
        render_bottom_popup(&chat, /*width*/ 80)
    );

    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_matches!(rx.try_recv(), Ok(AppEvent::OpenRateLimitResetCredits));
}

#[tokio::test]
async fn concurrent_usage_refreshes_preserve_referral_selection() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    set_chatgpt_auth(&mut chat);
    let startup_request_id = chat.start_rate_limit_reset_startup_check();
    assert!(chat.finish_rate_limit_reset_hint_refresh(
        startup_request_id,
        Vec::new(),
        Ok(RateLimitResetCreditsSummary { available_count: 0 }),
    ));

    chat.dispatch_command(SlashCommand::Usage);
    let referral_request_id = match rx.try_recv() {
        Ok(AppEvent::RefreshReferralInviteOffer { request_id }) => request_id,
        _ => panic!("expected referral offer refresh"),
    };
    assert_matches!(
        rx.try_recv(),
        Ok(AppEvent::RefreshRateLimits {
            origin: RateLimitRefreshOrigin::UsageMenu { request_id: 1 }
        })
    );
    assert!(
        chat.finish_referral_invite_offer_refresh(referral_request_id, Ok(Some(referral_offer())),)
    );
    chat.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    chat.finish_usage_menu_rate_limit_refresh(
        /*request_id*/ 1,
        Vec::new(),
        Ok(RateLimitResetCreditsSummary { available_count: 0 }),
    );
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_matches!(
        rx.try_recv(),
        Ok(AppEvent::OpenReferralInviteEmailPrompt {
            initial_email: None
        })
    );
}

#[tokio::test]
async fn referral_email_and_confirmation_require_an_explicit_send_selection() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.referral_invite_offer = Some(referral_offer());

    chat.show_referral_invite_email_prompt(None);
    assert_chatwidget_snapshot!(
        "referral_invite_email_prompt",
        render_bottom_popup(&chat, /*width*/ 80)
    );
    chat.handle_paste("friend@example.com".to_string());
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_matches!(
        rx.try_recv(),
        Ok(AppEvent::OpenReferralInviteConfirmation { email })
            if email == "friend@example.com"
    );

    chat.show_referral_invite_confirmation("friend@example.com".to_string());
    assert_chatwidget_snapshot!(
        "referral_invite_confirmation",
        render_bottom_popup(&chat, /*width*/ 80)
    );
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err(), "Cancel must be selected by default");

    chat.show_referral_invite_confirmation("friend@example.com".to_string());
    chat.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_matches!(
        rx.try_recv(),
        Ok(AppEvent::SendReferralInvite { email }) if email == "friend@example.com"
    );
}

#[tokio::test]
async fn referral_send_is_noncancelable_and_shows_success() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.referral_invite_offer = Some(referral_offer());

    let (request_id, expected_identity) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should start send");
    assert_eq!(expected_identity, referral_identity());
    assert_chatwidget_snapshot!(
        "referral_invite_sending",
        render_bottom_popup(&chat, /*width*/ 80)
    );
    chat.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(chat.bottom_pane.active_view_id(), Some("referral-invite"));

    assert!(chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Ok(ReferralInviteRewardStatus::Included),
    ));
    assert_chatwidget_snapshot!(
        "referral_invite_success",
        render_bottom_popup(&chat, /*width*/ 80)
    );

    chat.referral_invite_offer = Some(referral_offer());
    let (request_id, _) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should start send");
    assert!(chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Ok(ReferralInviteRewardStatus::NotIncluded),
    ));
    assert_chatwidget_snapshot!(
        "referral_invite_rewardless_success",
        render_bottom_popup(&chat, /*width*/ 80)
    );

    chat.referral_invite_offer = Some(referral_offer());
    let (request_id, _) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should start send");
    assert!(chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Ok(ReferralInviteRewardStatus::Unknown),
    ));
    assert_chatwidget_snapshot!(
        "referral_invite_unknown_reward_success",
        render_bottom_popup(&chat, /*width*/ 80)
    );
}

#[tokio::test]
async fn ambiguous_referral_send_failure_does_not_offer_retry() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.referral_invite_offer = Some(referral_offer());
    let (request_id, _) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should start send");

    assert!(chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Err(ReferralError::RequestTimedOut),
    ));
    let rendered = render_bottom_popup(&chat, /*width*/ 80);
    assert!(!rendered.contains("Try again"));
    assert_chatwidget_snapshot!("referral_invite_ambiguous_error", rendered);

    let (request_id, _) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should remain available");
    assert!(chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Err(ReferralError::InviteRequestFailed {
            status_code: Some(408),
        }),
    ));
    let rendered = render_bottom_popup(&chat, /*width*/ 80);
    assert!(!rendered.contains("Edit email"));
    assert!(rendered.contains("couldn't confirm"));
}

#[tokio::test]
async fn definite_referral_send_failure_can_edit_the_original_email() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.referral_invite_offer = Some(referral_offer());
    let (request_id, _) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should start send");

    assert!(chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Err(ReferralError::InviteRequestFailed {
            status_code: Some(400),
        }),
    ));
    assert_chatwidget_snapshot!(
        "referral_invite_definite_error",
        render_bottom_popup(&chat, /*width*/ 80)
    );
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_matches!(
        rx.try_recv(),
        Ok(AppEvent::OpenReferralInviteEmailPrompt {
            initial_email: Some(email)
        }) if email == "friend@example.com"
    );
}

#[tokio::test]
async fn account_change_dismisses_referral_views_and_stale_results() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    set_chatgpt_auth(&mut chat);
    chat.referral_invite_offer = Some(referral_offer());
    let (request_id, _) = chat
        .start_referral_invite_send("friend@example.com")
        .expect("eligible offer should start send");

    chat.update_account_state(
        /*status_account_display*/ None, /*plan_type*/ None,
        /*has_chatgpt_account*/ false, /*has_codex_backend_auth*/ false,
    );

    assert!(chat.bottom_pane.no_modal_or_popup_active());
    assert!(!chat.finish_referral_invite_send(
        request_id,
        "friend@example.com",
        Ok(ReferralInviteRewardStatus::Included),
    ));
}

fn referral_offer() -> PersistentReferralInviteOffer {
    PersistentReferralInviteOffer {
        expected_identity: referral_identity(),
        eligibility: PersistentReferralInviteEligibility {
            should_show: true,
            ineligible_reason: None,
            ineligible_reason_code: None,
            remaining_referrals: Some(2),
            grant_action: ReferralGrantAction::RateLimitResetCredit,
            grant_amount: 1,
            has_rewards: true,
            title: None,
            description: None,
        },
        rules: ReferralEligibilityRules {
            rules: vec!["They must be new to Codex.".to_string()],
            requires_explicit_confirmation: true,
        },
    }
}

fn referral_identity() -> ReferralIdentity {
    ReferralIdentity::new("user-1".to_string(), "account-1".to_string())
}
