
use async_trait::async_trait;

pub mod resend;

#[derive(Debug, Clone)]
pub struct OutboundEmail {
    pub to: String,
    pub subject: String,
    pub body_text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("provider rejected the request: {0}")]
    Rejected(String),
    #[error("network error talking to provider: {0}")]
    Network(#[from] reqwest::Error),
    #[error("provider returned an unexpected response: {0}")]
    UnexpectedResponse(String),
}

#[derive(Debug, Clone)]
pub struct SendReceipt {
    pub provider_message_id: String,
}

#[async_trait]
pub trait EmailSender: Send + Sync {
    async fn send(&self, email: OutboundEmail) -> Result<SendReceipt, SendError>;
}