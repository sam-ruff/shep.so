//! A plain window for a launch that found an older Shep still running and
//! could not get it to quit. It never touches the owner's process or data.
use iced::{
    Element, Length, Size, Task,
    widget::{button, column, container, text},
};

#[derive(Clone, Copy, Debug)]
pub enum Message {
    Dismiss,
}

pub fn run() -> iced::Result {
    iced::application(|| ((), Task::none()), update, view)
        .title("Shep")
        .window(iced::window::Settings {
            size: Size::new(560., 180.),
            resizable: false,
            #[cfg(target_os = "linux")]
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "so.shep.Shep".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

fn update(_state: &mut (), message: Message) -> Task<Message> {
    match message {
        Message::Dismiss => iced::exit(),
    }
}

fn view(_state: &()) -> Element<'_, Message> {
    let notice = text(crate::activation::STALE_OWNER_NOTICE).size(15);
    let dismiss = button(text("OK").size(14))
        .padding([8, 24])
        .on_press(Message::Dismiss);
    container(column![notice, dismiss].spacing(20))
        .padding(28)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
