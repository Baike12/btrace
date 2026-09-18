use async_trait::async_trait;
use langfuse_core::PgJob;
use langfuse_queue::JobProcessor;

/// Webhook 队列处理器
///
/// Holds only the HTTP client: every branch of `process` posts to a URL from
/// the job payload and never reads the database.
pub struct WebhookProcessor {
    http_client: reqwest::Client,
}

impl WebhookProcessor {
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::new(),
        }
    }
}

impl Default for WebhookProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JobProcessor for WebhookProcessor {
    async fn process(&self, job: &PgJob) -> Result<(), String> {
        let payload = &job.payload;
        let action = payload
            .get("action")
            .and_then(|a| a.as_str())
            .ok_or("Missing action in webhook payload")?;

        match action {
            "WEBHOOK" => self.handle_webhook(payload).await,
            "SLACK" => self.handle_slack(payload).await,
            "GITHUB_DISPATCH" => self.handle_github(payload).await,
            other => Err(format!("Unknown webhook action: {}", other)),
        }
    }
}

impl WebhookProcessor {
    async fn handle_webhook(&self, payload: &serde_json::Value) -> Result<(), String> {
        let url = payload.get("url").and_then(|u| u.as_str())
            .ok_or("Missing webhook URL")?;
        let body = payload.get("body").unwrap_or(&serde_json::Value::Null);

        let resp = self.http_client
            .post(url)
            .json(body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Webhook HTTP request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Webhook returned {}", resp.status()));
        }
        Ok(())
    }

    async fn handle_slack(&self, payload: &serde_json::Value) -> Result<(), String> {
        let webhook_url = payload.get("webhookUrl").and_then(|u| u.as_str())
            .ok_or("Missing Slack webhook URL")?;
        let message = payload.get("message").ok_or("Missing Slack message")?;

        let resp = self.http_client
            .post(webhook_url)
            .json(message)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Slack webhook failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Slack webhook returned {}", resp.status()));
        }
        Ok(())
    }

    async fn handle_github(&self, payload: &serde_json::Value) -> Result<(), String> {
        // GitHub repository_dispatch
        let url = payload.get("url").and_then(|u| u.as_str())
            .ok_or("Missing GitHub dispatch URL")?;
        let token = payload.get("token").and_then(|t| t.as_str())
            .ok_or("Missing GitHub token")?;
        let event_type = payload.get("eventType").and_then(|e| e.as_str())
            .ok_or("Missing event_type")?;

        let resp = self.http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/vnd.github.v3+json")
            .json(&serde_json::json!({"event_type": event_type}))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("GitHub dispatch failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("GitHub dispatch returned {}", resp.status()));
        }
        Ok(())
    }
}

/// 通知队列处理器 (comment mention notifications)
///
/// A stub: it logs and returns. It previously held a `PgPool` it never read,
/// which implied the real implementation was already querying users — it was
/// not. Add the field back together with the code that uses it.
pub struct NotificationProcessor;

impl NotificationProcessor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NotificationProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JobProcessor for NotificationProcessor {
    async fn process(&self, job: &PgJob) -> Result<(), String> {
        let notification_type = job.payload
            .get("notificationType")
            .and_then(|t| t.as_str())
            .ok_or("Missing notificationType")?;

        match notification_type {
            "COMMENT_MENTION" => {
                tracing::info!("Processing comment mention notification");
                // 发送邮件通知给被 @的用户
                // 实际实现需要查询用户 email、项目配置等
                Ok(())
            }
            other => Err(format!("Unknown notification type: {}", other)),
        }
    }
}

/// 实体变更队列处理器
pub struct EntityChangeProcessor;

impl EntityChangeProcessor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EntityChangeProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JobProcessor for EntityChangeProcessor {
    async fn process(&self, job: &PgJob) -> Result<(), String> {
        let entity_type = job.payload
            .get("entityType")
            .and_then(|t| t.as_str())
            .ok_or("Missing entityType")?;

        match entity_type {
            "prompt-version" => {
                tracing::info!("Processing prompt version change");
                Ok(())
            }
            other => Err(format!("Unknown entity type: {}", other)),
        }
    }
}

/// 监控队列处理器
///
/// A stub, like [`NotificationProcessor`]: nothing is read from the pool and no
/// request is made, so it holds neither.
pub struct MonitorProcessor;

impl MonitorProcessor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MonitorProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JobProcessor for MonitorProcessor {
    async fn process(&self, _job: &PgJob) -> Result<(), String> {
        tracing::info!("Processing monitor evaluation event");
        // 监控处理逻辑：接收 monitor evaluation 事件，
        // 匹配 alert 规则，将触发的事件入队到 webhook-queue
        Ok(())
    }
}
