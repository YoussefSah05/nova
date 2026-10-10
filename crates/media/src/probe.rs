//! Short ffprobe call -> [`StreamProbe`].
//!
//! Used by the HLS proxy at the start of a playback session to pick between
//! passthrough and on-the-fly remux. Always invoked with a short timeout
//! because some IPTV providers stall on probe requests - a slow probe must
//! not block the player.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;

use crate::ffprobe_bin;

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("ffprobe timed out after {0:?}")]
    Timeout(Duration),
    #[error("ffprobe failed: {0}")]
    Failed(String),
    #[error("ffprobe output was not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Codec / container snapshot used by the HLS proxy to pick a routing mode.
#[derive(Debug, Clone)]
pub struct StreamProbe {
    /// Probed container long name (e.g. "Matroska / WebM", "QuickTime / MOV",
    /// "Apple HTTP Live Streaming"). `None` if ffprobe couldn't tell.
    pub container: Option<String>,
    /// Probed format name (e.g. "hls", "mpegts", "mov,mp4,m4a,3gp,3g2,mj2").
    pub format_name: Option<String>,
    /// Primary video codec name (e.g. "h264", "hevc"). `None` for audio-only.
    pub video_codec: Option<String>,
    /// Primary audio codec name (e.g. "aac", "ac3", "eac3"). `None` for video-only.
    pub audio_codec: Option<String>,
    /// Duration in seconds when ffprobe can determine it.
    pub duration_sec: Option<f64>,
    /// Primary video width, if present.
    pub width: Option<i64>,
    /// Primary video height, if present.
    pub height: Option<i64>,
    /// `true` only when the upstream is an HLS playlist. MPEG-TS transport
    /// streams are deliberately not considered playlists.
    pub is_playlist_hls: bool,
    /// `true` for direct MPEG-TS streams. These need remuxing even when the
    /// codecs are browser-compatible.
    pub is_transport_stream: bool,
    /// Final route recommendation for the playback backend.
    pub route: PlaybackRoute,
}

/// Backwards-compatible name for call sites that still talk about a profile.
pub type StreamProfile = StreamProbe;

/// What the HLS proxy should do with this stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackRoute {
    /// HLS upstream, browser-compatible codecs - rewrite manifest, stream
    /// segments verbatim.
    HlsPassthrough,
    /// Direct browser-compatible MP4-family file - proxy with Range support.
    FilePassthrough,
    /// Container or audio codec unsupported in WebKit (MKV, AC-3/EAC-3, etc.)
    /// but video can be stream-copied. Remux to fMP4-HLS, re-encode audio
    /// only.
    RemuxHls,
    /// Video codec also needs transcoding (VC-1, AV1 on older Macs, etc.).
    TranscodeHls,
}

/// Probe `url` with a tight timeout, returning a [`StreamProbe`].
///
/// The HLS proxy calls this once per playback session and caches the result.
pub async fn probe_stream(url: &str, deadline: Duration) -> Result<StreamProbe, ProbeError> {
    let mut cmd = Command::new(ffprobe_bin());
    cmd.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        // Network timeout is in microseconds; cap below the overall deadline
        // so we get a clean ffprobe error rather than the kill below.
        "-timeout",
        // Half the deadline, in us. Saturating cast for safety.
        &(deadline.as_micros() / 2)
            .min(u128::from(u32::MAX))
            .to_string(),
        url,
    ])
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    run_probe(cmd, url, deadline).await
}

/// Audio stream tags only. Subtitles and a title's original language do not
/// establish which audio tracks are present in this particular version.
#[derive(Debug, PartialEq, Eq)]
pub struct AudioLanguages {
    pub languages: Vec<String>,
    pub unknown_tracks: u32,
    pub track_count: u32,
    pub tracks: Vec<AudioTrackInfo>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AudioTrackInfo {
    /// Position among audio streams, matching FFmpeg's `0:a:<index>`.
    pub audio_index: u32,
    pub language: Option<String>,
}

/// Bounded metadata request; no playback session or media download is started.
pub async fn probe_audio_languages(
    url: &str,
    deadline: Duration,
) -> Result<AudioLanguages, ProbeError> {
    let mut cmd = Command::new(ffprobe_bin());
    cmd.kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=codec_type:stream_tags=language",
            "-rw_timeout",
            &(deadline.as_micros() / 2)
                .min(u128::from(u32::MAX))
                .to_string(),
        ])
        .arg(url);
    let output = timeout(deadline, cmd.output())
        .await
        .map_err(|_| ProbeError::Timeout(deadline))??;
    if !output.status.success() {
        return Err(ProbeError::Failed("audio metadata unavailable".into()));
    }
    // Never echo ffprobe's stderr or raw JSON: either may contain provider data.
    let parsed: AudioProbeOutput = serde_json::from_slice(&output.stdout)?;
    Ok(parsed.into_languages())
}

