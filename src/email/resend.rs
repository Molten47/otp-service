// src/email/resend.rs
use super::{EmailSender, OutboundEmail, SendError, SendReceipt};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct ResendSender {
    client: reqwest::Client,
    api_key: String,
    from: String,
}

impl ResendSender {
    pub fn new(client: reqwest::Client, api_key: String, from: String) -> Self {
        Self { client, api_key, from }
    }
}

#[derive(Serialize)]
struct ResendRequest<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    text: &'a str,
}

#[derive(Deserialize)]
struct ResendResponse {
    id: String,
}

#[derive(Deserialize)]
struct ResendErrorBody {
    message: String,
}

#[async_trait]
impl EmailSender for ResendSender {
    async fn send(&self, email: OutboundEmail) -> Result<SendReceipt, SendError> {
        let body = ResendRequest {
            from: &self.from,
            to: [&email.to],
            subject: &email.subject,
            text: &email.body_text,
        };

        let response = self
            .client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            let parsed: ResendResponse = response
                .json()
                .await
                .map_err(|e| SendError::UnexpectedResponse(e.to_string()))?;
            Ok(SendReceipt { provider_message_id: parsed.id })
        } else {
            let text = response.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<ResendErrorBody>(&text)
                .map(|b| b.message)
                .unwrap_or(text);
            Err(SendError::Rejected(format!("{status}: {msg}")))
        }
    }
}