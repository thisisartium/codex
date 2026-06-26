use super::*;
use codex_chatgpt::referrals::ReferralError;
use pretty_assertions::assert_eq;
use uuid::Uuid;

#[test]
fn referral_event_variant_names_exclude_sensitive_values() {
    let cases = vec![
        (
            AppEvent::RefreshReferralInviteOffer {
                request_id: Uuid::nil(),
            },
            "RefreshReferralInviteOffer",
        ),
        (
            AppEvent::ReferralInviteOfferLoaded {
                request_id: Uuid::nil(),
                result: Err(ReferralError::RequestTimedOut),
            },
            "ReferralInviteOfferLoaded",
        ),
        (
            AppEvent::OpenReferralInviteEmailPrompt {
                initial_email: Some("friend@example.com".to_string()),
            },
            "OpenReferralInviteEmailPrompt",
        ),
        (
            AppEvent::OpenReferralInviteConfirmation {
                email: "friend@example.com".to_string(),
            },
            "OpenReferralInviteConfirmation",
        ),
        (
            AppEvent::SendReferralInvite {
                email: "friend@example.com".to_string(),
            },
            "SendReferralInvite",
        ),
        (
            AppEvent::ReferralInviteSent {
                request_id: Uuid::nil(),
                email: "friend@example.com".to_string(),
                result: Err(ReferralError::RequestTimedOut),
            },
            "ReferralInviteSent",
        ),
    ];

    for (event, expected) in cases {
        let variant = redacted_referral_event_variant(&event);
        assert_eq!(variant, Some(expected));
        assert!(!variant.unwrap_or_default().contains("friend@example.com"));
    }
}
