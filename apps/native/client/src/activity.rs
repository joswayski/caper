//! Account-wide notifications: the inbox rows and pages the API returns.
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub actor_name: String,
    pub body: String,
    pub created_at: String,
    pub unread: bool,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub thread_root_id: Option<String>,
    pub emoji: Option<String>,
}

impl Item {
    pub fn channel(&self) -> Option<&str> {
        self.conversation_id
            .as_deref()
            .or(self.channel_id.as_deref())
    }
    pub fn can_reply(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "message" | "mention" | "reply" | "reaction"
        ) && self.message_id.as_ref().is_some_and(|id| !id.is_empty())
            && self.channel().is_some_and(|id| !id.is_empty())
    }
    pub fn draft_key(&self) -> String {
        format!(
            "inbox:{}",
            [
                self.id.as_str(),
                self.channel().unwrap_or_default(),
                self.message_id.as_deref().unwrap_or_default(),
                self.thread_root_id.as_deref().unwrap_or_default()
            ]
            .map(|part| format!("{}:{part}", part.len()))
            .join("")
        )
    }
    pub fn quote_id(&self) -> Option<&str> {
        self.thread_root_id
            .is_none()
            .then_some(self.message_id.as_deref())
            .flatten()
    }
    pub fn reply_label(&self) -> String {
        if self.thread_root_id.is_some() {
            "Reply in thread".into()
        } else if self.conversation_id.is_some() {
            "Reply in DM".into()
        } else {
            format!(
                "Reply in #{}",
                self.channel_name.as_deref().unwrap_or("channel")
            )
        }
    }
    /// Only existing destinations. Never resolve the actor's username or accept a request.
    pub fn destination(&self, api: &crate::api::Api, token: &str) -> Result<String, String> {
        if !self.can_reply() {
            return Err("This notification cannot be replied to.".into());
        }
        if let Some(id) = &self.conversation_id {
            let directs = api
                .direct_conversations(token)
                .map_err(|error| error.to_string())?;
            if !directs.conversations.iter().any(|direct| {
                &direct.id == id
                    && !direct.blocked
                    && direct.status != crate::model::DirectStatus::Incoming
            }) {
                return Err("This conversation is not available for replies. Open it to review access or requests.".into());
            }
        }
        Ok(self.channel().unwrap().to_owned())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
    pub unread_count: u64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    pub unread_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Dms,
    Mentions,
    Threads,
    Reactions,
    Invitations,
}
impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Dms,
        Self::Mentions,
        Self::Threads,
        Self::Reactions,
        Self::Invitations,
    ];
    pub fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Dms => "dms",
            Self::Mentions => "mentions",
            Self::Threads => "threads",
            Self::Reactions => "reactions",
            Self::Invitations => "invitations",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Dms => "DMs",
            Self::Mentions => "Mentions",
            Self::Threads => "Threads",
            Self::Reactions => "Reactions",
            Self::Invitations => "Invitations",
        }
    }
}

pub fn path(filter: Filter, unread: bool, cursor: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("filter", filter.wire())
        .append_pair("unread", if unread { "true" } else { "false" });
    if let Some(cursor) = cursor {
        query.append_pair("cursor", cursor);
    }
    format!("api/notifications/inbox?{}", query.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_cursor_is_encoded_and_filters_are_explicit() {
        assert_eq!(
            path(Filter::Dms, true, Some("a+/=&?")),
            "api/notifications/inbox?filter=dms&unread=true&cursor=a%2B%2F%3D%26%3F"
        );
        assert_eq!(
            Filter::ALL.map(Filter::wire),
            [
                "all",
                "dms",
                "mentions",
                "threads",
                "reactions",
                "invitations"
            ]
        );
    }
}
