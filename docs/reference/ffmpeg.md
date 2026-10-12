# FFmpeg usage

`crates/media/` wraps FFmpeg and ffprobe as subprocesses. Binaries are resolved without relying
on the shell `PATH`, so GUI launches find Homebrew installs.

## Audio for speech recognition

Whisper needs 16 kHz mono signed 16-bit PCM. Nova drives `whisper-cli`, which reads a file, so
audio is extracted to a temporary WAV:

```bash
ffmpeg -hide_banner -loglevel error \
  [-reconnect 1 -reconnect_streamed 1 -reconnect_delay_max 5]   # http(s) inputs only
  -ss <offset> -t <duration> -i <input> \
  -vn -ac 1 -ar 16000 -c:a pcm_s16le -f wav -y <out.wav>
```

- Windows of 30 seconds (with overlap) are extracted around the playhead.
- The reconnect options are rejected for local files, so they are added only for network inputs.
- `-y` is safe because the caller owns the temporary path, and deletes it afterwards.
- FFmpeg's stderr can contain authenticated URLs, so it is never surfaced; errors are generic.

## Probing

```bash
ffprobe -v quiet -print_format json -show_streams -show_format <input>
```

Probes are short and time-bounded; they report codecs, duration, dimensions and the real audio
tracks shown in the Audio selector.

## Pitfalls

- Hard-coding `-threads`: let FFmpeg choose.
- Unbounded subprocesses: every call has a timeout and is killed on cancel.
