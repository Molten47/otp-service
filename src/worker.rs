
use crate::{email::EmailSender, state::SendJob};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::mpsc;


pub fn spawn_pool(
    worker_count: usize,
    job_rx: mpsc::Receiver<SendJob>,
    db: PgPool,
    sender: Arc<dyn EmailSender>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));
    let mut handles = Vec::with_capacity(worker_count);

    for worker_id in 0..worker_count {
        let job_rx = job_rx.clone();
        let db = db.clone();
        let sender = sender.clone();

        let handle = tokio::spawn(async move {
            loop {
                let job = {
                    let mut rx = job_rx.lock().await;
                    rx.recv().await
                };

                let Some(job) = job else {
                    tracing::info!(worker_id, "channel closed, worker shutting down");
                    break;
                };

                handle_job(worker_id, &db, &sender, job).await;
            }
        });

        handles.push(handle);
    }

    handles
}

async fn handle_job(worker_id: usize, db: &PgPool, sender: &Arc<dyn EmailSender>, job: SendJob) {
    use crate::email::OutboundEmail;

    let email = OutboundEmail {
        to: job.email.clone(),
        subject: "Your verification code".to_string(),
        body_text: format!("Your code is {}. It expires in 5 minutes.", job.code),
    };

    match sender.send(email).await {
        Ok(receipt) => {
            tracing::info!(worker_id, otp_id = %job.otp_id, "email accepted by provider");
            let _ = sqlx::query!(
                "update otps set provider_message_id = $1, sent_at = now() where id = $2",
                receipt.provider_message_id,
                job.otp_id,
            )
            .execute(db)
            .await;
        }
        Err(e) => {
            tracing::warn!(worker_id, otp_id = %job.otp_id, error = %e, "send failed");
            let _ = sqlx::query!(
                "update otps set send_error = $1 where id = $2",
                e.to_string(),
                job.otp_id,
            )
            .execute(db)
            .await;
        }
    }
}