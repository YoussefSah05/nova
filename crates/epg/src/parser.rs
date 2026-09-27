//! Streaming XMLTV parser.
//!
//! Iterates over `<programme>` elements in a (possibly very large) XMLTV
//! document and emits one [`Program`] per element without buffering the
//! whole document — `quick-xml` event mode, not DOM. `<channel>` icons met
//! along the way are collected on the side ([`Programs::take_channel_icons`]).
//! See
//! `docs/reference/xmltv-epg.md` for parsing rules.

use std::io::BufRead;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use quick_xml::Reader;

use crate::model::{ChannelIcon, EpgError, Program};
use crate::time::xmltv_to_epoch;

/// Streaming iterator over `<programme>` elements. Construct via [`parse`].
pub struct Programs<R: BufRead> {
    reader: Reader<R>,
    buf: Vec<u8>,
    inner_buf: Vec<u8>,
    channel_icons: Vec<ChannelIcon>,
}

/// Begin parsing an XMLTV document. The returned iterator yields one
/// [`Program`] per `<programme>` element. A `<channel>` element contributes its
/// first `<icon src>` to [`Programs::take_channel_icons`]; other content is
/// skipped (channel ID matching happens against `programme.channel` at the
/// storage layer).
pub fn parse<R: BufRead>(reader: R) -> Programs<R> {
    let mut reader = Reader::from_reader(reader);
    reader.config_mut().trim_text(true);
    Programs {
        reader,
        buf: Vec::with_capacity(4096),
        inner_buf: Vec::with_capacity(4096),
        channel_icons: Vec::new(),
    }
}

impl<R: BufRead> Iterator for Programs<R> {
    type Item = Result<Program, EpgError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.buf.clear();
            // Extract the programme's attribute set from `Event::Start` into
            // owned values BEFORE recursing into `parse_programme`, since that
            // method reuses the same reader (and thus the same outer buffer).
            let header = match self.reader.read_event_into(&mut self.buf) {
                Ok(Event::Start(start)) if start.local_name().as_ref() == b"programme" => {
                    match extract_programme_header(&start) {
                        Ok(h) => h,
                        Err(e) => return Some(Err(e)),
                    }
                }
                Ok(Event::Start(start)) if start.local_name().as_ref() == b"channel" => {
                    let id = match channel_id(&start) {
                        Ok(id) => id,
                        Err(e) => return Some(Err(e)),
                    };
                    if let Err(e) = self.parse_channel(id) {
                        return Some(Err(e));
                    }
                    continue;
                }
                Ok(Event::Eof) => return None,
                Ok(_) => continue,
                Err(e) => return Some(Err(EpgError::from(e))),
            };

