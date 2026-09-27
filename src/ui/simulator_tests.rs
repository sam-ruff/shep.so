//! Fast headless UI scenarios built on `iced_test`.
//!
//! Each scenario runs the real `App` against the demo engine and its in-memory
//! fixture store, and delivers input through the real widget tree. See
//! `docs/agents/simulator-tests.md` for what belongs here rather than in the
//! native MCP suite.
use super::*;

mod harness;
mod mail;
mod preferences;
mod shortcuts;

use harness::Harness;