#[derive(Deserialize)]
struct AudioProbeOutput {
    streams: Vec<AudioProbeStream>,
}

#[derive(Deserialize)]
struct AudioProbeStream {
    codec_type: Option<String>,
    tags: Option<AudioProbeTags>,
}

#[derive(Deserialize)]
struct AudioProbeTags {
    language: Option<String>,
}

impl AudioProbeOutput {
    fn into_languages(self) -> AudioLanguages {
        let mut result = AudioLanguages {
            languages: Vec::new(),
            unknown_tracks: 0,
            track_count: 0,
            tracks: Vec::new(),
        };
        for stream in self.streams {
            if stream.codec_type.as_deref() != Some("audio") {
                continue;
            }
            let code = stream
                .tags
                .and_then(|tags| tags.language)
                .and_then(|tag| compact_audio_language(&tag));
            result.tracks.push(AudioTrackInfo {
                audio_index: result.track_count,
                language: code.clone(),
            });
            result.track_count += 1;
            match code {
                Some(code) if !result.languages.contains(&code) => result.languages.push(code),
                Some(_) => {}
                None => result.unknown_tracks += 1,
            }
        }
        result
    }
}

/// Normalize stream tags, keeping valid three-letter tags when no short alias
/// is known. No arbitrary track names or provider text cross the IPC boundary.
fn compact_audio_language(tag: &str) -> Option<String> {
    let tag = tag.trim().to_ascii_lowercase();
    let primary = tag.split(['-', '_']).next()?;
    if !matches!(primary.len(), 2 | 3)
        || !primary.bytes().all(|b| b.is_ascii_alphabetic())
        || matches!(primary, "und" | "mul" | "zxx" | "mis" | "unk" | "xx" | "zz")
    {
        return None;
    }
    let code = match primary {
        "eng" => "en",
        "fra" | "fre" => "fr",
        "ara" | "arb" => "ar",
        "deu" | "ger" => "de",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "rus" => "ru",
        "jpn" => "ja",
        "kor" => "ko",
        "zho" | "chi" => "zh",
        "hin" => "hi",
        "nld" | "dut" => "nl",
        "swe" => "sv",
        "nor" => "no",
        "dan" => "da",
        "fin" => "fi",
        "pol" => "pl",
        "tur" => "tr",
        "heb" => "he",
        "ell" | "gre" => "el",
        "ces" | "cze" => "cs",
        "ron" | "rum" => "ro",
        "ukr" => "uk",
        "vie" => "vi",
        "tha" => "th",
        "ind" => "id",
        "msa" | "may" => "ms",
        "hun" => "hu",
        "bul" => "bg",
        "hrv" => "hr",
        "srp" => "sr",
        "slk" | "slo" => "sk",
        "slv" => "sl",
        "lit" => "lt",
        "lav" => "lv",
        "est" => "et",
        "fas" | "per" => "fa",
        "urd" => "ur",
        "ben" => "bn",
        "tam" => "ta",
        "tel" => "te",
        "cat" => "ca",
        "isl" | "ice" => "is",
        "nob" => "nb",
        "nno" => "nn",
        "eus" | "baq" => "eu",
        "cym" | "wel" => "cy",
        "gle" => "ga",
        "afr" => "af",
        "swa" => "sw",
        other => other,
    };
    Some(code.to_ascii_uppercase())
}

async fn run_probe(
    mut cmd: Command,
    url: &str,
    deadline: Duration,
) -> Result<StreamProbe, ProbeError> {
    cmd.kill_on_drop(true);
    let child_future = async {
        let output = cmd.output().await?;
        Ok::<_, std::io::Error>(output)
    };

    let output = match timeout(deadline, child_future).await {
        Ok(r) => r?,
        Err(_) => return Err(ProbeError::Timeout(deadline)),
    };

    if !output.status.success() {
        return Err(ProbeError::Failed(format!(
            "process exited with status {:?}",
            output.status.code()
        )));
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)?;
    Ok(parsed.into_profile(url))
}

// -- ffprobe JSON ------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    format_name: Option<String>,
    format_long_name: Option<String>,
    duration: Option<String>,
}

