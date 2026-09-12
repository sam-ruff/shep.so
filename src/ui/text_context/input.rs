use super::*;

pub struct Input<'a> {
    native: widget::TextInput<'a, Message>,
    value: widget::text_input::Value,
    placeholder: String,
    id: Option<Id>,
    secure: bool,
    editable: bool,
}
impl<'a> Input<'a> {
    pub fn new(placeholder: &str, value: &str) -> Self {
        Self {
            native: widget::text_input(placeholder, value),
            value: widget::text_input::Value::new(value),
            placeholder: placeholder.into(),
            id: None,
            secure: false,
            editable: false,
        }
    }
    pub fn on_input(mut self, action: impl Fn(String) -> Message + 'a) -> Self {
        self.native = self.native.on_input(action);
        self.editable = true;
        self
    }
    pub fn on_input_maybe(mut self, action: Option<impl Fn(String) -> Message + 'a>) -> Self {
        self.editable = action.is_some();
        self.native = self.native.on_input_maybe(action);
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
        TextContext {
            content: input.native.into(),
            kind: Kind::Input {
                value: input.value,
                editable: input.editable,
                secure: input.secure,
            },
            identity: Identity::Input(input.id, input.placeholder),
            theme: None,
        }
        .into()
    }
}
