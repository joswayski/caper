//! Notification levels and mutes for the account, spaces, channels and DMs.
//! Desktop shows only the controls for now (they govern phone push). Changes
//! apply at once and revert if saving fails. Pure so the rules are testable.

use crate::model::{NotificationLevel, NotificationOverride, NotificationSettings};
use chrono::{DateTime, Datelike, SecondsFormat, TimeZone, Timelike, Utc};
use std::collections::BTreeMap;

/// What a setting applies to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Account,
    Space(String),
    Channel { space: String, channel: String },
    Direct(String),
}

impl Scope {
    fn of(entry: &NotificationOverride) -> Option<Self> {
        match (&entry.space_id, &entry.channel_id, &entry.conversation_id) {
            (Some(space), Some(channel), _) => Some(Self::Channel {
                space: space.clone(),
                channel: channel.clone(),
            }),
            (Some(space), None, _) => Some(Self::Space(space.clone())),
            (None, _, Some(id)) => Some(Self::Direct(id.clone())),
            _ => None,
        }
    }

    /// An override for this scope with nothing set.
    fn empty(&self) -> NotificationOverride {
        let mut entry = NotificationOverride::default();
        match self {
            Self::Account => {}
            Self::Space(space) => entry.space_id = Some(space.clone()),
            Self::Channel { space, channel } => {
                entry.space_id = Some(space.clone());
                entry.channel_id = Some(channel.clone());
            }
            Self::Direct(id) => entry.conversation_id = Some(id.clone()),
        }
        entry
    }
}

/// One choice from a menu or Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    VoiceStarted(Option<bool>),
    /// The account level, or an override where `None` inherits. DMs use
    /// `Nothing` (off) or `None` (on).
    Level(Option<NotificationLevel>),
    /// `forever`, an RFC 3339 time, or `None` to unmute.
    Mute(Option<String>),
    /// Do Not Disturb, account only: `forever`, an RFC 3339 time, or `None`
    /// to turn it off.
    Pause(Option<String>),
}

/// A save's answer: the account's full settings, or one override.
#[derive(Debug)]
pub enum Saved {
    Settings(NotificationSettings),
    Override(NotificationOverride),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mute {
    Forever,
    Until(DateTime<Utc>),
}

/// A `mutedUntil` value as of `now`. Expired or unreadable mutes read as none.
pub fn mute(value: Option<&str>, now: DateTime<Utc>) -> Option<Mute> {
    match value? {
        "forever" => Some(Mute::Forever),
        value => DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|until| until.with_timezone(&Utc))
            .filter(|until| *until > now)
            .map(Mute::Until),
    }
}

/// The mute menu's choices, in minutes; `None` is until unmuted.
pub const MUTE_PRESETS: [(&str, Option<i64>); 5] = [
    ("For 15 minutes", Some(15)),
    ("For 1 hour", Some(60)),
    ("For 8 hours", Some(8 * 60)),
    ("For 24 hours", Some(24 * 60)),
    ("Until I turn it back on", None),
];

/// The `mutedUntil` a preset sends: `forever`, or a UTC time like
/// `2026-10-08T17:00:00Z`.
pub fn mute_value(minutes: Option<i64>, now: DateTime<Utc>) -> String {
    minutes.map_or_else(
        || "forever".into(),
        |minutes| {
            (now + chrono::Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Secs, true)
        },
    )
}

/// `Muted`, or `Muted until 5:00 PM` in local time with the date when it
/// isn't today (and the year when it isn't this year).
pub fn mute_label<Tz: TimeZone>(mute: Mute, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Mute::Until(until) = mute else {
        return "Muted".into();
    };
    let local = until.with_timezone(&now.timezone());
    let time = local.format("%-I:%M %p");
    if local.date_naive() == now.date_naive() {
        format!("Muted until {time}")
    } else if local.year() == now.year() {
        format!("Muted until {}, {time}", local.format("%b %-d"))
    } else {
        format!("Muted until {}, {time}", local.format("%b %-d, %Y"))
    }
}

/// Do Not Disturb's choices, the same on every client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PausePreset {
    Minutes(i64),
    /// 8:00 tomorrow, or 8:00 today before 5:00.
    TomorrowMorning,
    Forever,
}

