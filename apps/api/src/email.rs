use async_trait::async_trait;
use aws_sdk_sesv2::{
    Client,
    types::{Body, Content, Destination, EmailContent, Message},
};

#[async_trait]
pub(crate) trait EmailSender: Send + Sync {
    async fn send_login_code(&self, recipient: &str, code: &str) -> Result<(), ()>;
}

pub(crate) struct SesEmailSender {
    client: Client,
    from: String,
    configuration_set: String,
}

impl SesEmailSender {
    pub(crate) async fn from_env() -> Result<Self, String> {
        let get = |name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{name} is required when AUTH_SECRET is set"))
        };
        let from = get("SES_FROM_ADDRESS")?;
        let configuration_set = get("SES_CONFIGURATION_SET")?;
        if std::env::var("AWS_REGION").is_err() && std::env::var("AWS_DEFAULT_REGION").is_err() {
            return Err("AWS_REGION is required when AUTH_SECRET is set".into());
        }
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Ok(Self {
            client: Client::new(&config),
            from,
            configuration_set,
        })
    }
}

#[async_trait]
impl EmailSender for SesEmailSender {
    async fn send_login_code(&self, recipient: &str, code: &str) -> Result<(), ()> {
        let destination = Destination::builder().to_addresses(recipient).build();
        let subject = Content::builder()
            .data("Your Caper sign-in code")
            .charset("UTF-8")
            .build()
            .map_err(|_| ())?;
        let text = Content::builder()
            .data(format!(
                "Your Caper sign-in code is {code}. It expires in 10 minutes.\n\nIf you did not request this code, you can ignore this email."
            ))
            .charset("UTF-8")
            .build()
            .map_err(|_| ())?;
        let body = Body::builder().text(text).build();
        let message = Message::builder().subject(subject).body(body).build();
        let content = EmailContent::builder().simple(message).build();

        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.client
                .send_email()
                .from_email_address(&self.from)
                .destination(destination)
                .content(content)
                .configuration_set_name(&self.configuration_set)
                .send(),
        )
        .await
        .map_err(|_| {
            tracing::error!(kind = "timeout", "login email delivery failed");
        })?
            .map(|_| ())
            .map_err(|error| {
                tracing::error!(kind = %error.as_service_error().map_or("transport", |_| "service"), "login email delivery failed");
            })
    }
}
