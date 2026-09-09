//! Typed portable-field intent. A delayed native save must not replace an
//! unrelated setting received from another device. No serialization runs in UI.
use crate::model::{Appearance, ImagePolicy, Preferences, ReplyDisplay};

macro_rules! portable {
    ($($field:ident: $kind:ty),+ $(,)?) => {
        #[derive(Clone, Debug, Default)]
        pub struct Edits { $(pub $field: Option<$kind>),+ }

        #[derive(Clone, Copy, Debug)]
        struct Values { $($field: $kind),+ }
        impl From<&Preferences> for Values {
            fn from(p: &Preferences) -> Self { Self { $($field: p.$field),+ } }
        }
        impl Values {
            fn apply(self, p: &mut Preferences) { $(p.$field = self.$field;)+ }
        }
        impl Edits {
            pub fn all(p: &Preferences) -> Self { Self { $($field: Some(p.$field)),+ } }
            pub fn apply(&self, p: &mut Preferences) { $(if let Some(v) = self.$field { p.$field = v; })+ }
        }

        /// Each field has its own generation, including a deliberate return to
        /// its original value while an earlier save is still awaiting its reply.
        pub(crate) struct Tracker {
            observed: Values,
            $($field: u64),+
        }
        impl Tracker {
            pub fn new(p: &Preferences) -> Self { Self { observed: p.into(), $($field: 0),+ } }
            pub fn capture(&mut self, p: &Preferences, generation: u64) {
                $(if self.observed.$field != p.$field { self.$field = generation; })+
                self.observed = p.into();
            }
            pub fn edits(&self, p: &Preferences, acknowledged: u64) -> Edits {
                Edits { $($field: (self.$field > acknowledged).then_some(p.$field)),+ }
            }
            pub fn observe(&mut self, saved: &Preferences, live: &mut Preferences, acknowledged: u64) {
                $(if self.$field <= acknowledged { live.$field = saved.$field; })+
                self.observed = (&*live).into();
            }
        }
    };
}

// Keep aligned with profile_sync::metadata::SETTINGS and its adapter test.
portable! {
    appearance: Appearance,
    reply_display: ReplyDisplay,
    image_policy: ImagePolicy,
    unified_inbox: bool,
    cross_account_moves: bool,
    group_conversations: bool,
    unread_badge: bool,
    tooltips: bool,
}

#[derive(Clone, Debug)]
pub struct Write {
    pub value: Preferences,
    pub portable: Edits,
}
impl Write {
    pub(crate) fn merge(self, current: &Preferences) -> Preferences {
        let mut value = self.value;
        Values::from(current).apply(&mut value);
        self.portable.apply(&mut value);
        value
    }
}
impl From<Preferences> for Write {
    fn from(value: Preferences) -> Self {
        Self {
            portable: Edits::all(&value),
            value,
        }
    }
}
impl std::ops::Deref for Write {
    type Target = Preferences;
    fn deref(&self) -> &Preferences {
        &self.value
    }
}
