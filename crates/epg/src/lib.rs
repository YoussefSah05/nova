//! XMLTV EPG parser and normalizer.
//!
//! Implemented in Phase 4: streaming `<programme>` reader that emits one
//! [`Program`] at a time so callers can batch-insert into SQLite without
//! buffering hundreds of MB of XMLTV in memory.
//!
//! See `docs/reference/xmltv-epg.md` for the format, timezone, and channel-ID
//! matching rules this crate implements.

pub mod model;
pub mod parser;
pub mod time;

pub use model::{
    channel_id_matches, channel_logo_key, match_channel_icons, ChannelIcon, EpgError, Program,
};
pub use parser::{parse, Programs};
pub use time::xmltv_to_epoch;
