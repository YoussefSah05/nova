# XMLTV programme guides

How Nova reads XMLTV guides (`crates/epg/`).

## Format

The canonical description is the [XMLTV format](http://wiki.xmltv.org/index.php/XMLTVFormat):

```xml
<tv>
  <channel id="bbc1.uk">
    <display-name>BBC One</display-name>
    <icon src="https://…/bbc1.png" />
  </channel>
  <programme start="20240101120000 +0000" stop="20240101130000 +0000" channel="bbc1.uk">
    <title lang="en">Programme title</title>
    <sub-title>Episode name</sub-title>
    <desc>Description.</desc>
    <category lang="en">Drama</category>
  </programme>
</tv>
```

## Times

- `start` and `stop` carry a UTC offset (`+0000`, `+0200`, `-0500`). Parse them with the
  pattern `%Y%m%d%H%M%S %z`.
- When a provider omits the offset, assume UTC.
- Store times as **UTC epoch seconds** (`programs.start_utc`, `programs.stop_utc`), never as
  strings.

## Channel matching

A playlist's `tvg-id` and the guide's channel `id` often differ in case or carry a domain suffix
(`BBC1.uk` versus `bbc1`). Match case-insensitively first; a prefix match is the fallback when an
exact match fails.

## Parsing

- Guides can be hundreds of megabytes: parse with `quick-xml` in streaming mode, never into a
  document tree.
- Emit programmes as they are parsed and insert them in batches (500 rows per batch); feeds are
  spooled to disk with a 512 MiB cap and published in one short transaction.

## Pitfalls

- Loading a whole guide into memory.
- Storing times as strings.
- Assuming a provider's playlist and guide use identical channel IDs.
