//! Temporary TUI-owned referral flow backed by the existing ChatGPT APIs.

use codex_chatgpt::referrals::PersistentReferralInviteOffer;
use codex_chatgpt::referrals::ReferralError;
use uuid::Uuid;

use crate::render::line_utils::push_owned_lines;
use crate::wrapping::RtOptions;
use crate::wrapping::word_wrap_line;

use super::usage::USAGE_MENU_VIEW_ID;
use super::*;

const REFERRAL_INVITE_VIEW_ID: &str = "referral-invite";

impl ChatWidget {
    pub(crate) fn start_referral_invite_offer_refresh(&mut self) {
        self.referral_invite_offer = None;
        self.pending_referral_offer_request_id = None;
        if !self.has_chatgpt_account {
            return;
        }

        let request_id = Uuid::new_v4();
        self.pending_referral_offer_request_id = Some(request_id);
        self.app_event_tx
            .send(AppEvent::RefreshReferralInviteOffer { request_id });
    }

    pub(crate) fn finish_referral_invite_offer_refresh(
        &mut self,
        request_id: Uuid,
        result: Result<Option<PersistentReferralInviteOffer>, ReferralError>,
    ) -> bool {
        if self.pending_referral_offer_request_id != Some(request_id) {
            return false;
        }
        self.pending_referral_offer_request_id = None;
        self.referral_invite_offer = result.ok().flatten();

        let selected_index = self
            .bottom_pane
            .selected_index_for_active_view(USAGE_MENU_VIEW_ID);
        let mut params = self.usage_menu_params();
        params.initial_selected_idx = selected_index;
        let replaced = self
            .bottom_pane
            .replace_selection_view_if_present(USAGE_MENU_VIEW_ID, params);
        if replaced {
            self.request_redraw();
        }
        replaced
    }

    pub(super) fn referral_invite_menu_item(&self) -> Option<SelectionItem> {
        let offer = self.referral_invite_offer.as_ref()?;
        let description = referral_reward_description(offer);
        Some(SelectionItem {
            name: "Invite someone to Codex".to_string(),
            description: Some(description),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::OpenReferralInviteEmailPrompt {
                    initial_email: None,
                });
            })],
            dismiss_on_select: true,
            ..Default::default()
        })
    }

    pub(crate) fn show_referral_invite_email_prompt(&mut self, initial_email: Option<String>) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Invite someone to Codex".to_string(),
            "name@example.com".to_string(),
            initial_email.unwrap_or_default(),
            Some("Enter one email address, then press Enter.".to_string()),
            Box::new(move |email: String| {
                tx.send(AppEvent::OpenReferralInviteConfirmation {
                    email: email.trim().to_string(),
                });
            }),
        )
        .with_view_id(REFERRAL_INVITE_VIEW_ID);
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }

    pub(crate) fn show_referral_invite_confirmation(&mut self, email: String) {
        let Some(offer) = self.referral_invite_offer.as_ref() else {
            self.show_referral_invite_message(
                "This referral offer is no longer available. Reopen /usage to check again.",
            );
            return;
        };

        let header = referral_confirmation_header(offer, &email);
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(REFERRAL_INVITE_VIEW_ID),
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "Send invite".to_string(),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SendReferralInvite {
                            email: email.clone(),
                        });
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Cancel".to_string(),
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            initial_selected_idx: Some(1),
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn start_referral_invite_send(
        &mut self,
        email: &str,
    ) -> Option<(Uuid, codex_chatgpt::referrals::ReferralIdentity)> {
        let expected_identity = self
            .referral_invite_offer
            .as_ref()?
            .expected_identity
            .clone();
        let request_id = Uuid::new_v4();
        self.pending_referral_send_request_id = Some(request_id);
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(REFERRAL_INVITE_VIEW_ID),
            title: Some("Invite someone to Codex".to_string()),
            subtitle: Some(format!("Sending an invite to {email}...")),
            items: vec![SelectionItem {
                name: "Sending invite...".to_string(),
                is_disabled: true,
                ..Default::default()
            }],
            allow_cancel: false,
            ..Default::default()
        });
        self.request_redraw();
        Some((request_id, expected_identity))
    }

    pub(crate) fn finish_referral_invite_send(
        &mut self,
        request_id: Uuid,
        email: &str,
        result: Result<ReferralInviteRewardStatus, ReferralError>,
    ) -> bool {
        if self.pending_referral_send_request_id != Some(request_id) {
            return false;
        }
        self.pending_referral_send_request_id = None;

        let params = match result {
            Ok(ReferralInviteRewardStatus::Included) => {
                self.referral_invite_offer = None;
                referral_message_params(format!("Invite sent to {email}."))
            }
            Ok(ReferralInviteRewardStatus::NotIncluded) => {
                self.referral_invite_offer = None;
                referral_message_params(format!(
                    "Invite sent to {email}, but this invite did not include a reward."
                ))
            }
            Ok(ReferralInviteRewardStatus::Unknown) => {
                self.referral_invite_offer = None;
                referral_message_params(format!(
                    "Invite sent to {email}; reward status wasn't confirmed."
                ))
            }
            Err(error) => referral_send_error_params(&error, email),
        };
        let replaced = self
            .bottom_pane
            .replace_selection_view_if_present(REFERRAL_INVITE_VIEW_ID, params);
        if replaced {
            self.request_redraw();
        }
        replaced
    }

    pub(crate) fn clear_pending_referral_invite_requests(&mut self) {
        self.referral_invite_offer = None;
        self.pending_referral_offer_request_id = None;
        self.pending_referral_send_request_id = None;
        self.bottom_pane.dismiss_view_by_id(USAGE_MENU_VIEW_ID);
        self.bottom_pane.dismiss_view_by_id(REFERRAL_INVITE_VIEW_ID);
    }

    fn show_referral_invite_message(&mut self, message: &str) {
        self.bottom_pane
            .show_selection_view(referral_message_params(message.to_string()));
        self.request_redraw();
    }
}