impl FfprobeOutput {
    fn into_profile(self, url: &str) -> StreamProbe {
        let video_stream = self
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("video"));
        let video_codec = video_stream.and_then(|s| s.codec_name.clone());
        let audio_codec = self.streams.iter().find_map(|s| {
            (s.codec_type.as_deref() == Some("audio"))
                .then(|| s.codec_name.clone())
                .flatten()
        });
        let format_name = self.format.as_ref().and_then(|f| f.format_name.clone());
        let container = self
            .format
            .as_ref()
            .and_then(|f| f.format_long_name.clone());
        let duration_sec = self
            .format
            .as_ref()
            .and_then(|f| parse_duration(f.duration.as_deref()))
            .or_else(|| {
                self.streams
                    .iter()
                    .find_map(|s| parse_duration(s.duration.as_deref()))
            });

        let is_playlist_hls = format_name
            .as_deref()
            .map(|n| n.split(',').any(|part| part == "hls"))
            .unwrap_or(false)
            || url_looks_like_hls_playlist(url);
        let is_transport_stream = format_name
            .as_deref()
            .map(|n| n.split(',').any(|part| part == "mpegts"))
            .unwrap_or(false)
            || url_looks_like_transport_stream(url);

        let route = recommend_route(
            is_playlist_hls,
            is_transport_stream,
            format_name.as_deref(),
            video_codec.as_deref(),
            audio_codec.as_deref(),
        );

        StreamProbe {
            container,
            format_name,
            video_codec,
            audio_codec,
            duration_sec,
            width: video_stream.and_then(|s| s.width),
            height: video_stream.and_then(|s| s.height),
            is_playlist_hls,
            is_transport_stream,
            route,
        }
    }
}

fn recommend_route(
    is_playlist_hls: bool,
    is_transport_stream: bool,
    format_name: Option<&str>,
    video: Option<&str>,
    audio: Option<&str>,
) -> PlaybackRoute {
    // Codecs WebKit can hardware-decode on macOS: H.264 always, HEVC on
    // 10.13+. Everything else must be transcoded.
    let video_ok = matches!(video, Some("h264" | "hevc" | "h265") | None);
    // Audio codecs WebKit can decode: AAC, MP3. AC-3/EAC-3 require system
    // codec support that the embedded WebKit doesn't ship with on macOS.
    let audio_ok = matches!(audio, Some("aac" | "mp3") | None);

    if is_playlist_hls && video_ok && audio_ok {
        PlaybackRoute::HlsPassthrough
    } else if !is_playlist_hls
        && !is_transport_stream
        && is_browser_file_container(format_name)
        && video_ok
        && audio_ok
    {
        PlaybackRoute::FilePassthrough
    } else if video_ok {
        PlaybackRoute::RemuxHls
    } else {
        PlaybackRoute::TranscodeHls
    }
}

fn is_browser_file_container(format_name: Option<&str>) -> bool {
    format_name
        .map(|n| {
            n.split(',').any(|part| {
                matches!(
                    part,
                    "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2" | "mp3" | "aac"
                )
            })
        })
        .unwrap_or(false)
}