pub const PAUSE_PRESETS: [(&str, PausePreset); 5] = [
    ("30 minutes", PausePreset::Minutes(30)),
    ("1 hour", PausePreset::Minutes(60)),
    ("8 hours", PausePreset::Minutes(8 * 60)),
    ("Until tomorrow morning", PausePreset::TomorrowMorning),
    ("Until I turn it off", PausePreset::Forever),
];

/// The `pausedUntil` a preset sends as of `now` (in the zone whose morning
/// counts): `forever` or a UTC time like `2026-10-08T17:00:00Z`.
pub fn pause_value<Tz: TimeZone>(preset: PausePreset, now: &DateTime<Tz>) -> String {
    let until = match preset {
        PausePreset::Forever => return "forever".into(),
        PausePreset::Minutes(minutes) => {
            now.with_timezone(&Utc) + chrono::Duration::minutes(minutes)
        }
        PausePreset::TomorrowMorning => {
            let today = now.date_naive();
            let day = if now.hour() < 5 {
                today
            } else {
                today.succ_opt().unwrap_or(today)
            };
            let morning = day.and_hms_opt(8, 0, 0).expect("valid time");
            // A clock change at 8:00 (none in practice) falls back to UTC.
            now.timezone()
                .from_local_datetime(&morning)
                .earliest()
                .map_or_else(|| morning.and_utc(), |local| local.with_timezone(&Utc))
        }
    };
    until.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// When Do Not Disturb ends, in local time: `Until 3:45 PM` today, `Until
/// tomorrow at 8:00 AM`, `Until Thu 8:00 AM` within the week, `Until Dec 3,
/// 8:00 AM` beyond it, or `Until you turn it off`.
pub fn pause_label<Tz: TimeZone>(pause: Mute, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Mute::Until(until) = pause else {
        return "Until you turn it off".into();
    };
    let local = until.with_timezone(&now.timezone());
    let time = local.format("%-I:%M %p");
    let days = (local.date_naive() - now.date_naive()).num_days();
    match days {
        ..=0 => format!("Until {time}"),
        1 => format!("Until tomorrow at {time}"),
        2..=6 => format!("Until {} {time}", local.format("%a")),
        _ => format!("Until {}, {time}", local.format("%b %-d")),
    }
}

/// A space or channel level as its menu names it.
pub fn level_label(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::All => "All messages",
        NotificationLevel::Mentions => "Only @mentions",
        NotificationLevel::Nothing => "Nothing",
    }
}

/// The account level as Settings names it.
pub fn account_level_label(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Mentions => "Only @mentions and DMs",
        level => level_label(level),
    }
}

/// The inherit choice, naming what it inherits: `Default (Only @mentions)`.
pub fn default_label(inherited: NotificationLevel) -> String {
    format!("Default ({})", level_label(inherited))
}

#[derive(Clone, Debug)]
enum Previous {
    VoiceStarted(bool),
    Level(NotificationLevel),
    Pause(Option<String>),
    Override(Option<NotificationOverride>),
}

/// The signed-in account's settings as the controls show them: the last load
/// with any changes still saving applied on top.
#[derive(Debug, Default)]
pub struct Notifications {
    settings: Option<NotificationSettings>,
    /// Scopes with a save in flight, and what to restore if it fails.
    saving: BTreeMap<Scope, Previous>,
    /// Bumps when a change starts, so a load sent earlier cannot undo it.
    revision: u64,
    loading: bool,
    load_error: Option<String>,
    /// The last failed save and its scope, shown next to that control.
    error: Option<(Scope, String)>,
}

impl Notifications {
    /// Already loaded, for labelled fixtures and tests.
    pub fn with_settings(settings: NotificationSettings) -> Self {
        Self {
            settings: Some(settings),
            ..Self::default()
        }
    }

    /// Starts a load unless one is in flight; returns the revision to send.
    pub fn start_load(&mut self) -> Option<u64> {
        if self.loading {
            return None;
        }
        self.loading = true;
        Some(self.revision)
    }

    pub fn finish_load(&mut self, revision: u64, result: Result<NotificationSettings, String>) {
        self.loading = false;
        match result {
            Ok(settings) if revision == self.revision => {
                self.settings = Some(self.with_pending(settings));
                self.load_error = None;
            }
            Ok(_) => {}
            Err(error) => self.load_error = Some(error),
        }
    }