fn referral_confirmation_header(
    offer: &PersistentReferralInviteOffer,
    email: &str,
) -> ColumnRenderable<'static> {
    let mut sections = Vec::new();
    if let Some(title) = &offer.eligibility.title {
        sections.push(ReferralConfirmationSection::Text(title.clone()));
    }
    sections.push(ReferralConfirmationSection::Text(
        referral_reward_description(offer),
    ));
    sections.push(ReferralConfirmationSection::Text(format!(
        "Recipient: {email}"
    )));
    sections.extend(
        offer
            .rules
            .rules
            .iter()
            .cloned()
            .map(ReferralConfirmationSection::Rule),
    );
    if offer.rules.requires_explicit_confirmation {
        sections.push(ReferralConfirmationSection::Text(
            "By sending, you confirm that you have this person's consent.".to_string(),
        ));
    }
    ColumnRenderable::with(vec![
        Box::new(Line::from("Send referral invite?").bold()) as Box<dyn Renderable>,
        Box::new(ReferralConfirmationBody { sections }),
    ])
}

enum ReferralConfirmationSection {
    Text(String),
    Rule(String),
}

struct ReferralConfirmationBody {
    sections: Vec<ReferralConfirmationSection>,
}

impl ReferralConfirmationBody {
    fn wrapped_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for section in &self.sections {
            let (text, initial_indent, subsequent_indent) = match section {
                ReferralConfirmationSection::Text(text) => (text, "", ""),
                ReferralConfirmationSection::Rule(text) => (text, "• ", "  "),
            };
            let line = Line::from(text.as_str().dim());
            let options = RtOptions::new(width.max(1) as usize)
                .initial_indent(Line::from(initial_indent.dim()))
                .subsequent_indent(Line::from(subsequent_indent.dim()));
            push_owned_lines(&word_wrap_line(&line, options), &mut lines);
        }
        lines
    }
}

impl Renderable for ReferralConfirmationBody {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Renderable::render(&Paragraph::new(self.wrapped_lines(area.width)), area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.wrapped_lines(width)
            .len()
            .try_into()
            .unwrap_or(u16::MAX)
    }
}

fn referral_reward_description(offer: &PersistentReferralInviteOffer) -> String {
    if let Some(description) = &offer.eligibility.description {
        return description.clone();
    }

    let grant_amount = offer.eligibility.grant_amount;
    match offer.eligibility.grant_action {
        codex_chatgpt::referrals::ReferralGrantAction::RateLimitResetCredit
            if grant_amount == 1 =>
        {
            "You and the recipient can each earn a usage limit reset.".to_string()
        }
        codex_chatgpt::referrals::ReferralGrantAction::RateLimitResetCredit => {
            format!("You and the recipient can each earn {grant_amount} usage limit resets.")
        }
        codex_chatgpt::referrals::ReferralGrantAction::WorkspaceCredits => {
            format!("You and the recipient can each earn {grant_amount} workspace credits.")
        }
        codex_chatgpt::referrals::ReferralGrantAction::Unknown => {
            "Send one rewarded Codex invite.".to_string()
        }
    }
}

fn referral_message_params(message: String) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(REFERRAL_INVITE_VIEW_ID),
        title: Some("Invite someone to Codex".to_string()),
        subtitle: Some(message),
        items: vec![SelectionItem {
            name: "Close".to_string(),
            dismiss_on_select: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn referral_send_error_params(error: &ReferralError, email: &str) -> SelectionViewParams {
    let mut params = referral_message_params(referral_send_error_message(error).to_string());
    if matches!(
        error,
        ReferralError::InviteRequestFailed {
            status_code: Some(status_code),
        } if is_definite_referral_rejection_status(*status_code)
    ) {
        let email = email.to_string();
        params.items.insert(
            0,
            SelectionItem {
                name: "Edit email".to_string(),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenReferralInviteEmailPrompt {
                        initial_email: Some(email.clone()),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        );
    }
    params
}

fn referral_send_error_message(error: &ReferralError) -> &'static str {
    match error {
        ReferralError::AccountChanged => {
            "Your account changed, so the invite was not sent. Reopen /usage to try again."
        }
        ReferralError::ChatGptAuthenticationRequired
        | ReferralError::UserIdUnavailable
        | ReferralError::AccountIdUnavailable => {
            "Sign in with ChatGPT, then reopen /usage to try again."
        }
        ReferralError::UnsupportedClient => {
            "Referral invites aren't available in this client session."
        }
        ReferralError::InviteRequestFailed {
            status_code: Some(status_code),
        } if is_definite_referral_rejection_status(*status_code) => {
            "The invite wasn't sent. Check the email or your eligibility."
        }
        ReferralError::RequestTimedOut
        | ReferralError::InviteRequestFailed { .. }
        | ReferralError::EligibilityRequestFailed { .. }
        | ReferralError::RulesRequestFailed { .. } => {
            "We couldn't confirm the invite. Ask the recipient before trying again."
        }
    }
}

fn is_definite_referral_rejection_status(status_code: u16) -> bool {
    (400..500).contains(&status_code) && !matches!(status_code, 408 | 499)
}
