//! The ASR worker's wire protocol over its stdin/stdout pipes.
//!
//! Every message is one JSON line. A decode request is followed by its audio
//! as raw little-endian f32 samples, so a 15 s buffer costs a 960 KB memcpy,
//! not a WAV encode or an HTTP round trip. The worker answers each request
//! with exactly one line, in order.
//!
//! ```text
//! worker → app   {"ready":{"accel":"metal","load_ms":1490}}
//! app → worker   {"decode":{"samples":240000,"offset_sec":12.0,"prompt":"…","language":"fr"}}
//!                <240000 × f32 LE>
//! worker → app   {"decoded":{"words":[…],"language":"fr","accel":"coreml","decode_ms":310}}
//!            or  {"failed":"…"}
//! ```

use std::io::{self, BufRead, Read, Write};

use serde::{Deserialize, Serialize};

/// Longest buffer a request may carry: the encoder's 30 s window.
pub const MAX_REQUEST_SAMPLES: usize = 30 * 16_000;

/// Which encoder served a decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Accel {
    /// The GPU path: ready in about 1.5 s, used while Core ML compiles.
    Metal,
    /// The Neural Engine encoder: faster steps, frees the GPU for video.
    Coreml,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodeRequest {
    pub samples: usize,
    /// Stream time of the buffer's first sample, added to word timestamps.
    pub offset_sec: f64,
    pub prompt: String,
    /// ISO 639-1 code, or None to detect.
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decoded {
    pub words: Vec<WireWord>,
    /// The language whisper decoded in (detected or forced).
    pub language: Option<String>,
    pub accel: Accel,
    pub decode_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ready {
    pub accel: Accel,
    pub load_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    Decode(DecodeRequest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Ready(Ready),
    Decoded(Decoded),
    Failed(String),
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

pub fn write_line<T: Serialize>(out: &mut impl Write, message: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *out, message).map_err(io::Error::other)?;
    out.write_all(b"\n")?;
    out.flush()
}

/// Read one JSON line; `Ok(None)` at a clean end of stream.
pub fn read_line<T: for<'de> Deserialize<'de>>(input: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|_| invalid("malformed ASR worker message"))
}

pub fn write_decode(out: &mut impl Write, request: &DecodeRequest, pcm: &[f32]) -> io::Result<()> {
    if pcm.len() != request.samples || pcm.len() > MAX_REQUEST_SAMPLES {
        return Err(invalid("decode request does not match its audio"));
    }
    serde_json::to_writer(&mut *out, &Request::Decode(request.clone()))
        .map_err(io::Error::other)?;
    out.write_all(b"\n")?;
    let mut bytes = Vec::with_capacity(pcm.len() * 4);
    for sample in pcm {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    out.write_all(&bytes)?;
    out.flush()
}

/// The audio following a decode header.
pub fn read_samples(input: &mut impl Read, count: usize) -> io::Result<Vec<f32>> {
    if count > MAX_REQUEST_SAMPLES {
        return Err(invalid("decode request exceeds the encoder window"));
    }
    let mut bytes = vec![0u8; count * 4];
    input.read_exact(&mut bytes)?;
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decode_request_round_trips_with_its_audio() {
        let request = DecodeRequest {
            samples: 3,
            offset_sec: 1.5,
            prompt: "مرحبا".into(),
            language: Some("ar".into()),
        };
        let mut wire = Vec::new();
        write_decode(&mut wire, &request, &[0.25, -1.0, 0.5]).unwrap();
        let mut input = io::Cursor::new(wire);
        let Some(Request::Decode(read)) = read_line::<Request>(&mut input).unwrap() else {
            panic!("expected a decode request");
        };
        assert_eq!(read, request);
        assert_eq!(
            read_samples(&mut input, read.samples).unwrap(),
            [0.25, -1.0, 0.5]
        );
        assert!(read_line::<Request>(&mut input).unwrap().is_none());
    }

    #[test]
    fn mismatched_or_oversized_audio_is_refused() {
        let request = DecodeRequest {
            samples: 2,
            offset_sec: 0.0,
            prompt: String::new(),
            language: None,
        };
        assert!(write_decode(&mut Vec::new(), &request, &[0.0]).is_err());
        assert!(read_samples(&mut io::Cursor::new(Vec::new()), MAX_REQUEST_SAMPLES + 1).is_err());
    }

    #[test]
    fn responses_use_a_stable_shape() {
        let mut wire = Vec::new();
        write_line(
            &mut wire,
            &Response::Ready(Ready {
                accel: Accel::Coreml,
                load_ms: 12,
            }),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(wire).unwrap(),
            "{\"ready\":{\"accel\":\"coreml\",\"load_ms\":12}}\n"
        );
    }
}