            match self.parse_programme(header) {
                Ok(Some(prog)) => return Some(Ok(prog)),
                Ok(None) => continue, // skipped (malformed times / missing fields)
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

struct ProgrammeHeader {
    channel_tvg_id: Option<String>,
    start_utc: Option<i64>,
    stop_utc: Option<i64>,
}

fn extract_programme_header(start: &BytesStart<'_>) -> Result<ProgrammeHeader, EpgError> {
    let mut h = ProgrammeHeader {
        channel_tvg_id: None,
        start_utc: None,
        stop_utc: None,
    };
    for attr in start.attributes().with_checks(false).flatten() {
        match attr.key.as_ref() {
            b"channel" => h.channel_tvg_id = Some(decode_attr(&attr.value)?),
            b"start" => h.start_utc = xmltv_to_epoch(&decode_attr(&attr.value)?),
            b"stop" => h.stop_utc = xmltv_to_epoch(&decode_attr(&attr.value)?),
            _ => {}
        }
    }
    Ok(h)
}

fn channel_id(start: &BytesStart<'_>) -> Result<Option<String>, EpgError> {
    for attr in start.attributes().with_checks(false).flatten() {
        if attr.key.as_ref() == b"id" {
            let id = decode_attr(&attr.value)?;
            return Ok(Some(id.trim().to_string()).filter(|s| !s.is_empty()));
        }
    }
    Ok(None)
}

fn icon_src(start: &BytesStart<'_>) -> Result<Option<String>, EpgError> {
    for attr in start.attributes().with_checks(false).flatten() {
        if attr.key.as_ref() == b"src" {
            return Ok(Some(decode_attr(&attr.value)?));
        }
    }
    Ok(None)
}

impl<R: BufRead> Programs<R> {
    /// The `<channel>` icons seen so far (all of them once the iterator is
    /// exhausted), leaving the list empty.
    pub fn take_channel_icons(&mut self) -> Vec<ChannelIcon> {
        std::mem::take(&mut self.channel_icons)
    }

    /// Consume one `<channel>` element, keeping its first icon.
    fn parse_channel(&mut self, id: Option<String>) -> Result<(), EpgError> {
        let mut icon: Option<String> = None;
        loop {
            self.inner_buf.clear();
            let skip: Option<Vec<u8>> = match self.reader.read_event_into(&mut self.inner_buf) {
                Ok(Event::Start(inner)) => {
                    if inner.local_name().as_ref() == b"icon" && icon.is_none() {
                        icon = icon_src(&inner)?;
                    }
                    Some(inner.local_name().as_ref().to_vec())
                }
                Ok(Event::Empty(inner)) => {
                    if inner.local_name().as_ref() == b"icon" && icon.is_none() {
                        icon = icon_src(&inner)?;
                    }
                    None
                }
                Ok(Event::End(end)) if end.local_name().as_ref() == b"channel" => break,
                Ok(Event::Eof) => break,
                Ok(_) => None,
                Err(e) => return Err(EpgError::from(e)),
            };
            if let Some(name) = skip {
                self.skip_to_end(&name)?;
            }
        }
        if let (Some(channel_id), Some(icon_url)) = (
            id,
            icon.as_deref()
                .and_then(nova_core::artwork::plausible_image_url),
        ) {
            self.channel_icons.push(ChannelIcon {
                channel_id,
                icon_url,
            });
        }
        Ok(())
    }

    fn parse_programme(&mut self, header: ProgrammeHeader) -> Result<Option<Program>, EpgError> {
        let ProgrammeHeader {
            channel_tvg_id,
            start_utc,
            stop_utc,
        } = header;

        // Drain the rest of <programme> regardless of whether we keep it, so the
        // outer loop is always positioned at the next element.
        let mut title: Option<String> = None;
        let mut sub_title: Option<String> = None;
        let mut description: Option<String> = None;
        let mut category: Option<String> = None;
        let mut icon_url: Option<String> = None;

        // Outcome of consuming one event from the inner buffer. All borrowed
        // data is copied out before `inner_buf` is reused by the next read,
        // because `self.read_text` / `self.skip_to_end` need the buffer too.
        enum Action {
            ReadText(ChildKind, Vec<u8>),
            SkipUnknown(Vec<u8>),
            Icon(Option<String>, Option<Vec<u8>>), // name=None for self-closing
            EndProgramme,
            Eof,
            Skip,
        }
        enum ChildKind {
            Title,
            SubTitle,
            Desc,
            Category,
        }

        loop {
            self.inner_buf.clear();
            let action: Action = match self.reader.read_event_into(&mut self.inner_buf) {
                Ok(Event::Start(inner)) => {
                    let name = inner.local_name().as_ref().to_vec();
                    match name.as_slice() {
                        b"title" => Action::ReadText(ChildKind::Title, name),
                        b"sub-title" => Action::ReadText(ChildKind::SubTitle, name),
                        b"desc" => Action::ReadText(ChildKind::Desc, name),
                        b"category" => Action::ReadText(ChildKind::Category, name),
                        b"icon" => {
                            let mut src = None;
                            for attr in inner.attributes().with_checks(false).flatten() {
                                if attr.key.as_ref() == b"src" {
                                    src = Some(decode_attr(&attr.value)?);
                                }
                            }
                            Action::Icon(src, Some(name))
                        }
                        _ => Action::SkipUnknown(name),
                    }
                }
                Ok(Event::Empty(inner)) => {
                    if inner.local_name().as_ref() == b"icon" {
                        let mut src = None;
                        for attr in inner.attributes().with_checks(false).flatten() {
                            if attr.key.as_ref() == b"src" {
                                src = Some(decode_attr(&attr.value)?);
                            }
                        }
                        Action::Icon(src, None)
                    } else {
                        Action::Skip
                    }
                }
                Ok(Event::End(end)) if end.local_name().as_ref() == b"programme" => {
                    Action::EndProgramme
                }
                Ok(Event::Eof) => Action::Eof,
                Ok(_) => Action::Skip,
                Err(e) => return Err(EpgError::from(e)),
            };

            match action {
                Action::ReadText(ChildKind::Title, name) => title = self.read_text(&name).ok(),
                Action::ReadText(ChildKind::SubTitle, name) => {
                    sub_title = self.read_text(&name).ok()
                }
                Action::ReadText(ChildKind::Desc, name) => description = self.read_text(&name).ok(),
                Action::ReadText(ChildKind::Category, name) => {
                    // Only keep the first category — XMLTV often emits one per language.
                    if category.is_none() {
                        category = self.read_text(&name).ok();
                    } else {
                        self.skip_to_end(&name)?;
                    }
                }
                Action::SkipUnknown(name) => self.skip_to_end(&name)?,
                Action::Icon(src, name) => {
                    if src.is_some() {
                        icon_url = src;
                    }
                    if let Some(name) = name {
                        self.skip_to_end(&name)?;
                    }
                }
                Action::EndProgramme | Action::Eof => break,
                Action::Skip => continue,
            }
        }

        let (Some(channel_tvg_id), Some(start_utc), Some(stop_utc), Some(title)) =
            (channel_tvg_id, start_utc, stop_utc, title)
        else {
            return Ok(None); // skip malformed programmes
        };

        if stop_utc <= start_utc {
            return Ok(None); // skip zero/negative-length programmes
        }

        Ok(Some(Program {
            channel_tvg_id,
            title,
            sub_title,
            description,
            start_utc,
            stop_utc,
            category,
            icon_url,
        }))
    }

    /// Reads the text content of the currently-open element and consumes its
    /// closing tag. Handles mixed CDATA / Text segments by concatenating them.
    fn read_text(&mut self, end_name: &[u8]) -> Result<String, EpgError> {
        let mut out = String::new();
        loop {
            self.inner_buf.clear();
            match self.reader.read_event_into(&mut self.inner_buf) {
                Ok(Event::Text(t)) => {
                    let s = t.unescape().map_err(EpgError::from)?;
                    out.push_str(&s);
                }
                Ok(Event::CData(c)) => {
                    out.push_str(std::str::from_utf8(c.as_ref())?);
                }
                Ok(Event::End(e)) if e.local_name().as_ref() == end_name => break,
                Ok(Event::Eof) => break,
                Ok(_) => continue,
                Err(e) => return Err(EpgError::from(e)),
            }
        }
        Ok(out)
    }

    /// Consumes events until the matching end tag for `name` is seen.
    fn skip_to_end(&mut self, name: &[u8]) -> Result<(), EpgError> {
        let depth_target = QName(name);
        self.reader
            .read_to_end_into(depth_target, &mut self.inner_buf)?;
        Ok(())
    }
}

fn decode_attr(bytes: &[u8]) -> Result<String, EpgError> {
    Ok(std::str::from_utf8(bytes)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_programme() {
        let xml = br#"<?xml version="1.0"?>
        <tv>
          <channel id="bbc1.uk"><display-name>BBC One</display-name></channel>
          <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="bbc1.uk">
            <title lang="en">Test Programme</title>
            <sub-title>Pilot</sub-title>
            <desc>An example.</desc>
            <category lang="en">Drama</category>
            <icon src="http://example.com/img.png" />
          </programme>
        </tv>"#;

        let progs: Vec<_> = parse(xml.as_slice()).collect::<Result<_, _>>().unwrap();
        assert_eq!(progs.len(), 1);
        let p = &progs[0];
        assert_eq!(p.channel_tvg_id, "bbc1.uk");
        assert_eq!(p.title, "Test Programme");
        assert_eq!(p.sub_title.as_deref(), Some("Pilot"));
        assert_eq!(p.description.as_deref(), Some("An example."));
        assert_eq!(p.category.as_deref(), Some("Drama"));
        assert_eq!(p.icon_url.as_deref(), Some("http://example.com/img.png"));
        assert_eq!(p.start_utc, 1704110400); // 2024-01-01 12:00:00 UTC
        assert_eq!(p.stop_utc, 1704114000);
    }

    #[test]
    fn skips_malformed_programmes() {
        let xml = br#"<tv>
          <programme start="bogus" stop="20240101130000" channel="c1">
            <title>Skipped</title>
          </programme>
          <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="c2">
            <title>Kept</title>
          </programme>
        </tv>"#;

        let progs: Vec<_> = parse(xml.as_slice()).collect::<Result<_, _>>().unwrap();
        assert_eq!(progs.len(), 1);
        assert_eq!(progs[0].title, "Kept");
        assert_eq!(progs[0].channel_tvg_id, "c2");
    }

    #[test]
    fn handles_multiple_programmes_on_different_channels() {
        let xml = br#"<tv>
          <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="a">
            <title>A1</title>
          </programme>
          <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="b">
            <title>B1</title>
          </programme>
          <programme start="20240101130000 +0000" stop="20240101140000 +0000" channel="a">
            <title>A2</title>
          </programme>
        </tv>"#;

        let progs: Vec<_> = parse(xml.as_slice()).collect::<Result<_, _>>().unwrap();
        assert_eq!(progs.len(), 3);
        assert_eq!(progs[0].title, "A1");
        assert_eq!(progs[1].title, "B1");
        assert_eq!(progs[2].title, "A2");
    }

    #[test]
    fn negative_duration_skipped() {
        // stop < start → skip.
        let xml = br#"<tv>
          <programme start="20240101130000 +0000" stop="20240101120000 +0000" channel="c1">
            <title>Backwards</title>
          </programme>
        </tv>"#;
        let progs: Vec<_> = parse(xml.as_slice()).collect::<Result<_, _>>().unwrap();
        assert!(progs.is_empty());
    }

    #[test]
    fn icon_self_closing() {
        let xml = br#"<tv>
          <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="c1">
            <title>T</title>
            <icon src="http://x/img.png"/>
          </programme>
        </tv>"#;
        let progs: Vec<_> = parse(xml.as_slice()).collect::<Result<_, _>>().unwrap();
        assert_eq!(progs[0].icon_url.as_deref(), Some("http://x/img.png"));
    }

    #[test]
    fn collects_channel_icons_alongside_programmes() {
        let xml = br#"<tv>
          <channel id="bbc1.uk">
            <display-name lang="en">BBC One</display-name>
            <icon src="https://logos.example/bbc1.png" width="300"/>
            <icon src="https://logos.example/bbc1-alt.png"/>
          </channel>
          <channel id="cdn.tv"><icon src="https://logos.example/channel/42"></icon></channel>
          <channel id="noicon"><display-name>No icon</display-name></channel>
          <channel id="bad"><icon src="https://image.tmdb.org/t/p/w500"/></channel>
          <channel><icon src="https://logos.example/orphan.png"/></channel>
          <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="bbc1.uk">
            <title>News</title>
          </programme>
        </tv>"#;
        let mut programs = parse(xml.as_slice());
        let progs: Vec<_> = programs.by_ref().collect::<Result<_, _>>().unwrap();
        assert_eq!(progs.len(), 1, "channels never become programmes");
        assert_eq!(progs[0].title, "News");
        let icons = programs.take_channel_icons();
        assert_eq!(
            icons,
            vec![
                ChannelIcon {
                    channel_id: "bbc1.uk".into(),
                    icon_url: "https://logos.example/bbc1.png".into(),
                },
                ChannelIcon {
                    channel_id: "cdn.tv".into(),
                    icon_url: "https://logos.example/channel/42".into(),
                },
            ]
        );
        assert!(programs.take_channel_icons().is_empty());
    }
}