    /// Applies `change` at once and returns whether to save it. Nothing
    /// changes before the first load, while this scope is saving, or when the
    /// choice is already current.
    pub fn begin(&mut self, scope: &Scope, change: &Change) -> bool {
        if self.saving.contains_key(scope) {
            return false;
        }
        let Some(settings) = self.settings.as_mut() else {
            return false;
        };
        let previous = match (scope, change) {
            (Scope::Account, Change::VoiceStarted(Some(enabled))) => {
                if settings.voice_started == *enabled {
                    return false;
                }
                Previous::VoiceStarted(std::mem::replace(&mut settings.voice_started, *enabled))
            }
            (Scope::Account, Change::Level(Some(level))) => {
                if settings.level == *level {
                    return false;
                }
                Previous::Level(std::mem::replace(&mut settings.level, *level))
            }
            (Scope::Account, Change::Pause(until)) => {
                if settings.paused_until == *until {
                    return false;
                }
                Previous::Pause(std::mem::replace(&mut settings.paused_until, until.clone()))
            }
            (Scope::Account, _) => return false,
            (scope, change) => {
                let previous = settings
                    .overrides
                    .iter()
                    .find(|entry| Scope::of(entry).as_ref() == Some(scope))
                    .cloned();
                let mut entry = previous.clone().unwrap_or_else(|| scope.empty());
                match change {
                    Change::VoiceStarted(enabled) => entry.voice_started = *enabled,
                    Change::Level(level) => entry.level = *level,
                    Change::Mute(until) => entry.muted_until = until.clone(),
                    Change::Pause(_) => return false,
                }
                if previous.as_ref() == Some(&entry)
                    || (previous.is_none() && entry == scope.empty())
                {
                    return false;
                }
                set_entry(&mut settings.overrides, scope, Some(entry));
                Previous::Override(previous)
            }
        };
        self.saving.insert(scope.clone(), previous);
        self.revision += 1;
        self.error = None;
        true
    }

    pub fn finish_save(&mut self, scope: &Scope, result: Result<Saved, String>) {
        let Some(previous) = self.saving.remove(scope) else {
            return;
        };
        match result {
            Ok(Saved::Settings(settings)) => {
                self.settings = Some(self.with_pending(settings));
            }
            Ok(Saved::Override(entry)) => {
                if let Some(settings) = self.settings.as_mut() {
                    set_entry(&mut settings.overrides, scope, Some(entry));
                }
            }
            Err(error) => {
                if let Some(settings) = self.settings.as_mut() {
                    match previous {
                        Previous::VoiceStarted(enabled) => settings.voice_started = enabled,
                        Previous::Level(level) => settings.level = level,
                        Previous::Pause(until) => settings.paused_until = until,
                        Previous::Override(entry) => {
                            set_entry(&mut settings.overrides, scope, entry);
                        }
                    }
                }
                self.error = Some((scope.clone(), format!("Could not save: {error}")));
            }
        }
    }

    /// `settings` from the server, keeping the values of scopes still saving.
    fn with_pending(&self, mut settings: NotificationSettings) -> NotificationSettings {
        let Some(current) = &self.settings else {
            return settings;
        };
        for scope in self.saving.keys() {
            if *scope == Scope::Account {
                settings.level = current.level;
                settings.voice_started = current.voice_started;
                settings.paused_until = current.paused_until.clone();
            } else {
                let entry = current
                    .overrides
                    .iter()
                    .find(|entry| Scope::of(entry).as_ref() == Some(scope))
                    .cloned();
                set_entry(&mut settings.overrides, scope, entry);
            }
        }
        settings
    }

    /// The account settings another device saved (`notifications.updated`,
    /// which carries no overrides). An account save in flight keeps its
    /// values; its own answer settles them.
    pub fn apply_account(&mut self, account: NotificationSettings) {
        if self.saving.contains_key(&Scope::Account) {
            return;
        }
        if let Some(settings) = self.settings.as_mut() {
            settings.level = account.level;
            settings.mobile = account.mobile;
            settings.voice_started = account.voice_started;
            settings.paused_until = account.paused_until;
        }
    }

    /// Do Not Disturb as of `now`, or `None` when it is off or has ended.
    pub fn paused(&self, now: DateTime<Utc>) -> Option<Mute> {
        mute(self.settings.as_ref()?.paused_until.as_deref(), now)
    }

