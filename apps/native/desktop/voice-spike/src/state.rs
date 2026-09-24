#![allow(dead_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallContext {
    pub channel_id: String,
    pub channel_name: String,
    pub space_name: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioIntent {
    pub muted: bool,
    pub deafened: bool,
}

impl AudioIntent {
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if !muted {
            self.deafened = false;
        }
    }

    pub fn set_deafened(&mut self, deafened: bool) {
        if deafened == self.deafened {
            return;
        }
        self.deafened = deafened;
        self.muted = deafened;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Joining(CallContext),
    Connected(CallContext),
    Reconnecting(CallContext),
    Failed(CallContext),
}

#[derive(Debug)]
pub struct CallState {
    pub phase: Phase,
    pub audio: AudioIntent,
    pub generation: u64,
    pub browsed_channel: Option<String>,
}

impl Default for CallState {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            audio: AudioIntent::default(),
            generation: 0,
            browsed_channel: None,
        }
    }
}

impl CallState {
    pub fn active_channel(&self) -> Option<&str> {
        match &self.phase {
            Phase::Joining(context)
            | Phase::Connected(context)
            | Phase::Reconnecting(context)
            | Phase::Failed(context) => Some(&context.channel_id),
            Phase::Idle => None,
        }
    }

    pub fn browse(&mut self, channel_id: String) {
        self.browsed_channel = Some(channel_id);
    }

    pub fn join(&mut self, context: CallContext) -> u64 {
        self.generation += 1;
        self.phase = Phase::Joining(context);
        self.generation
    }

    pub fn connected(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        let (Phase::Joining(context) | Phase::Reconnecting(context)) = &self.phase else {
            return false;
        };
        self.phase = Phase::Connected(context.clone());
        true
    }

    pub fn reconnecting(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        let Phase::Connected(context) = &self.phase else {
            return false;
        };
        self.phase = Phase::Reconnecting(context.clone());
        true
    }

    pub fn leave_now(&mut self) -> u64 {
        self.generation += 1;
        self.phase = Phase::Idle;
        self.generation
    }

    pub fn revoke_channel(&mut self, channel_id: &str) -> bool {
        let active = match &self.phase {
            Phase::Joining(context)
            | Phase::Connected(context)
            | Phase::Reconnecting(context)
            | Phase::Failed(context) => context.channel_id == channel_id,
            Phase::Idle => false,
        };
        if active {
            self.leave_now();
        }
        active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(channel: &str) -> CallContext {
        CallContext {
            channel_id: channel.into(),
            channel_name: channel.into(),
            space_name: "Fixture Studio".into(),
        }
    }

    #[test]
    fn browsing_does_not_replace_or_leave_active_voice() {
        let mut state = CallState::default();
        let generation = state.join(context("general"));
        assert!(state.connected(generation));
        state.browse("planning".into());
        assert!(matches!(
            state.phase,
            Phase::Connected(CallContext { ref channel_id, .. }) if channel_id == "general"
        ));
    }

    #[test]
    fn explicit_join_replaces_context_and_rejects_old_completion() {
        let mut state = CallState::default();
        let old = state.join(context("general"));
        let current = state.join(context("planning"));
        assert!(!state.connected(old));
        assert!(state.connected(current));
        assert!(matches!(
            state.phase,
            Phase::Connected(CallContext { ref channel_id, .. }) if channel_id == "planning"
        ));
    }

    #[test]
    fn logout_or_access_revocation_tears_down_active_context_synchronously() {
        let mut state = CallState::default();
        let generation = state.join(context("private"));
        assert!(state.connected(generation));
        assert!(!state.revoke_channel("other"));
        assert!(state.revoke_channel("private"));
        assert_eq!(state.phase, Phase::Idle);
        assert!(!state.connected(generation));
    }

    #[test]
    fn unmute_undeafens_and_undeafen_unmutes_but_repeated_false_preserves_mute() {
        let mut audio = AudioIntent::default();
        audio.set_deafened(true);
        assert_eq!((audio.muted, audio.deafened), (true, true));
        audio.set_muted(false);
        assert_eq!((audio.muted, audio.deafened), (false, false));

        audio.set_muted(true);
        audio.set_deafened(true);
        audio.set_deafened(false);
        assert_eq!((audio.muted, audio.deafened), (false, false));
        audio.set_muted(true);
        audio.set_deafened(false);
        assert!(audio.muted, "idempotent undeafen preserves mute");
    }
}
