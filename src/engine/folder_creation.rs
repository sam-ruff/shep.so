use super::*;
use crate::folders::Mailbox;
mod durable;

#[derive(Debug, Clone)]
pub struct Request {
    pub serial: u64,
    pub account: String,
    pub connection: String,
    pub parent: Option<String>,
    pub name: String,
}
