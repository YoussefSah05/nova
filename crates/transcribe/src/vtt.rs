//! WebVTT generation with the post-processing rules from
//! `docs/reference/whisper.md`:
//! - merge cues shorter than 1.5s into the neighbouring cue,
//! - cap cue text at 70 characters per line, 2 lines max,
//! - trim whitespace.
//!
//! Pure and deterministic so it can be unit-tested without a model.

use nova_core::TranscriptSegment;

const MIN_CUE_SEC: f64 = 1.5;
/// Don't merge across a silence longer than this — likely a speaker/scene change.
const MAX_MERGE_GAP_SEC: f64 = 0.5;
const MAX_LINE_CHARS: usize = 70;

/// Render transcript segments as a WebVTT document.
pub fn segments_to_vtt(segments: &[TranscriptSegment]) -> String {
    let cues = merge_short_cues(segments);

    let mut out = String::from("WEBVTT\n");
    for (i, cue) in cues.iter().enumerate() {
        out.push('\n');
        out.push_str(&(i + 1).to_string());
        out.push('\n');
        out.push_str(&format_timestamp(cue.start_sec));
        out.push_str(" --> ");
        out.push_str(&format_timestamp(cue.end_sec));
        out.push('\n');
        out.push_str(&wrap_two_lines(&cue.text));
        out.push('\n');
    }
    out
}

struct Cue {
    start_sec: f64,
    end_sec: f64,
    text: String,
}

/// Grow cues shorter than [`MIN_CUE_SEC`] by absorbing the following segment,
/// unless separated by a gap larger than [`MAX_MERGE_GAP_SEC`].
fn merge_short_cues(segments: &[TranscriptSegment]) -> Vec<Cue> {
    let mut cues: Vec<Cue> = Vec::new();
    for seg in segments {
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(last) = cues.last_mut() {
            let last_dur = last.end_sec - last.start_sec;
            let gap = seg.start_sec - last.end_sec;
            if last_dur < MIN_CUE_SEC && gap <= MAX_MERGE_GAP_SEC {
                last.end_sec = seg.end_sec;
                last.text.push(' ');
                last.text.push_str(text);
                continue;
            }
        }
        cues.push(Cue {
            start_sec: seg.start_sec,
            end_sec: seg.end_sec,
            text: text.to_string(),
        });
    }
    cues
}

/// `HH:MM:SS.mmm` per the WebVTT spec.
fn format_timestamp(sec: f64) -> String {
    let total_ms = (sec.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Wrap `text` to at most two lines of ≤70 characters. If it fits on one line,
/// return it as-is; otherwise break at the word boundary nearest the midpoint
/// so the two lines are visually balanced.
fn wrap_two_lines(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_LINE_CHARS {
        return text.to_string();
    }

    let words: Vec<&str> = text.split_whitespace().collect();
    let total: usize = text.chars().count();
    let target = total / 2;

    let mut best_split = 1usize;
    let mut best_dist = usize::MAX;
    let mut running = 0usize;
    for (i, w) in words.iter().enumerate() {
        running += w.chars().count();
        if i > 0 {
            running += 1; // the space
        }
        // Candidate: first line = words[..=i].
        if i + 1 < words.len() {
            let dist = running.abs_diff(target);
            if dist < best_dist {
                best_dist = dist;
                best_split = i + 1;
            }
        }
    }

    let first = words[..best_split].join(" ");
    let second = words[best_split..].join(" ");
    format!("{first}\n{second}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::TranscriptSegment;

    fn seg(seq: i64, start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            seq,
            start_sec: start,
            end_sec: end,
            text: text.to_string(),
            words: None,
        }
    }

    #[test]
    fn header_and_basic_cue() {
        let vtt = segments_to_vtt(&[seg(0, 0.0, 2.0, "Hello world.")]);
        assert!(vtt.starts_with("WEBVTT\n"));
        assert!(vtt.contains("00:00:00.000 --> 00:00:02.000"));
        assert!(vtt.contains("Hello world."));
    }

    #[test]
    fn short_cue_merges_into_neighbour() {
        // A 0.4s cue followed closely by another should merge into one cue.
        let vtt = segments_to_vtt(&[
            seg(0, 0.0, 0.4, "Wait"),
            seg(1, 0.5, 2.5, "what did you say?"),
        ]);
        // One cue index ("1"), spanning 0.0 -> 2.5, text joined.
        assert!(vtt.contains("00:00:00.000 --> 00:00:02.500"));
        assert!(vtt.contains("Wait what did you say?"));
        // There must not be a second cue.
        assert!(!vtt.contains("\n2\n"));
    }

    #[test]
    fn long_line_wraps_to_two_balanced_lines() {
        let long = "This is a single very long caption line that clearly exceeds seventy characters and must wrap";
        let wrapped = wrap_two_lines(long);
        let lines: Vec<&str> = wrapped.split('\n').collect();
        assert_eq!(lines.len(), 2, "must be exactly two lines");
        assert!(lines[0].chars().count() <= MAX_LINE_CHARS);
        assert!(lines[1].chars().count() <= MAX_LINE_CHARS);
    }

    #[test]
    fn timestamp_formats_hours() {
        assert_eq!(format_timestamp(3661.5), "01:01:01.500");
    }
}
