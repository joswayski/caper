use crate::RuntimeEnvironment;
use async_trait::async_trait;
use aws_sdk_sesv2::{
    Client,
    types::{Body, Content, Destination, EmailContent, Message},
};

#[async_trait]
pub(crate) trait EmailSender: Send + Sync {
    async fn send_login_code(&self, recipient: &str, code: &str) -> Result<(), ()>;
    /// Sends the one-time note from the founder to a newly created account.
    async fn send_welcome(&self, recipient: &str) -> Result<(), ()>;
}

pub(crate) struct SesEmailSender {
    client: Client,
    from: String,
    configuration_set: String,
    welcome: Option<WelcomeSender>,
}

/// A personal sender whose replies go to an inbox someone reads. Welcome email
/// stays off until both are configured.
struct WelcomeSender {
    from: String,
    reply_to: String,
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
            "Your Caper sign-in code is {code}. It expires in 10 minutes.\n\nIf you did not request this code, you can safely ignore this email.\n\nCurious about Caper? Follow the project on GitHub: https://github.com/joswayski/caper"
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
      <p style="margin:24px 0 0;color:#5f6368;font-size:14px;line-height:1.5;">If you did not request this code, you can safely ignore this email.</p>
      <p style="margin:16px 0 0;color:#5f6368;font-size:14px;line-height:1.5;">Curious about Caper? <a href="https://github.com/joswayski/caper" style="color:#b64d32;">Follow the project on GitHub.</a></p>
    </div>
  </body>
</html>"#
        ),
    }
}

fn welcome_email() -> LoginEmail {
    LoginEmail {
        subject: "Welcome to Caper".into(),
        text: "Hey, I'm Jose. I'm building Caper, and I wanted to say thanks for signing up.\n\nCaper is early, so if something breaks, feels off, or you have an idea, I'd love to hear it. You can reply to this email any time, it comes straight to me. In the app, you can message @feedback any time to reach me. You can also find me in the Caper chat space, on X or on GitHub:\n\nCaper chat: https://caper.chat/invite/uz3HFpD8jV\nX: https://x.com/josevalerio\nGitHub: https://github.com/joswayski/caper\n\nThanks for trying it out q:^]".into(),
        html: r#"<!doctype html>
<html lang="en">
  <body style="margin:0;padding:32px 16px;background:#f3f4f5;color:#0c0d0f;font-family:Arial,sans-serif;">
    <div style="max-width:560px;margin:0 auto;padding:32px;background:#ffffff;border:1px solid #e1e3e5;border-radius:8px;font-size:16px;line-height:1.5;">
      <p style="margin:0 0 16px;">Hey, I'm Jose. I'm building Caper, and I wanted to say thanks for signing up.</p>
      <p style="margin:0 0 16px;">Caper is early, so if something breaks, feels off, or you have an idea, I'd love to hear it. You can reply to this email any time, it comes straight to me.</p>
      <p style="margin:0 0 16px;">In the app, you can message <strong>@feedback</strong> any time to reach me. You can also find me in the <a href="https://caper.chat/invite/uz3HFpD8jV" style="color:#b64d32;">Caper chat space</a>, on <a href="https://x.com/josevalerio" style="color:#b64d32;">X</a> or on <a href="https://github.com/joswayski/caper" style="color:#b64d32;">GitHub</a>.</p>
      <p style="margin:0;">Thanks for trying it out q:^]</p>
    </div>
  </body>
</html>"#
            .into(),
    }
}

fn simple_content(email: LoginEmail) -> Result<EmailContent, ()> {
    let content = |data: String| {
        Content::builder()
            .data(data)
            .charset("UTF-8")
            .build()
            .map_err(|_| ())
    };
    let body = Body::builder()
        .text(content(email.text)?)
        .html(content(email.html)?)
        .build();
    let message = Message::builder()
        .subject(content(email.subject)?)
        .body(body)
        .build();
    Ok(EmailContent::builder().simple(message).build())
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
        let optional = |name| {
            environment
                .get(name)
                .filter(|value| !value.trim().is_empty())
        };
        let welcome = match (
            optional("SES_WELCOME_FROM_ADDRESS"),
            optional("SES_WELCOME_REPLY_TO"),
        ) {
            (Some(from), Some(reply_to)) => Some(WelcomeSender { from, reply_to }),
            (None, None) => None,
            _ => {
                return Err(
                    "SES_WELCOME_FROM_ADDRESS and SES_WELCOME_REPLY_TO must be set together".into(),
                );
            }
        };
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
            welcome,
        })
    }
}

#[async_trait]
impl EmailSender for SesEmailSender {
    async fn send_login_code(&self, recipient: &str, code: &str) -> Result<(), ()> {
        let content = simple_content(login_email(code))?;
        let request = self
            .client
            .send_email()
            .from_email_address(&self.from)
            .destination(Destination::builder().to_addresses(recipient).build())
            .content(content)
            .configuration_set_name(&self.configuration_set);
        send(request, "login").await
    }

    async fn send_welcome(&self, recipient: &str) -> Result<(), ()> {
        let Some(welcome) = &self.welcome else {
            return Ok(());
        };
        let content = simple_content(welcome_email())?;
        let request = self
            .client
            .send_email()
            .from_email_address(&welcome.from)
            .reply_to_addresses(&welcome.reply_to)
            .destination(Destination::builder().to_addresses(recipient).build())
            .content(content)
            .configuration_set_name(&self.configuration_set);
        send(request, "welcome").await
    }
}

async fn send(
    request: aws_sdk_sesv2::operation::send_email::builders::SendEmailFluentBuilder,
    purpose: &'static str,
) -> Result<(), ()> {
    tokio::time::timeout(std::time::Duration::from_secs(30), request.send())
        .await
        .map_err(|_| {
            tracing::error!(purpose, kind = "timeout", "email delivery failed");
        })?
        .map(|_| ())
        .map_err(|error| {
            tracing::error!(purpose, kind = %error.as_service_error().map_or("transport", |_| "service"), "email delivery failed");
        })
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
        assert!(email.text.contains("you can safely ignore this email"));
        assert!(email.html.contains("you can safely ignore this email"));
        assert!(email.text.contains("https://github.com/joswayski/caper"));
        assert!(
            email
                .html
                .contains("href=\"https://github.com/joswayski/caper\"")
        );
    }

    #[test]
    fn welcome_invites_replies_and_links_every_contact_channel() {
        let email = welcome_email();

        assert_eq!(email.subject, "Welcome to Caper");
        for body in [&email.text, &email.html] {
            assert!(body.contains("reply to this email any time"));
            assert!(body.contains("@feedback"));
            assert!(body.contains("q:^]"));
            assert!(body.contains("https://caper.chat/invite/uz3HFpD8jV"));
            assert!(body.contains("https://x.com/josevalerio"));
            assert!(body.contains("https://github.com/joswayski/caper"));
        }
    }
}
