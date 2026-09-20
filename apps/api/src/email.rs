use crate::RuntimeEnvironment;
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

struct LoginEmail {
    subject: String,
    text: String,
    html: String,
}

fn login_email(code: &str) -> LoginEmail {
    LoginEmail {
        subject: format!("{code} is your Caper sign-in code"),
        text: format!(
            "Your Caper sign-in code is {code}. It expires in 10 minutes.\n\nIf you did not request this code, you can ignore this email."
        ),
        html: format!(
            r#"<!doctype html>
<html lang="en">
  <body style="margin:0;padding:32px 16px;background:#f3f4f5;color:#0c0d0f;font-family:Arial,sans-serif;">
    <div style="max-width:560px;margin:0 auto;padding:32px;background:#ffffff;border:1px solid #e1e3e5;border-radius:8px;">
      <h1 style="margin:0 0 24px;font-size:28px;line-height:1.2;">Your Caper sign-in code</h1>
      <p style="margin:0;font-size:16px;line-height:1.5;">Enter this code to sign in to Caper:</p>
      <p style="margin:32px 0;text-align:center;">
        <strong style="font-size:36px;line-height:1;letter-spacing:6px;">{code}</strong>
      </p>
      <p style="margin:0;font-size:16px;line-height:1.5;">It expires in 10 minutes.</p>
      <p style="margin:24px 0 0;color:#5f6368;font-size:14px;line-height:1.5;">If you did not request this code, you can ignore this email.</p>
    </div>
  </body>
</html>"#
        ),
    }
}

impl SesEmailSender {
    pub(crate) async fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        let get = |name| {
            environment
                .get(name)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{name} is required when AUTH_SECRET is set"))
        };
        let from = get("SES_FROM_ADDRESS")?;
        let configuration_set = get("SES_CONFIGURATION_SET")?;
        if environment.get("AWS_REGION").is_none()
            && environment.get("AWS_DEFAULT_REGION").is_none()
        {
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
        let email = login_email(code);
        let subject = Content::builder()
            .data(email.subject)
            .charset("UTF-8")
            .build()
            .map_err(|_| ())?;
        let text = Content::builder()
            .data(email.text)
            .charset("UTF-8")
            .build()
            .map_err(|_| ())?;
        let html = Content::builder()
            .data(email.html)
            .charset("UTF-8")
            .build()
            .map_err(|_| ())?;
        let body = Body::builder().text(text).html(html).build();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_code_is_prominent_in_subject_and_html_with_text_fallback() {
        let email = login_email("A7K29Z");

        assert_eq!(email.subject, "A7K29Z is your Caper sign-in code");
        assert!(email.text.contains("sign-in code is A7K29Z"));
        assert!(email.html.contains(">A7K29Z</strong>"));
        assert!(email.html.contains("font-size:36px"));
    }
}
