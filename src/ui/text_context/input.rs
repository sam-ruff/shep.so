use super::*;
use std::rc::Rc;

pub(super) type OnInput<'a> = Rc<dyn Fn(String) -> Message + 'a>;
/// Each value the input reports, in order, before the app redraws it. `None`
/// while the field keeps no history.
pub(super) type Edits = Rc<std::cell::RefCell<Option<Vec<String>>>>;

pub struct Input<'a> {
    native: widget::TextInput<'a, Message>,
    value: widget::text_input::Value,
    text: String,
    placeholder: String,
    id: Option<Id>,
    secure: bool,
    editable: bool,
    paste: Option<fn(&str) -> String>,
    on_input: Option<OnInput<'a>>,
    edits: Edits,
    history: bool,
}
impl<'a> Input<'a> {
    pub fn new(placeholder: &str, value: &str) -> Self {
        Self {
            native: widget::text_input(placeholder, value),
            value: widget::text_input::Value::new(value),
            text: value.into(),
            placeholder: placeholder.into(),
            id: None,
            secure: false,
            editable: false,
            paste: None,
            on_input: None,
            edits: Edits::default(),
            history: true,
        }
    }
    /// Rewrites pasted text before the input drops its control characters.
    pub fn map_paste(mut self, map: fn(&str) -> String) -> Self {
        self.paste = Some(map);
        self
    }
    /// Leaves Ctrl+Z and Ctrl+Y to the caller, which keeps its own history.
    pub fn own_history(mut self) -> Self {
        self.history = false;
        self
    }
    pub fn on_input(self, action: impl Fn(String) -> Message + 'a) -> Self {
        self.on_input_maybe(Some(action))
    }
    pub fn on_input_maybe(mut self, action: Option<impl Fn(String) -> Message + 'a>) -> Self {
        let action = action.map(|action| Rc::new(action) as OnInput<'a>);
        self.editable = action.is_some();
        let edits = self.edits.clone();
        self.native = self.native.on_input_maybe(action.clone().map(|action| {
            move |value: String| {
                if let Some(edits) = edits.borrow_mut().as_mut() {
                    edits.push(value.clone());
                }
                action(value)
            }
        }));
        self.on_input = action;
        self
    }
    pub fn on_submit(mut self, message: Message) -> Self {
        self.native = self.native.on_submit(message);
        self
    }
    pub fn id(mut self, id: impl Into<Id>) -> Self {
        let id = id.into();
        self.native = self.native.id(id.clone());
        self.id = Some(id);
        self
    }
    pub fn secure(mut self, secure: bool) -> Self {
        self.native = self.native.secure(secure);
        self.secure = secure;
        if secure {
            self.value = widget::text_input::Value::new(&"*".repeat(self.value.len()));
        }
        self
    }
    pub fn size(mut self, size: impl Into<iced::Pixels>) -> Self {
        self.native = self.native.size(size);
        self
    }
    pub fn padding(mut self, padding: impl Into<iced::Padding>) -> Self {
        self.native = self.native.padding(padding);
        self
    }
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.native = self.native.width(width);
        self
    }
    pub fn style(
        mut self,
        style: impl Fn(&Theme, widget::text_input::Status) -> widget::text_input::Style + 'a,
    ) -> Self {
        self.native = self.native.style(style);
        self
    }
}
impl<'a> From<Input<'a>> for Element<'a, Message> {
    fn from(input: Input<'a>) -> Self {
        // Secret values are never copied into an undo history.
        let undo = input
            .on_input
            .filter(|_| input.history && !input.secure)
            .map(|publish| {
                *input.edits.borrow_mut() = Some(Vec::new());
                Undo {
                    text: input.text,
                    publish,
                    edits: input.edits,
                }
            });
        TextContext {
            content: input.native.into(),
            kind: Kind::Input {
                value: input.value,
                editable: input.editable,
                secure: input.secure,
                paste: input.paste,
                undo,
            },
            identity: Identity::Input(input.id, input.placeholder),
            theme: None,
        }
        .into()
    }
}