fn parse_duration(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

pub fn url_looks_like_hls_playlist(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains(".m3u8")
}

pub fn url_looks_like_transport_stream(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.ends_with(".ts") || lower.contains(".ts?")
}

pub fn url_looks_like_browser_file(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    [".mp4", ".m4v", ".mov", ".mp3", ".aac"]
        .iter()
        .any(|ext| lower.ends_with(ext) || lower.contains(&format!("{ext}?")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_tags_exclude_subtitles_and_preserve_unknown_tracks() {
        let raw = r#"{"streams":[
          {"codec_type":"audio","tags":{"language":"fra"}},
          {"codec_type":"audio","tags":{"language":"fr-FR"}},
          {"codec_type":"audio","tags":{"language":"ara"}},
          {"codec_type":"audio","tags":{"language":"EN_us"}},
          {"codec_type":"audio","tags":{"language":"und"}},
          {"codec_type":"audio"},
          {"codec_type":"subtitle","tags":{"language":"deu"}},
          {"codec_type":"video"}
        ]}"#;
        let parsed: AudioProbeOutput = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed.into_languages(),
            AudioLanguages {
                languages: vec!["FR".into(), "AR".into(), "EN".into()],
                unknown_tracks: 2,
                track_count: 6,
                tracks: [Some("FR"), Some("FR"), Some("AR"), Some("EN"), None, None]
                    .into_iter()
                    .enumerate()
                    .map(|(index, language)| AudioTrackInfo {
                        audio_index: index as u32,
                        language: language.map(str::to_string),
                    })
                    .collect(),
            }
        );
    }

    #[test]
    fn audio_tags_do_not_export_arbitrary_provider_text() {
        for tag in [
            "",
            "und",
            "mul",
            "zxx",
            "MULTI",
            "VOSTFR",
            "https://private.invalid",
            "Arabic audio",
        ] {
            assert_eq!(compact_audio_language(tag), None, "{tag}");
        }
        assert_eq!(compact_audio_language(" fre "), Some("FR".into()));
        assert_eq!(compact_audio_language("ar-MA"), Some("AR".into()));
        assert_eq!(compact_audio_language("yue"), Some("YUE".into()));
        let parsed: AudioProbeOutput = serde_json::from_str(r#"{"streams":[]}"#).unwrap();
        assert_eq!(parsed.into_languages().track_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires NOVA_TEST_AUDIO_FILE with FR/AR/EN and one untagged audio stream"]
    async fn real_audio_probe_reads_only_container_audio_languages() {
        let file = std::env::var("NOVA_TEST_AUDIO_FILE").expect("tagged local media fixture");
        let info = probe_audio_languages(&file, Duration::from_secs(6))
            .await
            .unwrap();
        assert_eq!(info.languages, vec!["FR", "AR", "EN"]);
        assert_eq!(info.track_count, 4);
        assert_eq!(info.unknown_tracks, 1);
        assert_eq!(info.tracks.len(), 4);
        assert_eq!(info.tracks[3].audio_index, 3);
        assert_eq!(info.tracks[3].language, None);
    }

    #[test]
    fn classification_recognises_browser_friendly_hls() {
        assert_eq!(
            recommend_route(true, false, Some("hls"), Some("h264"), Some("aac")),
            PlaybackRoute::HlsPassthrough
        );
    }

    #[test]
    fn classification_recognises_browser_friendly_mp4() {
        assert_eq!(
            recommend_route(
                false,
                false,
                Some("mov,mp4,m4a,3gp,3g2,mj2"),
                Some("h264"),
                Some("aac"),
            ),
            PlaybackRoute::FilePassthrough
        );
    }

    #[test]
    fn classification_remuxes_direct_mpegts() {
        assert_eq!(
            recommend_route(false, true, Some("mpegts"), Some("h264"), Some("aac")),
            PlaybackRoute::RemuxHls
        );
    }

    #[test]
    fn classification_remuxes_browser_codecs_in_non_mp4_containers() {
        assert_eq!(
            recommend_route(
                false,
                false,
                Some("matroska,webm"),
                Some("h264"),
                Some("aac")
            ),
            PlaybackRoute::RemuxHls
        );
    }

    #[test]
    fn classification_remuxes_ac3_audio() {
        assert_eq!(
            recommend_route(false, false, Some("mov,mp4"), Some("h264"), Some("ac3")),
            PlaybackRoute::RemuxHls
        );
        assert_eq!(
            recommend_route(false, false, Some("mov,mp4"), Some("hevc"), Some("eac3")),
            PlaybackRoute::RemuxHls
        );
    }

    #[test]
    fn classification_transcodes_exotic_video() {
        assert_eq!(
            recommend_route(false, false, Some("mov,mp4"), Some("vc1"), Some("aac")),
            PlaybackRoute::TranscodeHls
        );
        assert_eq!(
            recommend_route(false, false, Some("mov,mp4"), Some("av1"), Some("aac")),
            PlaybackRoute::TranscodeHls
        );
    }
}

#[cfg(all(test, unix))]
mod cancellation_tests {
    use super::*;

    #[tokio::test]
    async fn timeout_terminates_the_probe_process() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo $$ > \"$1\"; exec sleep 30", "probe-fixture"])
            .arg(&pidfile)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        assert!(matches!(
            run_probe(cmd, "fixture", Duration::from_millis(300)).await,
            Err(ProbeError::Timeout(_))
        ));
        let pid = tokio::fs::read_to_string(pidfile).await.unwrap();
        let mut alive = true;
        for _ in 0..50 {
            alive = Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .unwrap()
                .success();
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Clean up the regression's leaked process even when this assertion fails.
        if alive {
            let _ = Command::new("kill").arg(pid.trim()).status().await;
        }
        assert!(!alive, "ffprobe child survived its timeout");
    }
}