    /// Whether the first load has finished.
    pub fn ready(&self) -> bool {
        self.settings.is_some()
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn saving(&self, scope: &Scope) -> bool {
        self.saving.contains_key(scope)
    }

    pub fn error(&self, scope: &Scope) -> Option<&str> {
        self.error
            .as_ref()
            .filter(|(failed, _)| failed == scope)
            .map(|(_, error)| error.as_str())
    }

    pub fn account_level(&self) -> Option<NotificationLevel> {
        self.settings.as_ref().map(|settings| settings.level)
    }

    pub fn account_voice_started(&self) -> bool {
        self.settings
            .as_ref()
            .is_none_or(|settings| settings.voice_started)
    }

    pub fn voice_started(&self, scope: &Scope) -> Option<bool> {
        self.entry(scope)?.voice_started
    }

    fn entry(&self, scope: &Scope) -> Option<&NotificationOverride> {
        self.settings
            .as_ref()?
            .overrides
            .iter()
            .find(|entry| Scope::of(entry).as_ref() == Some(scope))
    }

    /// The scope's own level; `None` inherits.
    pub fn level(&self, scope: &Scope) -> Option<NotificationLevel> {
        self.entry(scope)?.level
    }

    /// The level a space or channel inherits when its own is `None`: the
    /// space's level for a channel, then the account's, then `All`.
    pub fn inherited(&self, scope: &Scope) -> NotificationLevel {
        let space = match scope {
            Scope::Channel { space, .. } => self.level(&Scope::Space(space.clone())),
            _ => None,
        };
        space
            .or_else(|| self.account_level())
            .unwrap_or(NotificationLevel::All)
    }

    /// The scope's own mute as of `now`.
    pub fn mute(&self, scope: &Scope, now: DateTime<Utc>) -> Option<Mute> {
        mute(self.entry(scope)?.muted_until.as_deref(), now)
    }

    /// Muted itself, or (for a channel) with its space.
    pub fn muted(&self, scope: &Scope, now: DateTime<Utc>) -> bool {
        self.mute(scope, now).is_some()
            || matches!(scope, Scope::Channel { space, .. }
                if self.mute(&Scope::Space(space.clone()), now).is_some())
    }

    /// The bell-slash label for a muted space, channel or DM as of `now`
    /// (in the zone to show times in): its own mute, or "Muted with the
    /// space" for a channel in a muted space.
    pub fn muted_label<Tz: TimeZone>(&self, scope: &Scope, now: &DateTime<Tz>) -> Option<String>
    where
        Tz::Offset: std::fmt::Display,
    {
        let utc = now.with_timezone(&Utc);
        match self.mute(scope, utc) {
            Some(mute) => Some(mute_label(mute, now)),
            None => self
                .muted(scope, utc)
                .then(|| "Muted with the space".into()),
        }
    }

    /// How long until the next timed mute or Do Not Disturb ends, to
    /// redraw when it does.
    pub fn next_expiry(&self, now: DateTime<Utc>) -> Option<std::time::Duration> {
        let settings = self.settings.as_ref()?;
        settings
            .overrides
            .iter()
            .map(|entry| entry.muted_until.as_deref())
            .chain([settings.paused_until.as_deref()])
            .filter_map(|until| match mute(until, now)? {
                Mute::Until(until) => (until - now).to_std().ok(),
                Mute::Forever => None,
            })
            .min()
    }
}

/// Replaces the scope's override; `None`, or one with nothing set, removes it.
fn set_entry(
    overrides: &mut Vec<NotificationOverride>,
    scope: &Scope,
    entry: Option<NotificationOverride>,
) {
    overrides.retain(|existing| Scope::of(existing).as_ref() != Some(scope));
    if let Some(entry) = entry.filter(|entry| {
        entry.level.is_some() || entry.muted_until.is_some() || entry.voice_started.is_some()
    }) {
        overrides.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn settings() -> NotificationSettings {
        serde_json::from_value(serde_json::json!({
            "level": "mentions",
            "mobile": "whenInactive",
            "overrides": [
                {"spaceId": "spc1", "level": "nothing", "mutedUntil": null},
                {"spaceId": "spc1", "channelId": "chn1", "level": null, "mutedUntil": "2026-10-08T17:00:00Z"},
                {"conversationId": "dm1", "level": "nothing", "mutedUntil": "forever"},
                {"spaceId": "spc2", "level": "someday", "mutedUntil": "2026-10-08T09:00:00Z"}
            ]
        }))
        .unwrap()
    }

    fn loaded() -> Notifications {
        let mut notifications = Notifications::default();
        let revision = notifications.start_load().unwrap();
        notifications.finish_load(revision, Ok(settings()));
        notifications
    }

    #[test]
    fn pause_presets_labels_saves_and_pushes() {
        let new_york = FixedOffset::west_opt(5 * 3600).unwrap();
        let evening = at("2026-11-24T23:10:00Z").with_timezone(&new_york); // 6:10 PM
        let small_hours = at("2026-11-25T07:30:00Z").with_timezone(&new_york); // 2:30 AM
        let preset = |k: usize, now: &DateTime<FixedOffset>| pause_value(PAUSE_PRESETS[k].1, now);
        assert_eq!(preset(0, &evening), "2026-11-24T23:40:00Z");
        assert_eq!(preset(2, &evening), "2026-11-25T07:10:00Z");
        // Tomorrow morning is 8:00 local the next day, or today before 5:00.
        assert_eq!(preset(3, &evening), "2026-11-25T13:00:00Z");
        assert_eq!(preset(3, &small_hours), "2026-11-25T13:00:00Z");
        let five = at("2026-11-25T10:00:00Z").with_timezone(&new_york);
        assert_eq!(preset(3, &five), "2026-11-26T13:00:00Z");
        assert_eq!(preset(4, &evening), "forever");

        let label =
            |value: &str| pause_label(mute(Some(value), evening.to_utc()).unwrap(), &evening);
        assert_eq!(label("2026-11-24T23:40:00Z"), "Until 6:40 PM");
        assert_eq!(label("2026-11-25T13:00:00Z"), "Until tomorrow at 8:00 AM");
        assert_eq!(label("2026-11-27T13:00:00Z"), "Until Fri 8:00 AM");
        assert_eq!(label("2026-12-03T13:00:00Z"), "Until Dec 3, 8:00 AM");
        assert_eq!(label("forever"), "Until you turn it off");

        let mut notifications = loaded();
        let now = evening.to_utc();
        assert_eq!(notifications.paused(now), None);
        let until = preset(1, &evening);
        assert!(notifications.begin(&Scope::Account, &Change::Pause(Some(until.clone()))));
        assert_eq!(notifications.paused(now), Some(Mute::Until(at(&until))));
        assert_eq!(
            notifications.next_expiry(now),
            Some(std::time::Duration::from_secs(3600))
        );
        // Saving: another device's push doesn't undo it; a failure restores.
        let mut elsewhere = settings();
        elsewhere.paused_until = Some("forever".into());
        notifications.apply_account(elsewhere.clone());
        assert_eq!(notifications.paused(now), Some(Mute::Until(at(&until))));
        notifications.finish_save(&Scope::Account, Err("offline".into()));
        assert_eq!(notifications.paused(now), None);
        notifications.apply_account(elsewhere);
        assert_eq!(notifications.paused(now), Some(Mute::Forever));
        // Pushes carry no overrides and keep the loaded ones.
        assert_eq!(
            notifications.level(&Scope::Space("spc1".into())),
            Some(NotificationLevel::Nothing)
        );
        // An elapsed pause reads as off.
        assert_eq!(mute(Some("2026-11-24T23:00:00Z"), now), None);
    }

    #[test]
    fn voice_defaults_overrides_and_failure_restore() {
        let mut notifications = loaded();
        assert!(notifications.account_voice_started());
        let space = Scope::Space("voice-space".into());
        assert_eq!(notifications.voice_started(&space), None);
        assert!(notifications.begin(&space, &Change::VoiceStarted(Some(false))));
        assert_eq!(notifications.voice_started(&space), Some(false));
        notifications.finish_save(&space, Err("offline".into()));
        assert_eq!(notifications.voice_started(&space), None);
        assert!(notifications.begin(&Scope::Account, &Change::VoiceStarted(Some(false))));
        assert!(!notifications.account_voice_started());
        notifications.finish_save(&Scope::Account, Err("offline".into()));
        assert!(notifications.account_voice_started());
    }

    fn channel() -> Scope {
        Scope::Channel {
            space: "spc1".into(),
            channel: "chn1".into(),
        }
    }

    #[test]
    fn settings_read_levels_mutes_and_inheritance() {
        let now = at("2026-10-08T12:00:00Z");
        let notifications = loaded();
        assert_eq!(
            notifications.account_level(),
            Some(NotificationLevel::Mentions)
        );
        let space = Scope::Space("spc1".into());
        assert_eq!(
            notifications.level(&space),
            Some(NotificationLevel::Nothing)
        );
        assert_eq!(notifications.level(&channel()), None);
        assert_eq!(
            notifications.inherited(&channel()),
            NotificationLevel::Nothing,
            "a channel inherits its space"
        );
        assert_eq!(
            notifications.inherited(&space),
            NotificationLevel::Mentions,
            "a space inherits the account"
        );
        assert_eq!(
            notifications.level(&Scope::Space("spc2".into())),
            Some(NotificationLevel::Mentions),
            "unknown levels read as mentions"
        );
        assert_eq!(
            notifications.mute(&channel(), now),
            Some(Mute::Until(at("2026-10-08T17:00:00Z")))
        );
        assert!(notifications.muted(&Scope::Direct("dm1".into()), now));
        assert!(
            !notifications.muted(&Scope::Space("spc2".into()), now),
            "expired mutes read as none"
        );
        assert!(!notifications.muted(&Scope::Direct("dm2".into()), now));
        assert_eq!(
            notifications.next_expiry(now),
            Some(std::time::Duration::from_secs(5 * 3600))
        );
        assert_eq!(
            Notifications::default().inherited(&space),
            NotificationLevel::All
        );
    }

    #[test]
    fn muted_space_mutes_its_channels() {
        let now = at("2026-10-08T12:00:00Z");
        let mut notifications = loaded();
        let other = Scope::Channel {
            space: "spc1".into(),
            channel: "chn2".into(),
        };
        assert!(!notifications.muted(&other, now));
        assert!(notifications.begin(
            &Scope::Space("spc1".into()),
            &Change::Mute(Some("forever".into()))
        ));
        assert!(notifications.muted(&other, now));
        assert_eq!(
            notifications.mute(&other, now),
            None,
            "the channel's own mute is unchanged"
        );
    }

    #[test]
    fn changes_apply_at_once_and_revert_on_failure() {
        let now = at("2026-10-08T12:00:00Z");
        let mut notifications = loaded();
        let direct = Scope::Direct("dm2".into());
        assert!(notifications.begin(&direct, &Change::Mute(Some("forever".into()))));
        assert!(notifications.muted(&direct, now), "applied immediately");
        assert!(notifications.saving(&direct));
        assert!(
            !notifications.begin(&direct, &Change::Level(Some(NotificationLevel::Nothing))),
            "one save per scope at a time"
        );
        notifications.finish_save(&direct, Err("offline".into()));
        assert!(!notifications.muted(&direct, now), "reverted");
        assert!(!notifications.saving(&direct));
        assert_eq!(
            notifications.error(&direct),
            Some("Could not save: offline")
        );
        assert_eq!(notifications.error(&channel()), None);

        assert!(notifications.begin(&channel(), &Change::Mute(None)));
        assert_eq!(notifications.error(&direct), None, "a new change clears it");
        assert!(!notifications.muted(&channel(), now));
        notifications.finish_save(&channel(), Err("offline".into()));
        assert_eq!(
            notifications.mute(&channel(), now),
            Some(Mute::Until(at("2026-10-08T17:00:00Z"))),
            "the earlier mute is restored"
        );

        assert!(notifications.begin(
            &Scope::Account,
            &Change::Level(Some(NotificationLevel::All))
        ));
        assert_eq!(notifications.account_level(), Some(NotificationLevel::All));
        notifications.finish_save(&Scope::Account, Err("offline".into()));
        assert_eq!(
            notifications.account_level(),
            Some(NotificationLevel::Mentions)
        );
    }

    #[test]
    fn saves_take_the_server_answer_and_skip_no_ops() {
        let now = at("2026-10-08T12:00:00Z");
        let mut notifications = loaded();
        let direct = Scope::Direct("dm1".into());
        assert!(!notifications.begin(&direct, &Change::Mute(Some("forever".into()))));
        assert!(!notifications.begin(&Scope::Direct("dm9".into()), &Change::Mute(None)));
        assert!(!notifications.begin(
            &Scope::Account,
            &Change::Level(Some(NotificationLevel::Mentions))
        ));
        assert!(!Notifications::default().begin(&direct, &Change::Mute(None)));

        assert!(notifications.begin(&direct, &Change::Mute(None)));
        notifications.finish_save(
            &direct,
            Ok(Saved::Override(NotificationOverride {
                conversation_id: Some("dm1".into()),
                level: Some(NotificationLevel::Nothing),
                ..Default::default()
            })),
        );
        assert!(!notifications.muted(&direct, now));
        assert_eq!(
            notifications.level(&direct),
            Some(NotificationLevel::Nothing)
        );

        assert!(notifications.begin(&direct, &Change::Level(None)));
        notifications.finish_save(
            &direct,
            Ok(Saved::Override(NotificationOverride {
                conversation_id: Some("dm1".into()),
                ..Default::default()
            })),
        );
        assert!(
            notifications.entry(&direct).is_none(),
            "an override with nothing set is dropped"
        );

        assert!(notifications.begin(
            &Scope::Account,
            &Change::Level(Some(NotificationLevel::Nothing))
        ));
        let mut answer = settings();
        answer.level = NotificationLevel::Nothing;
        answer.overrides.clear();
        notifications.finish_save(&Scope::Account, Ok(Saved::Settings(answer)));
        assert_eq!(
            notifications.account_level(),
            Some(NotificationLevel::Nothing)
        );
        assert_eq!(notifications.level(&Scope::Space("spc1".into())), None);
    }

    #[test]
    fn loads_keep_changes_still_saving_and_drop_stale_answers() {
        let now = at("2026-10-08T12:00:00Z");
        let mut notifications = loaded();
        let stale = notifications.start_load().unwrap();
        assert_eq!(notifications.start_load(), None, "one load at a time");
        let direct = Scope::Direct("dm2".into());
        assert!(notifications.begin(&direct, &Change::Mute(Some("forever".into()))));
        notifications.finish_load(stale, Ok(settings()));
        assert!(
            notifications.muted(&direct, now),
            "a load sent before the change cannot undo it"
        );

        let revision = notifications.start_load().unwrap();
        let mut fresh = settings();
        fresh.level = NotificationLevel::All;
        notifications.finish_load(revision, Ok(fresh));
        assert_eq!(notifications.account_level(), Some(NotificationLevel::All));
        assert!(notifications.muted(&direct, now), "still saving: kept");

        let revision = notifications.start_load().unwrap();
        notifications.finish_load(revision, Err("offline".into()));
        assert_eq!(notifications.load_error(), Some("offline"));
        assert!(notifications.ready(), "a failed refresh keeps what loaded");
    }

    #[test]
    fn presets_and_labels_use_the_spec_copy() {
        let now = at("2026-10-08T12:00:00Z");
        assert_eq!(mute_value(Some(15), now), "2026-10-08T12:15:00Z");
        assert_eq!(mute_value(Some(24 * 60), now), "2026-10-09T12:00:00Z");
        assert_eq!(mute_value(None, now), "forever");
        assert_eq!(
            MUTE_PRESETS.map(|(label, _)| label),
            [
                "For 15 minutes",
                "For 1 hour",
                "For 8 hours",
                "For 24 hours",
                "Until I turn it back on"
            ]
        );
        let pacific = FixedOffset::west_opt(7 * 3600).unwrap();
        let local = now.with_timezone(&pacific);
        assert_eq!(mute_label(Mute::Forever, &local), "Muted");
        assert_eq!(
            mute_label(Mute::Until(at("2026-10-09T00:00:00Z")), &local),
            "Muted until 5:00 PM",
            "local time"
        );
        assert_eq!(
            mute_label(Mute::Until(at("2026-10-09T16:30:00Z")), &local),
            "Muted until Oct 9, 9:30 AM"
        );
        assert_eq!(
            mute_label(Mute::Until(at("2027-01-02T16:30:00Z")), &local),
            "Muted until Jan 2, 2027, 9:30 AM"
        );
        assert_eq!(
            default_label(NotificationLevel::Mentions),
            "Default (Only @mentions)"
        );
        assert_eq!(
            default_label(NotificationLevel::All),
            "Default (All messages)"
        );
        assert_eq!(
            account_level_label(NotificationLevel::Mentions),
            "Only @mentions and DMs"
        );
        assert_eq!(account_level_label(NotificationLevel::Nothing), "Nothing");
        assert_eq!(mute(Some("not a time"), now), None);
    }
}
