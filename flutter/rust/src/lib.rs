pub mod api;
mod database;
mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */
mod operations;

#[cfg(test)]
mod tests;

mod drafts;
#[cfg(test)]
mod forward_tests;

mod outgoing;
#[cfg(test)]
mod outgoing_tests;

mod sent;
#[cfg(test)]
mod sent_tests;

mod accounts;
#[cfg(test)]
mod accounts_tests;

mod connections;
#[cfg(test)]
mod connections_tests;
